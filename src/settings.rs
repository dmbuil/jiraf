//! The Settings screen: validation, Jira checks, clean writes of `jiraf.toml` and `.env`, and its
//! commands.

use crate::app::{Holder, Status};
use crate::config::{self, CONFIG_FILE, Config, ENV_FILE, Fields};
use crate::jira::{self, Jira, JiraError};
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

/// Something wrong with one field of the form. `at` is the field's path, e.g. `jira.url`, `token`,
/// `fields.epic_link`, `bubbles.2.jql`, `tabs` (0-based indexes, in the order the page sent them).
#[derive(Serialize, Debug, PartialEq)]
pub struct Problem {
    /// Path of the field in the form.
    pub at: String,
    /// What to fix, shown under the field.
    pub message: String,
}

impl Problem {
    /// A problem at `at`.
    fn new(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self { at: at.into(), message: message.into() }
    }
}

/// Everything that would stop `cfg` from loading or working, checked without Jira. `token_available`
/// says whether a token was typed or one is already saved.
pub fn validate(cfg: &Config, token_available: bool) -> Vec<Problem> {
    let mut out = Vec::new();
    let url = cfg.jira.url.trim();
    if url.is_empty() {
        out.push(Problem::new("jira.url", "Enter your Jira URL"));
    } else if !reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()) {
        out.push(Problem::new("jira.url", "Not a valid URL"));
    }
    if !token_available {
        out.push(Problem::new("token", "Paste your Jira token"));
    }
    for (key, value) in field_ids(&cfg.fields) {
        if value.trim().is_empty() {
            out.push(Problem::new(format!("fields.{key}"), "Required"));
        }
    }
    let fu = &cfg.follow_up;
    for (key, days) in [("stale_days", fu.stale_days), ("done_days", fu.done_days)] {
        if days < 1 {
            out.push(Problem::new(format!("follow_up.{key}"), "Must be 1 or more"));
        }
    }
    if !(1..=255).contains(&fu.ext_max_len) {
        out.push(Problem::new("follow_up.ext_max_len", "Between 1 and 255"));
    }
    if fu.hierarchy.is_empty() {
        out.push(Problem::new("follow_up.hierarchy", "Add at least one issue type"));
    } else if !fu.hierarchy.contains(&fu.initiative_type) {
        out.push(Problem::new("follow_up.initiative_type", "Must be one of the hierarchy levels"));
    }

    if cfg.bubbles.is_empty() {
        out.push(Problem::new("bubbles", "Add at least one bubble"));
    }
    let ids: std::collections::HashSet<&str> = cfg.bubbles.iter().map(|b| b.id.as_str()).collect();
    let mut names = std::collections::HashSet::new();
    for (i, b) in cfg.bubbles.iter().enumerate() {
        let name = b.name.trim().to_lowercase();
        if name.is_empty() {
            out.push(Problem::new(format!("bubbles.{i}.name"), "Required"));
        } else if !names.insert(name) {
            out.push(Problem::new(format!("bubbles.{i}.name"), "Another bubble already has this name"));
        }
        if b.jql.trim().is_empty() {
            out.push(Problem::new(format!("bubbles.{i}.jql"), "Required"));
        }
        for x in &b.exclude {
            if *x == b.id {
                out.push(Problem::new(format!("bubbles.{i}.exclude"), "A bubble can't exclude itself"));
            } else if !ids.contains(x.as_str()) {
                out.push(Problem::new(format!("bubbles.{i}.exclude"), "Excludes a bubble that no longer exists"));
            }
        }
    }

    if cfg.tabs.is_empty() {
        out.push(Problem::new("tabs", "Add at least one tab"));
    }
    let mut names = std::collections::HashSet::new();
    for (i, t) in cfg.tabs.iter().enumerate() {
        let name = t.name.trim().to_lowercase();
        if name.is_empty() {
            out.push(Problem::new(format!("tabs.{i}.name"), "Required"));
        } else if !names.insert(name) {
            out.push(Problem::new(format!("tabs.{i}.name"), "Another tab already has this name"));
        }
    }
    out
}

/// Each `[fields]` key with its value.
fn field_ids(f: &Fields) -> [(&'static str, &String); 6] {
    [
        ("extended_status", &f.extended_status),
        ("activity_type", &f.activity_type),
        ("parent_link", &f.parent_link),
        ("epic_link", &f.epic_link),
        ("target_start", &f.target_start),
        ("target_end", &f.target_end),
    ]
}

/// What Jira said about a config on Save ([`check_with_jira`]).
#[derive(Default, Debug)]
pub struct JiraCheck {
    /// Block the save: rejected token, unknown field ids.
    pub errors: Vec<Problem>,
    /// Saved anyway: bubbles whose JQL Jira rejects.
    pub warnings: Vec<Problem>,
    /// Jira couldn't be reached (or answered oddly) for at least one check, which was skipped.
    pub unverified: bool,
}

/// The HTTP status of `e` if it is a [`JiraError`].
fn jira_status(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<JiraError>().map(|j| j.status.as_u16())
}

/// Check `cfg` against its Jira with `token`: the token (`/myself`), then the field ids, then every
/// bubble's effective JQL. Jira being out of reach never blocks: those checks are skipped and
/// [`JiraCheck::unverified`] is set.
pub async fn check_with_jira(cfg: &Config, token: &str) -> JiraCheck {
    let mut out = JiraCheck::default();
    let j = match Jira::with(cfg.jira.url.trim().trim_end_matches('/'), token.trim()) {
        Ok(j) => j,
        Err(_) => {
            out.unverified = true;
            return out;
        }
    };
    if let Err(e) = j.get("/rest/api/2/myself").await {
        if matches!(jira_status(&e), Some(401 | 403)) {
            out.errors.push(Problem::new("token", "Jira rejected this token"));
        } else {
            log::warn!("settings: couldn't check the token: {e:#}");
            out.unverified = true;
        }
        return out;
    }
    match jira::field_names(&j).await {
        Ok(names) => {
            for (key, id) in field_ids(&cfg.fields) {
                if !names.contains_key(id.trim()) {
                    out.errors.push(Problem::new(format!("fields.{key}"), "No field with this id in Jira"));
                }
            }
        }
        Err(e) => {
            log::warn!("settings: couldn't read the field list: {e:#}");
            out.unverified = true;
            return out;
        }
    }
    let bubbles = crate::config::resolve_excludes(&cfg.bubbles).unwrap_or_else(|_| cfg.bubbles.clone());
    for (i, b) in bubbles.iter().enumerate() {
        if let Err(e) = j.check_jql(jira::strip_order_by(&b.jql)).await {
            match e.downcast_ref::<JiraError>() {
                Some(je) if je.status.as_u16() == 400 => out.warnings.push(Problem::new(format!("bubbles.{i}.jql"), je.message.clone())),
                _ => {
                    log::warn!("settings: couldn't check bubble `{}`: {e:#}", b.id);
                    out.unverified = true;
                }
            }
        }
    }
    out
}

/// Replace `path` in one step: write `<name>.tmp` next to it, then rename over it.
///
/// # Errors
/// If either step fails.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

/// Write `cfg` as a clean `jiraf.toml` in `dir`, keeping the previous file as `jiraf.toml.bak`.
///
/// # Errors
/// If the backup or the new file can't be written.
pub fn write_config(dir: &Path, cfg: &Config) -> Result<()> {
    let path = dir.join(CONFIG_FILE);
    if path.exists() {
        let bak = dir.join(format!("{CONFIG_FILE}.bak"));
        std::fs::copy(&path, &bak).with_context(|| format!("backing up to {}", bak.display()))?;
    }
    atomic_write(&path, &crate::config::render_toml(cfg)?)
}

/// Set `JIRA_TOKEN` in `env_file` to `token` (trimmed): its line is replaced in place (keeping the line
/// ending), or appended; every other line stays as it was. Creates the file if needed.
///
/// # Errors
/// If the file exists but can't be read, or can't be written.
pub fn write_token(env_file: &Path, token: &str) -> Result<()> {
    let old = match std::fs::read_to_string(env_file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", env_file.display())),
    };
    let line = format!("JIRA_TOKEN={}", token.trim());
    let mut out = String::with_capacity(old.len() + line.len() + 1);
    let mut done = false;
    for piece in old.split_inclusive('\n') {
        let text = piece.trim_end_matches(['\r', '\n']);
        let key = text.trim_start().strip_prefix("export ").unwrap_or(text.trim_start()).split('=').next().unwrap_or("").trim();
        if !done && key == "JIRA_TOKEN" {
            out.push_str(&line);
            out.push_str(&piece[text.len()..]);
            done = true;
        } else {
            out.push_str(piece);
        }
    }
    if !done {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&line);
        out.push('\n');
    }
    atomic_write(env_file, &out)
}

/* ---------- commands ---------- */

/// What the Settings screen opens with ([`settings`]). Never carries the token itself.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// The current `jiraf.toml` as written (excludes not folded in), or [`Config::template`] when
    /// there is none or it can't be read.
    pub config: Config,
    /// Whether a token is saved (or set in the environment).
    pub token_set: bool,
    /// Whether `JIRA_TOKEN` in the process environment wins over the one in `.env`.
    pub token_from_env: bool,
    /// No `jiraf.toml` yet.
    pub first_run: bool,
    /// Why `jiraf.toml` couldn't be read, if it couldn't (Save then replaces it, keeping a `.bak`).
    pub parse_error: Option<String>,
    /// Full path of the config folder.
    pub config_dir: String,
}

/// The outcome of [`save_settings`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReport {
    /// Whether the files were written.
    pub saved: bool,
    /// Why not, field by field (empty when saved).
    pub errors: Vec<Problem>,
    /// Saved anyway: bubbles whose JQL Jira rejects.
    pub warnings: Vec<Problem>,
    /// Jira couldn't be reached for some checks, which were skipped.
    pub unverified: bool,
    /// The app's status after the reload that follows a save.
    pub status: Option<Status>,
}

/// Everything the Settings screen shows.
#[tauri::command]
pub fn settings(holder: tauri::State<'_, Holder>) -> SettingsView {
    let dir = &holder.dir;
    let path = dir.join(CONFIG_FILE);
    let (config, first_run, parse_error) = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::template(), true, None),
        Err(e) => (Config::template(), false, Some(format!("reading {}: {e}", path.display()))),
        Ok(raw) => match Config::parse(&raw) {
            Ok(c) => (c, false, None),
            Err(e) => (Config::template(), false, Some(format!("{e:#}"))),
        },
    };
    SettingsView {
        config,
        token_set: config::read_token(&dir.join(ENV_FILE)).is_ok(),
        token_from_env: config::token_from_env(),
        first_run,
        parse_error,
        config_dir: dir.display().to_string(),
    }
}

/// Validate `config` (and `token`, when one was typed), check them with Jira, and if nothing blocks,
/// write `jiraf.toml` (keeping a `.bak`) and `.env`, then reload the app.
///
/// # Errors
/// Only if a file can't be written; everything else is in the [`SaveReport`].
#[tauri::command]
pub async fn save_settings(holder: tauri::State<'_, Holder>, config: Config, token: Option<String>) -> Result<SaveReport, String> {
    let mut cfg = config;
    config::assign_ids(&mut cfg);
    cfg.jira.url = cfg.jira.url.trim().trim_end_matches('/').to_string();
    let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let env = holder.dir.join(ENV_FILE);
    let current = config::read_token(&env).ok();
    let unsaved = |errors, warnings, unverified| SaveReport { saved: false, errors, warnings, unverified, status: None };
    let errors = validate(&cfg, token.is_some() || current.is_some());
    if !errors.is_empty() {
        return Ok(unsaved(errors, Vec::new(), false));
    }
    let check = check_with_jira(&cfg, token.as_deref().or(current.as_deref()).unwrap_or_default()).await;
    if !check.errors.is_empty() {
        return Ok(unsaved(check.errors, check.warnings, check.unverified));
    }
    write_config(&holder.dir, &cfg).map_err(|e| format!("{e:#}"))?;
    if let Some(t) = &token {
        write_token(&env, t).map_err(|e| format!("{e:#}"))?;
    }
    log::info!("settings saved");
    let status = holder.reload().await;
    Ok(SaveReport { saved: true, errors: Vec::new(), warnings: check.warnings, unverified: check.unverified, status: Some(status) })
}

/// Save a new token from the startup error screen: checked with Jira first when `jiraf.toml` can be
/// read (an unreachable Jira doesn't block), then written to `.env`, then the app reloads.
///
/// # Errors
/// If the token is empty or Jira rejects it, or `.env` can't be written.
#[tauri::command]
pub async fn save_token(holder: tauri::State<'_, Holder>, token: String) -> Result<Status, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste your Jira token".into());
    }
    if let Ok(raw) = std::fs::read_to_string(holder.dir.join(CONFIG_FILE))
        && let Ok(cfg) = Config::parse(&raw)
        && let Ok(j) = Jira::with(&cfg.jira.url, token)
        && let Err(e) = j.get("/rest/api/2/myself").await
        && matches!(jira_status(&e), Some(401 | 403))
    {
        return Err("Jira rejected this token".into());
    }
    write_token(&holder.dir.join(ENV_FILE), token).map_err(|e| format!("{e:#}"))?;
    log::info!("token saved");
    Ok(holder.reload().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Bubble, Fields};

    /// A config that passes [`validate`].
    fn valid() -> Config {
        let mut c = Config::template();
        c.jira.url = "https://jira.example.com".into();
        c.fields = Fields {
            extended_status: "customfield_1".into(),
            activity_type: "customfield_2".into(),
            parent_link: "customfield_3".into(),
            epic_link: "customfield_4".into(),
            target_start: "customfield_5".into(),
            target_end: "customfield_6".into(),
        };
        c.bubbles = vec![Bubble { id: "alpha".into(), name: "Alpha".into(), jql: "project = A".into(), icon_project: None, exclude: vec![] }];
        c
    }

    /// [`valid`] pointed at `url`, with bubbles `ok` (JQL `x`) and `broken` (JQL `bad`).
    fn with_url(url: &str) -> Config {
        let mut c = valid();
        c.jira.url = url.into();
        let b = |id: &str, jql: &str| Bubble { id: id.into(), name: id.into(), jql: jql.into(), icon_project: None, exclude: vec![] };
        c.bubbles = vec![b("ok", "x"), b("broken", "bad")];
        c
    }

    /// A one-shot-per-connection fake Jira on a random local port; returns its base URL. Each route is
    /// (path prefix, status, body); status 0 answers a search by its body: 400 if the JQL contains `bad`.
    async fn fake_jira(routes: Vec<(&'static str, u16, &'static str)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // read the headers, then as much body as Content-Length says
                    let (head, body) = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let head = text[..end].to_string();
                            let len = head
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break (head, String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).to_string());
                            }
                        }
                    };
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let (status, reply) = match routes.iter().find(|(p, _, _)| path.starts_with(p)) {
                        Some((_, 0, _)) if body.contains("bad") => (400, r#"{"errorMessages":["Error in the JQL Query"]}"#),
                        Some((_, 0, _)) => (200, r#"{"issues":[],"total":0}"#),
                        Some((_, st, b)) => (*st, *b),
                        None => (404, "{}"),
                    };
                    let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        url
    }

    #[tokio::test]
    async fn rejected_token_blocks() {
        let url = fake_jira(vec![("/rest/api/2/myself", 401, "{}")]).await;
        let r = check_with_jira(&with_url(&url), "t").await;
        assert_eq!(r.errors, vec![Problem { at: "token".into(), message: "Jira rejected this token".into() }]);
    }

    #[tokio::test]
    async fn bad_jql_warns_and_unknown_field_blocks() {
        let fields = r#"[{"id":"customfield_1","name":"Ext"}]"#; // only extended_status exists
        let url = fake_jira(vec![("/rest/api/2/myself", 200, r#"{"displayName":"x"}"#), ("/rest/api/2/field", 200, fields), ("/rest/api/2/search", 0, "")]).await;
        let mut c = with_url(&url);
        c.bubbles[1].exclude = vec![];
        let r = check_with_jira(&c, "t").await;
        assert!(r.errors.iter().any(|p| p.at == "fields.epic_link" && p.message == "No field with this id in Jira"), "{r:?}");
        assert!(!r.errors.iter().any(|p| p.at == "fields.extended_status"));
        assert_eq!(r.warnings, vec![Problem { at: "bubbles.1.jql".into(), message: "Error in the JQL Query".into() }]);
        assert!(!r.unverified);
    }

    #[tokio::test]
    async fn unreachable_jira_does_not_block() {
        let r = check_with_jira(&with_url("http://127.0.0.1:9"), "t").await;
        assert!(r.errors.is_empty() && r.warnings.is_empty() && r.unverified, "{r:?}");
    }

    #[test]
    fn env_only_token_line_changes() {
        let d = tempfile::tempdir().unwrap();
        let env = d.path().join(".env");
        std::fs::write(&env, "# mine\r\nRUST_LOG=debug\r\nexport JIRA_TOKEN=\"old\"\r\nOTHER=1").unwrap();
        write_token(&env, "  new  ").unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "# mine\r\nRUST_LOG=debug\r\nJIRA_TOKEN=new\r\nOTHER=1");
        std::fs::write(&env, "RUST_LOG=info").unwrap();
        write_token(&env, "t").unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "RUST_LOG=info\nJIRA_TOKEN=t\n");
        std::fs::remove_file(&env).unwrap();
        write_token(&env, "t").unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "JIRA_TOKEN=t\n");
    }

    #[test]
    fn config_write_keeps_a_backup_and_loads_back() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("jiraf.toml"), "# hand-written\n").unwrap();
        write_config(d.path(), &valid()).unwrap();
        assert_eq!(std::fs::read_to_string(d.path().join("jiraf.toml.bak")).unwrap(), "# hand-written\n");
        let back = Config::parse(&std::fs::read_to_string(d.path().join("jiraf.toml")).unwrap()).unwrap();
        assert_eq!(back.bubbles[0].jql, valid().bubbles[0].jql);
        assert!(!d.path().join("jiraf.toml.tmp").exists());
    }

    #[test]
    fn validation_blocks_what_load_would_reject() {
        assert_eq!(validate(&valid(), true), vec![]);
        let mut c = valid();
        c.jira.url = "jira.example.com".into();
        c.bubbles[0].exclude = vec!["gone".into(), c.bubbles[0].id.clone()];
        c.bubbles.push(c.bubbles[0].clone()); // same name
        c.bubbles[1].id = "b2".into();
        c.bubbles[1].exclude = vec![];
        c.fields.epic_link = "".into();
        let at: Vec<_> = validate(&c, false).into_iter().map(|p| (p.at, p.message)).collect();
        assert!(at.contains(&("jira.url".into(), "Not a valid URL".into())), "{at:?}");
        assert!(at.contains(&("token".into(), "Paste your Jira token".into())));
        assert!(at.contains(&("fields.epic_link".into(), "Required".into())));
        assert!(at.contains(&("bubbles.0.exclude".into(), "Excludes a bubble that no longer exists".into())));
        assert!(at.contains(&("bubbles.0.exclude".into(), "A bubble can't exclude itself".into())));
        assert!(at.contains(&("bubbles.1.name".into(), "Another bubble already has this name".into())));
        let mut e = valid();
        e.bubbles.clear();
        e.tabs.clear();
        let at: Vec<_> = validate(&e, true).into_iter().map(|p| p.at).collect();
        assert_eq!(at, ["bubbles", "tabs"]);
    }
}
