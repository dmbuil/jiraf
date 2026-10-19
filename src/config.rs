//! Configuration: `jiraf.toml` (everything but secrets) plus `JIRA_TOKEN` from `.env`, both in the
//! config folder (see [`Config::load`]).
//!
//! [`Config::load`] parses and validates the file at startup and on every reload. Every section except `[jira]`,
//! `[[bubbles]]` and `[[tabs]]` is optional and falls back to the defaults documented on each field.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Name of the config file inside the config folder.
pub const CONFIG_FILE: &str = "jiraf.toml";
/// Name of the secrets file inside the config folder.
pub const ENV_FILE: &str = ".env";

/// The whole `jiraf.toml`, plus the token taken from the environment.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// `[server]`: where jiraf listens and where it keeps icon copies.
    #[serde(default)]
    pub server: Server,
    /// `[jira]`: which Jira instance to talk to, and how long to cache its answers.
    pub jira: Jira,
    /// `[fields]`: custom field ids of the Jira instance.
    #[serde(default)]
    pub fields: Fields,
    /// `[follow_up]`: thresholds and hierarchy used by the dashboard.
    #[serde(default)]
    pub follow_up: FollowUp,
    /// `[[bubbles]]`: the filters shown in the top bar. Excludes are already folded into each `jql`
    /// once [`Config::load`] returns.
    pub bubbles: Vec<Bubble>,
    /// `[[tabs]]`: how the tree is split, in display order.
    pub tabs: Vec<Tab>,
    /// Filled from `JIRA_TOKEN`, never read from the TOML file.
    #[serde(skip)]
    pub token: String,
    /// [`Server::icon_dir`] resolved against the config folder ([`resolve_icon_dir`]).
    #[serde(skip)]
    pub icon_dir: PathBuf,
}

/// `[server]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    /// Former listen address. Ignored by the app (it opens no port), but still accepted so older files load.
    #[serde(default)]
    pub bind: Option<String>,
    /// Local copies of Jira icons/avatars. A disposable cache: delete it any time. A relative path is
    /// relative to the config folder.
    #[serde(default = "default_icon_dir")]
    pub icon_dir: String,
}

impl Default for Server {
    fn default() -> Self {
        Self { bind: None, icon_dir: default_icon_dir() }
    }
}

/// Default for [`Server::icon_dir`].
fn default_icon_dir() -> String {
    "icons".into()
}


/// `[jira]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct Jira {
    /// Base URL, e.g. `https://jira.example.com` (no trailing slash needed).
    pub url: String,
    /// In-memory cache lifetime for the dashboard data. Nothing is ever written to disk.
    #[serde(default = "default_cache")]
    pub cache_seconds: u64,
}

/// Default for [`Jira::cache_seconds`].
fn default_cache() -> u64 {
    60
}

/// Custom field ids differ per Jira instance: set yours in jiraf.toml (`GET /rest/api/2/field` lists them).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fields {
    /// Single-line text field holding the follow-up note ("Extended Status"). Editable from the page.
    pub extended_status: String,
    /// "Activity Type" single-select field that decides which [`Tab`] an issue and its descendants land in.
    pub activity_type: String,
    /// Advanced Roadmaps "Parent Link" (Epic → Initiative, Initiative → Initiative).
    pub parent_link: String,
    /// "Epic Link" (Task/Story → Epic).
    pub epic_link: String,
    /// Advanced Roadmaps "Target start" (shown alongside Target end).
    pub target_start: String,
    /// Advanced Roadmaps "Target end": editable, and drives the "late" check.
    pub target_end: String,
}

/// Example ids only: every Jira instance numbers its custom fields differently.
impl Default for Fields {
    fn default() -> Self {
        Self {
            extended_status: "customfield_11501".into(),
            activity_type: "customfield_12303".into(),
            parent_link: "customfield_10265".into(),
            epic_link: "customfield_10101".into(),
            target_start: "customfield_10266".into(),
            target_end: "customfield_10267".into(),
        }
    }
}

/// `[follow_up]` section. Serialized as-is into the dashboard payload, so the page uses the same values.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FollowUp {
    /// Extended Status older than this counts as "no update".
    pub stale_days: i64,
    /// Closed issues stay visible this many days.
    pub done_days: i64,
    /// Jira single-line text fields cap at 255 chars.
    pub ext_max_len: usize,
    /// Issue type names, top of the hierarchy first.
    pub hierarchy: Vec<String>,
    /// Name of the Initiative level in [`FollowUp::hierarchy`].
    pub initiative_type: String,
    /// Optional display order for priorities (Jira's own order otherwise).
    pub priority_order: Vec<String>,
}

impl Default for FollowUp {
    fn default() -> Self {
        Self {
            stale_days: 30,
            done_days: 14,
            ext_max_len: 255,
            hierarchy: ["Initiative", "Epic", "Task", "Sub-task"].map(String::from).to_vec(),
            initiative_type: "Initiative".into(),
            priority_order: Vec::new(),
        }
    }
}

/// One selectable "project" in the top bar, defined by an arbitrary JQL filter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Bubble {
    /// Unique, URL-safe id (appears in the page URL as `#p=<id>`).
    pub id: String,
    /// Label shown in the top bar.
    pub name: String,
    /// Any JQL; a trailing `ORDER BY` is ignored. After loading, it also carries the `AND NOT (…)` of
    /// every excluded bubble.
    pub jql: String,
    /// Optional Jira project key whose avatar is used as the bubble icon.
    #[serde(default)]
    pub icon_project: Option<String>,
    /// Ids of other bubbles to subtract: the effective JQL becomes `(jql) AND NOT (their jql)`, so
    /// complementary bubbles stay in sync from a single definition.
    #[serde(default, skip_serializing)]
    pub exclude: Vec<String>,
}

/// A tree tab. An item goes to the tab that pins it or one of its ancestors in `issues`; otherwise to the
/// first tab whose `activity_types` lists the top-most Activity Type in its line (its root ancestor's if
/// set, else the next one down, …, else its own), so a whole tree stays in one tab. If nothing in the
/// line has one, it goes to the tab with `include_unset = true`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    /// Unique id (appears in the page URL as `&t=<id>`). `att` and `other` are reserved by the page.
    pub id: String,
    /// Label shown on the tab.
    pub name: String,
    /// Activity Type values that belong to this tab (whole values; case and accents are ignored).
    #[serde(default)]
    pub activity_types: Vec<String>,
    /// Whether items without any Activity Type (on themselves or an ancestor) belong to this tab.
    #[serde(default)]
    pub include_unset: bool,
    /// Issue keys pinned to this tab whatever their Activity Type (descendants follow).
    #[serde(default)]
    pub issues: Vec<String>,
}

/// Bake `exclude` into each bubble's JQL: `(jql) AND NOT (other jql) AND NOT (…)`.
///
/// Only the excluded bubbles' own JQL is used (no chaining), which keeps it predictable and loop-free.
///
/// # Errors
/// If a bubble excludes an unknown id or itself.
fn resolve_excludes(bubbles: &[Bubble]) -> Result<Vec<Bubble>> {
    let by_id: std::collections::HashMap<&str, &Bubble> = bubbles.iter().map(|b| (b.id.as_str(), b)).collect();
    bubbles
        .iter()
        .map(|b| {
            let mut out = b.clone();
            if b.exclude.is_empty() {
                return Ok(out);
            }
            let mut jql = format!("({})", crate::jira::strip_order_by(&b.jql));
            for id in &b.exclude {
                let other = by_id.get(id.as_str()).with_context(|| format!("bubble `{}` excludes unknown bubble `{id}`", b.id))?;
                if other.id == b.id {
                    bail!("bubble `{}` excludes itself", b.id);
                }
                jql.push_str(&format!(" AND NOT ({})", crate::jira::strip_order_by(&other.jql)));
            }
            out.jql = jql;
            Ok(out)
        })
        .collect()
}

/// The value of `name` in the dotenv file `env_file`, without touching the process environment.
///
/// # Errors
/// If the file exists but can't be read or parsed.
pub fn env_file_var(env_file: &Path, name: &str) -> Result<Option<String>> {
    let iter = match dotenvy::from_path_iter(env_file) {
        Ok(it) => it,
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", env_file.display())),
    };
    for item in iter {
        let (k, v) = item.with_context(|| format!("parsing {}", env_file.display()))?;
        if k == name {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

/// The Jira token: `JIRA_TOKEN` from the process environment if set, else from `env_file`. Read
/// afresh on every call, so an edited `.env` is picked up on reload.
///
/// # Errors
/// If the token is missing or empty (the message names `env_file`), or the file can't be read.
pub fn read_token(env_file: &Path) -> Result<String> {
    let token = match std::env::var("JIRA_TOKEN").ok().filter(|t| !t.trim().is_empty()) {
        Some(t) => t,
        None => env_file_var(env_file, "JIRA_TOKEN")?.with_context(|| format!("JIRA_TOKEN is not set (put it in {})", env_file.display()))?,
    };
    if token.trim().is_empty() {
        bail!("JIRA_TOKEN is empty in {}", env_file.display());
    }
    Ok(token)
}

/// Resolve [`Server::icon_dir`]: absolute paths as-is, relative ones against the config folder `dir`.
pub fn resolve_icon_dir(dir: &Path, icon_dir: &str) -> PathBuf {
    dir.join(icon_dir)
}

/// Create the config folder `dir` if needed, with `jiraf.example.toml` and `.env.example` in it when
/// they're missing. Never writes `jiraf.toml` or `.env`.
///
/// # Errors
/// If the folder or an example file can't be written.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for (name, body) in [("jiraf.example.toml", include_str!("../jiraf.example.toml")), (".env.example", include_str!("../.env.example"))] {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    Ok(())
}

impl Config {
    /// Read, parse and validate `jiraf.toml` in the config folder `dir`, and take the token from
    /// [`read_token`] (`dir/.env`).
    ///
    /// Also normalises the Jira URL (no trailing slash), resolves bubble excludes and the icon folder.
    ///
    /// # Errors
    /// If the file is missing or malformed, `JIRA_TOKEN` is unset or empty, there are no bubbles or tabs,
    /// bubble ids repeat, or an exclude is invalid.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(CONFIG_FILE);
        if !path.exists() {
            bail!(
                "{} not found: copy jiraf.example.toml to jiraf.toml and .env.example to .env in {}, then fill them in",
                path.display(),
                dir.display()
            );
        }
        let raw = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        cfg.jira.url = cfg.jira.url.trim_end_matches('/').to_string();
        cfg.token = read_token(&dir.join(ENV_FILE))?;
        cfg.icon_dir = resolve_icon_dir(dir, &cfg.server.icon_dir);
        if cfg.server.bind.is_some() {
            log::warn!("[server] bind is ignored by the app");
        }
        if cfg.bubbles.is_empty() {
            bail!("jiraf.toml needs at least one [[bubbles]] entry");
        }
        if cfg.tabs.is_empty() {
            bail!("jiraf.toml needs at least one [[tabs]] entry");
        }
        let mut ids = std::collections::HashSet::new();
        for b in &cfg.bubbles {
            if !ids.insert(&b.id) {
                bail!("duplicate bubble id `{}`", b.id);
            }
        }
        cfg.bubbles = resolve_excludes(&cfg.bubbles)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal bubble for tests.
    fn bubble(id: &str, jql: &str, exclude: &[&str]) -> Bubble {
        Bubble { id: id.into(), name: id.into(), jql: jql.into(), icon_project: None, exclude: exclude.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn excludes_become_and_not() {
        let r = resolve_excludes(&[bubble("a", "project = DV ORDER BY rank", &["b"]), bubble("b", "summary ~ X", &[])]).unwrap();
        assert_eq!(r[0].jql, "(project = DV) AND NOT (summary ~ X)");
        assert_eq!(r[1].jql, "summary ~ X");
    }

    #[test]
    fn tabs_reject_legacy_label_keys() {
        assert!(toml::from_str::<Tab>("id = \"a\"\nname = \"A\"\nactivity_types = [\"Sostenibilidad\", \"Soporte a Comercial\"]\ninclude_unset = true").is_ok());
        assert!(toml::from_str::<Tab>("id = \"a\"\nname = \"A\"\nmatch = [\"roadmap\"]").is_err());
    }

    /// Writes `body` to `dir/name`.
    fn put(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn token_from_edited_env_file_wins_on_reload() {
        if std::env::var("JIRA_TOKEN").is_ok() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        put(d.path(), ENV_FILE, "JIRA_TOKEN=a\n");
        assert_eq!(read_token(&d.path().join(ENV_FILE)).unwrap(), "a");
        put(d.path(), ENV_FILE, "JIRA_TOKEN=b\n");
        assert_eq!(read_token(&d.path().join(ENV_FILE)).unwrap(), "b");
    }

    #[test]
    fn token_missing_or_empty_names_the_file() {
        if std::env::var("JIRA_TOKEN").is_ok() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let env = d.path().join(ENV_FILE);
        let e = format!("{:#}", read_token(&env).unwrap_err());
        assert!(e.contains("is not set") && e.contains(&env.display().to_string()), "{e}");
        put(d.path(), ENV_FILE, "JIRA_TOKEN=\n");
        let e = format!("{:#}", read_token(&env).unwrap_err());
        assert!(e.contains("is empty"), "{e}");
    }

    #[test]
    fn icon_dir_relative_and_absolute() {
        assert_eq!(resolve_icon_dir(Path::new("/c"), "icons"), Path::new("/c/icons"));
        assert_eq!(resolve_icon_dir(Path::new("/c"), "/tmp/i"), Path::new("/tmp/i"));
    }

    #[test]
    fn ensure_dir_keeps_existing_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("cfg");
        ensure_dir(&dir).unwrap();
        assert!(dir.join("jiraf.example.toml").is_file() && dir.join(".env.example").is_file());
        put(&dir, CONFIG_FILE, "mine");
        put(&dir, ENV_FILE, "JIRA_TOKEN=x");
        ensure_dir(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(dir.join(ENV_FILE)).unwrap(), "JIRA_TOKEN=x");
    }

    #[test]
    fn missing_config_says_what_to_copy() {
        let d = tempfile::tempdir().unwrap();
        let e = format!("{:#}", Config::load(d.path()).unwrap_err());
        assert!(e.contains("jiraf.example.toml") && e.contains(&d.path().display().to_string()), "{e}");
    }

    #[test]
    fn loads_example_with_bind() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), CONFIG_FILE, &format!("{}\n", include_str!("../jiraf.example.toml").replacen("[server]", "[server]\nbind = \"127.0.0.1:8080\"", 1)));
        put(d.path(), ENV_FILE, "JIRA_TOKEN=t\n");
        let cfg = Config::load(d.path()).unwrap();
        assert_eq!(cfg.icon_dir, d.path().join("icons"));
    }

    #[test]
    fn rejects_unknown_and_self_excludes() {
        assert!(resolve_excludes(&[bubble("a", "x", &["zz"])]).is_err());
        assert!(resolve_excludes(&[bubble("a", "x", &["a"])]).is_err());
    }
}
