//! Thin Jira Data Center REST v2 client (Bearer PAT) plus the mapping from raw issues to the
//! compact shape the page uses.

use crate::config::{Config, Fields};
use anyhow::{Context, Result, anyhow};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone)]
pub struct Jira {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// A Jira error worth showing to the user as-is (validation messages, permission problems…).
#[derive(Debug)]
pub struct JiraError {
    pub status: StatusCode,
    pub message: String,
}

impl std::fmt::Display for JiraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jira answered {}: {}", self.status, self.message)
    }
}
impl std::error::Error for JiraError {}

impl Jira {
    pub fn new(cfg: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("jiraf/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { http, base: cfg.jira.url.clone(), token: cfg.token.clone() })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Option<Value>> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.request(method.clone(), &url).bearer_auth(&self.token).header("Accept", "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.with_context(|| format!("{method} {path}"))?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(JiraError { status, message: error_text(&text) }.into());
        }
        if text.trim().is_empty() {
            return Ok(None); // 204 on PUT / transitions
        }
        Ok(Some(serde_json::from_str(&text).with_context(|| format!("decoding {method} {path}"))?))
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.call(Method::GET, path, None).await?.ok_or_else(|| anyhow!("empty response from GET {path}"))
    }
    pub async fn post(&self, path: &str, body: &Value) -> Result<Option<Value>> {
        self.call(Method::POST, path, Some(body)).await
    }
    pub async fn put(&self, path: &str, body: &Value) -> Result<Option<Value>> {
        self.call(Method::PUT, path, Some(body)).await
    }

    /// Raw bytes for icons/avatars (they need auth on most DC instances).
    pub async fn get_bytes(&self, path: &str) -> Result<(String, Vec<u8>)> {
        let resp = self.http.get(format!("{}{}", self.base, path)).bearer_auth(&self.token).send().await?;
        if !resp.status().is_success() {
            return Err(JiraError { status: resp.status(), message: "image fetch failed".into() }.into());
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        Ok((ct, resp.bytes().await?.to_vec()))
    }

    /// All pages of a JQL search (POST, so long `key in (…)` lists are fine).
    pub async fn search(&self, jql: &str, fields: &[String], changelog: bool) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            let mut body = json!({ "jql": jql, "startAt": out.len(), "maxResults": 100, "fields": fields });
            if changelog {
                body["expand"] = json!(["changelog"]);
            }
            let page = self.post("/rest/api/2/search", &body).await?.unwrap_or_default();
            let issues = page["issues"].as_array().cloned().unwrap_or_default();
            let total = page["total"].as_u64().unwrap_or(0) as usize;
            let got = issues.len();
            out.extend(issues);
            if got == 0 || out.len() >= total {
                return Ok(out);
            }
        }
    }
}

fn error_text(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return body.chars().take(300).collect();
    };
    let mut msgs: Vec<String> = v["errorMessages"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(String::from)).collect();
    if let Some(errs) = v["errors"].as_object() {
        msgs.extend(errs.iter().map(|(k, m)| format!("{k}: {}", m.as_str().unwrap_or_default())));
    }
    if msgs.is_empty() { body.chars().take(300).collect() } else { msgs.join("; ") }
}

/// The compact issue shape the page works with (short keys keep the payload small).
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub k: String,
    pub pj: String,
    pub s: String,
    pub t: String,
    pub st: String,
    /// Status category key: new / indeterminate / done.
    pub sc: String,
    /// Assignee display name and username.
    pub a: Option<String>,
    pub an: Option<String>,
    pub due: Option<String>,
    pub upd: Option<String>,
    pub pr: Option<String>,
    /// Parent key: Sub-task parent, else Parent Link, else Epic Link.
    pub p: Option<String>,
    pub ext: String,
    /// Date/author of the last change to Extended Status, from the changelog.
    #[serde(rename = "extAt")]
    pub ext_at: Option<String>,
    #[serde(rename = "extBy")]
    pub ext_by: Option<String>,
    pub ts: Option<String>,
    pub te: Option<String>,
    pub res: Option<String>,
    pub il: Vec<String>,
    /// Ids of the bubbles whose JQL returned this issue.
    pub bubbles: Vec<String>,
    /// Ancestor fetched only for hierarchy context (closed or outside every filter).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ctx: bool,
    /// Comment count and the latest comment.
    pub nc: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c: Option<Comment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub a: Option<String>,
    pub d: Option<String>,
    pub b: String,
}

/// Long comments are cut for the popover; the full text is one click away in Jira.
const COMMENT_MAX: usize = 3000;

pub fn search_fields(f: &Fields) -> Vec<String> {
    let mut v: Vec<String> = ["summary", "issuetype", "status", "assignee", "duedate", "updated", "priority", "parent", "project", "resolutiondate", "comment"]
        .map(String::from)
        .to_vec();
    v.extend([&f.extended_status, &f.initiative_label, &f.parent_link, &f.epic_link, &f.target_start, &f.target_end].map(|s| s.clone()));
    v
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|x| !x.is_empty()).map(String::from)
}
fn day(v: &Value) -> Option<String> {
    v.as_str().map(|x| x.chars().take(10).collect())
}
/// Parent Link / Epic Link come back as a plain key on DC, but be lenient with objects too.
fn key_of(v: &Value) -> Option<String> {
    s(v).or_else(|| s(&v["key"]))
}

pub fn map_issue(raw: &Value, f: &Fields, ext_field_name: &str) -> Issue {
    let x = &raw["fields"];
    let (ext_at, ext_by) = last_change(raw, &f.extended_status, ext_field_name);
    let comments = x["comment"]["comments"].as_array().map(Vec::as_slice).unwrap_or_default();
    let nc = x["comment"]["total"].as_u64().map_or(comments.len(), |n| n as usize);
    let c = comments.last().map(|c| {
        let body = c["body"].as_str().unwrap_or_default();
        let mut b: String = body.chars().take(COMMENT_MAX).collect();
        if b.len() < body.len() {
            b.push('…');
        }
        Comment { a: s(&c["author"]["displayName"]), d: day(&c["created"]), b }
    });
    Issue {
        k: raw["key"].as_str().unwrap_or_default().to_string(),
        pj: x["project"]["key"].as_str().unwrap_or_default().to_string(),
        s: x["summary"].as_str().unwrap_or_default().to_string(),
        t: x["issuetype"]["name"].as_str().unwrap_or_default().to_string(),
        st: x["status"]["name"].as_str().unwrap_or_default().to_string(),
        sc: x["status"]["statusCategory"]["key"].as_str().unwrap_or("new").to_string(),
        a: s(&x["assignee"]["displayName"]),
        an: s(&x["assignee"]["name"]),
        due: day(&x["duedate"]),
        upd: day(&x["updated"]),
        pr: s(&x["priority"]["name"]),
        p: key_of(&x["parent"]).or_else(|| key_of(&x[&f.parent_link])).or_else(|| key_of(&x[&f.epic_link])),
        ext: x[&f.extended_status].as_str().unwrap_or_default().to_string(),
        ext_at,
        ext_by,
        ts: day(&x[&f.target_start]),
        te: day(&x[&f.target_end]),
        res: day(&x["resolutiondate"]),
        il: x[&f.initiative_label].as_array().into_iter().flatten().filter_map(|l| s(l)).collect(),
        bubbles: Vec::new(),
        ctx: false,
        nc,
        c,
    }
}

/// Most recent changelog entry touching the given field (matched by id, or by name on older DC versions).
fn last_change(raw: &Value, field_id: &str, field_name: &str) -> (Option<String>, Option<String>) {
    let mut best: Option<(&str, Option<&str>)> = None;
    for h in raw["changelog"]["histories"].as_array().into_iter().flatten() {
        let touches = h["items"].as_array().into_iter().flatten().any(|it| it["fieldId"].as_str() == Some(field_id) || it["field"].as_str() == Some(field_name));
        let Some(created) = h["created"].as_str() else { continue };
        if touches && best.is_none_or(|(c, _)| created > c) {
            best = Some((created, h["author"]["displayName"].as_str()));
        }
    }
    best.map_or((None, None), |(c, a)| (Some(c.chars().take(10).collect()), a.map(String::from)))
}

/// `ORDER BY` can't live inside the parentheses we wrap user JQL in.
pub fn strip_order_by(jql: &str) -> &str {
    match jql.to_ascii_lowercase().rfind("order by") {
        Some(i) => jql[..i].trim_end(),
        None => jql.trim(),
    }
}

/// Fetches every bubble's issues (open + recently closed), tags each with its bubbles, then pulls in
/// missing ancestors so the tree is complete.
pub async fn load_issues(jira: &Jira, cfg: &Config, ext_field_name: &str) -> Result<Vec<Issue>> {
    let fields = search_fields(&cfg.fields);
    let searches = cfg.bubbles.iter().map(|b| {
        let jql = format!("({}) AND (statusCategory != Done OR resolved >= -{}d)", strip_order_by(&b.jql), cfg.follow_up.done_days);
        let fields = &fields;
        async move { (b.id.clone(), jira.search(&jql, fields, true).await.with_context(|| format!("bubble `{}`", b.id))) }
    });
    let mut by_key: BTreeMap<String, Issue> = BTreeMap::new();
    for (bubble, res) in futures_util::future::join_all(searches).await {
        for raw in res? {
            let issue = map_issue(&raw, &cfg.fields, ext_field_name);
            by_key.entry(issue.k.clone()).or_insert(issue).bubbles.push(bubble.clone());
        }
    }

    // Ancestors outside the result set (closed, or not matched by any filter): a few levels at most.
    let mut tried: HashSet<String> = HashSet::new();
    for _ in 0..4 {
        let missing: Vec<String> = by_key
            .values()
            .filter_map(|i| i.p.clone())
            .filter(|p| !by_key.contains_key(p) && tried.insert(p.clone()))
            .collect();
        if missing.is_empty() {
            break;
        }
        for chunk in missing.chunks(200) {
            let jql = format!("key in ({})", chunk.join(","));
            // A parent we can't see (permissions, deleted) must not break the dashboard.
            match jira.search(&jql, &fields, false).await {
                Ok(raws) => {
                    for raw in raws {
                        let mut i = map_issue(&raw, &cfg.fields, ext_field_name);
                        i.ctx = true;
                        by_key.insert(i.k.clone(), i);
                    }
                }
                Err(e) => log::warn!("could not load parent issues {chunk:?}: {e:#}"),
            }
        }
    }
    Ok(by_key.into_values().collect())
}

/// One issue, freshly read (after an edit).
pub async fn load_issue(jira: &Jira, cfg: &Config, ext_field_name: &str, key: &str) -> Result<Issue> {
    let fields = search_fields(&cfg.fields).join(",");
    let raw = jira.get(&format!("/rest/api/2/issue/{key}?fields={fields}&expand=changelog")).await?;
    Ok(map_issue(&raw, &cfg.fields, ext_field_name))
}

/// Resolve a custom field's display name (changelog items on some DC versions only carry the name).
pub async fn field_names(jira: &Jira) -> Result<HashMap<String, String>> {
    let all = jira.get("/rest/api/2/field").await?;
    Ok(all
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| Some((f["id"].as_str()?.to_string(), f["name"].as_str()?.to_string())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_order_by() {
        assert_eq!(strip_order_by("project = DV ORDER BY rank"), "project = DV");
        assert_eq!(strip_order_by("project = DV order by key desc"), "project = DV");
        assert_eq!(strip_order_by("  project = DV "), "project = DV");
    }

    #[test]
    fn maps_issue_and_last_ext_change() {
        let f = Fields::default();
        let raw = json!({
            "key": "DV-1",
            "fields": {
                "summary": "Sum", "issuetype": {"name": "Epic"},
                "status": {"name": "Blocked", "statusCategory": {"key": "indeterminate"}},
                "assignee": {"name": "jdoe", "displayName": "JOHN DOE"},
                "project": {"key": "DV"}, "updated": "2026-10-13T10:00:00.000+0200",
                "customfield_10265": "DV-17", "customfield_11501": "note",
                "customfield_11502": ["Roadmap_X"], "customfield_10267": "2026-10-30"
            },
            "changelog": {"histories": [
                {"created": "2026-09-01T08:00:00.000+0200", "author": {"displayName": "A"}, "items": [{"field": "Extended Status"}]},
                {"created": "2026-10-05T08:00:00.000+0200", "author": {"displayName": "B"}, "items": [{"field": "status"}]},
                {"created": "2026-09-20T08:00:00.000+0200", "author": {"displayName": "C"}, "items": [{"fieldId": "customfield_11501", "field": "x"}]}
            ]}
        });
        let i = map_issue(&raw, &f, "Extended Status");
        assert_eq!(i.p.as_deref(), Some("DV-17"));
        assert_eq!(i.an.as_deref(), Some("jdoe"));
        assert_eq!(i.ext_at.as_deref(), Some("2026-09-20"));
        assert_eq!(i.ext_by.as_deref(), Some("C"));
        assert_eq!(i.te.as_deref(), Some("2026-10-30"));
        assert_eq!(i.upd.as_deref(), Some("2026-10-13"));
        assert_eq!(i.il, vec!["Roadmap_X"]);
        assert_eq!(i.nc, 0);
        assert!(i.c.is_none());
    }

    #[test]
    fn jira_error_text() {
        assert_eq!(error_text(r#"{"errorMessages":["No"],"errors":{"priority":"bad"}}"#), "No; priority: bad");
        assert_eq!(error_text("plain"), "plain");
    }
}
