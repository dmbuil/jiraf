//! The Settings screen: validation, Jira checks, clean writes of `jiraf.toml` and `.env`, and its
//! commands.

use crate::config::{CONFIG_FILE, Config};
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
    let f = &cfg.fields;
    for (key, value) in [
        ("extended_status", &f.extended_status),
        ("activity_type", &f.activity_type),
        ("parent_link", &f.parent_link),
        ("epic_link", &f.epic_link),
        ("target_start", &f.target_start),
        ("target_end", &f.target_end),
    ] {
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
