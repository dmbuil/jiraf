//! Configuration: `jiraf.toml` (everything but secrets) plus `JIRA_TOKEN` from the environment / `.env`.
//!
//! [`Config::load`] parses and validates the file once at startup. Every section except `[jira]`,
//! `[[bubbles]]` and `[[tabs]]` is optional and falls back to the defaults documented on each field.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
}

/// `[server]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    /// Listen address (`host:port`). Defaults to `127.0.0.1:8080`; keep it on localhost.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Local copies of Jira icons/avatars. A disposable cache: delete it any time.
    #[serde(default = "default_icon_dir")]
    pub icon_dir: String,
}

impl Default for Server {
    fn default() -> Self {
        Self { bind: default_bind(), icon_dir: default_icon_dir() }
    }
}

/// Default for [`Server::icon_dir`].
fn default_icon_dir() -> String {
    "icons".into()
}

/// Default for [`Server::bind`].
fn default_bind() -> String {
    "127.0.0.1:8080".into()
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

impl Config {
    /// Read, parse and validate the config file at `path`, and take the token from `JIRA_TOKEN`.
    ///
    /// Also normalises the Jira URL (no trailing slash) and resolves bubble excludes.
    ///
    /// # Errors
    /// If the file is missing or malformed, `JIRA_TOKEN` is unset or empty, there are no bubbles or tabs,
    /// bubble ids repeat, or an exclude is invalid.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} (start from jiraf.example.toml: cp jiraf.example.toml jiraf.toml)", path.display()))?;
        let mut cfg: Config = toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        cfg.jira.url = cfg.jira.url.trim_end_matches('/').to_string();
        cfg.token = std::env::var("JIRA_TOKEN").context("JIRA_TOKEN is not set (put it in .env)")?;
        if cfg.token.trim().is_empty() {
            bail!("JIRA_TOKEN is empty");
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

    #[test]
    fn rejects_unknown_and_self_excludes() {
        assert!(resolve_excludes(&[bubble("a", "x", &["zz"])]).is_err());
        assert!(resolve_excludes(&[bubble("a", "x", &["a"])]).is_err());
    }
}
