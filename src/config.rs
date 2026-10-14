//! `jiraf.toml` (everything but secrets) + `JIRA_TOKEN` from the environment / `.env`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    pub jira: Jira,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub follow_up: FollowUp,
    pub bubbles: Vec<Bubble>,
    pub tabs: Vec<Tab>,
    /// Filled from `JIRA_TOKEN`, never read from the TOML file.
    #[serde(skip)]
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
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

fn default_icon_dir() -> String {
    "icons".into()
}

fn default_bind() -> String {
    "127.0.0.1:8080".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Jira {
    /// Base URL, e.g. `https://jira.example.com` (no trailing slash needed).
    pub url: String,
    /// In-memory cache lifetime for the dashboard data. Nothing is ever written to disk.
    #[serde(default = "default_cache")]
    pub cache_seconds: u64,
}

fn default_cache() -> u64 {
    60
}

/// Custom field ids differ per Jira instance: set yours in jiraf.toml (`GET /rest/api/2/field` lists them).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Fields {
    pub extended_status: String,
    pub initiative_label: String,
    pub parent_link: String,
    pub epic_link: String,
    pub target_start: String,
    pub target_end: String,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            extended_status: "customfield_11501".into(),
            initiative_label: "customfield_11502".into(),
            parent_link: "customfield_10265".into(),
            epic_link: "customfield_10101".into(),
            target_start: "customfield_10266".into(),
            target_end: "customfield_10267".into(),
        }
    }
}

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
    /// Name of the hierarchy level that carries the Initiative Label.
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
    pub id: String,
    pub name: String,
    pub jql: String,
    /// Optional Jira project key whose avatar is used as the bubble icon.
    #[serde(default)]
    pub icon_project: Option<String>,
    /// Ids of other bubbles to subtract: the effective JQL becomes `(jql) AND NOT (their jql)`, so
    /// complementary bubbles stay in sync from a single definition.
    #[serde(default, skip_serializing)]
    pub exclude: Vec<String>,
}

/// A tree tab. Items go to the tab listing their Initiative in `issues`; otherwise to the first tab whose
/// `match` substrings appear (case-insensitive) in the Initiative's label; an Initiative without label
/// goes to the tab with `include_unlabeled = true`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tab {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub include_unlabeled: bool,
    /// Initiative keys pinned to this tab whatever their label (children follow).
    #[serde(default)]
    pub issues: Vec<String>,
}

/// Bake `exclude` into each bubble's JQL. Only the excluded bubbles' own JQL is used (no chaining),
/// which keeps it predictable and loop-free.
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
    fn rejects_unknown_and_self_excludes() {
        assert!(resolve_excludes(&[bubble("a", "x", &["zz"])]).is_err());
        assert!(resolve_excludes(&[bubble("a", "x", &["a"])]).is_err());
    }
}
