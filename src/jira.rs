//! Thin Jira Data Center REST v2 client (Bearer PAT) plus the mapping from raw issues to the
//! compact shape the page uses.
//!
//! - [`Jira`]: HTTP client bound to one instance and token.
//! - [`load_issues`]: everything the dashboard needs, across all bubbles, with missing ancestors.
//! - [`map_issue`]: raw Jira JSON → [`Issue`], including the Extended Status age from the changelog.

use crate::config::{Config, Fields};
use anyhow::{Context, Result, anyhow};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Jira REST client for one instance, authenticated with a personal access token (Bearer).
///
/// Cheap to clone: the underlying `reqwest::Client` is reference-counted.
#[derive(Clone)]
pub struct Jira {
    /// Shared HTTP client (connection pool, 60 s timeout, `jiraf/<version>` user agent).
    http: reqwest::Client,
    /// Base URL without trailing slash, e.g. `https://jira.example.com`.
    base: String,
    /// Personal access token sent as `Authorization: Bearer …`.
    token: String,
}

/// A Jira error worth showing to the user as-is (validation messages, permission problems…).
#[derive(Debug)]
pub struct JiraError {
    /// HTTP status Jira answered with.
    pub status: StatusCode,
    /// Jira's own messages (`errorMessages` and per-field `errors`), joined with `; `.
    pub message: String,
}

impl std::fmt::Display for JiraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jira answered {}: {}", self.status, self.message)
    }
}
impl std::error::Error for JiraError {}

impl Jira {
    /// Build a client for `cfg.jira.url` using `cfg.token`. Makes no request yet.
    ///
    /// # Errors
    /// If the HTTP client can't be built (e.g. TLS backend failure).
    pub fn new(cfg: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("jiraf/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { http, base: cfg.jira.url.clone(), token: cfg.token.clone() })
    }

    /// Base URL of the instance (no trailing slash).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Send one JSON request to `base + path`.
    ///
    /// Returns `Ok(None)` for empty bodies (Jira answers `204` to PUTs and transitions).
    ///
    /// # Errors
    /// A [`JiraError`] for non-2xx answers; a transport or decoding error otherwise.
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

    /// `GET base + path`, expecting a JSON body.
    ///
    /// # Errors
    /// As [`Jira::call`], plus an error if the body is empty.
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.call(Method::GET, path, None).await?.ok_or_else(|| anyhow!("empty response from GET {path}"))
    }
    /// `POST base + path` with a JSON body. `Ok(None)` when Jira answers with no content.
    ///
    /// # Errors
    /// As [`Jira::call`].
    pub async fn post(&self, path: &str, body: &Value) -> Result<Option<Value>> {
        self.call(Method::POST, path, Some(body)).await
    }
    /// `PUT base + path` with a JSON body. `Ok(None)` when Jira answers with no content (the usual case).
    ///
    /// # Errors
    /// As [`Jira::call`].
    pub async fn put(&self, path: &str, body: &Value) -> Result<Option<Value>> {
        self.call(Method::PUT, path, Some(body)).await
    }

    /// Raw bytes for icons/avatars (they need auth on most DC instances), with their content type.
    ///
    /// # Errors
    /// A [`JiraError`] for non-2xx answers, or a transport error.
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

    /// All pages of a JQL search (POST, so long `key in (…)` lists are fine), 100 issues per page.
    ///
    /// `fields` limits what Jira returns; `changelog` adds each issue's history (`expand=changelog`).
    ///
    /// # Errors
    /// As [`Jira::call`]; Jira answers `400` for invalid JQL.
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

/// Turn a Jira error body into one readable line: `errorMessages` plus `field: message` pairs.
/// Falls back to the first 300 characters of the raw body when it isn't Jira's JSON error shape.
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
    /// Issue key, e.g. `ABC-123`.
    pub k: String,
    /// Project key.
    pub pj: String,
    /// Summary.
    pub s: String,
    /// Issue type name (Initiative, Epic, Task, Sub-task…).
    pub t: String,
    /// Status name.
    pub st: String,
    /// Status category key: new / indeterminate / done.
    pub sc: String,
    /// Assignee display name.
    pub a: Option<String>,
    /// Assignee username (what Jira DC needs to assign).
    pub an: Option<String>,
    /// Assignee avatar. Jira's absolute URL here; the API rewrites it to the local `/icon` proxy, or
    /// drops it when it isn't hosted by Jira (e.g. Gravatar), and the page then shows initials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub av: Option<String>,
    /// Due date (`YYYY-MM-DD`).
    pub due: Option<String>,
    /// Last update of the issue (`YYYY-MM-DD`).
    pub upd: Option<String>,
    /// Priority name.
    pub pr: Option<String>,
    /// Parent key: Sub-task parent, else Parent Link, else Epic Link.
    pub p: Option<String>,
    /// Extended Status text (empty when unset).
    pub ext: String,
    /// Date (`YYYY-MM-DD`) of the last change to Extended Status, from the changelog.
    /// `None` if it was never edited after creation.
    #[serde(rename = "extAt")]
    pub ext_at: Option<String>,
    /// Display name of whoever made that last change.
    #[serde(rename = "extBy")]
    pub ext_by: Option<String>,
    /// Target start (`YYYY-MM-DD`).
    pub ts: Option<String>,
    /// Target end (`YYYY-MM-DD`).
    pub te: Option<String>,
    /// Resolution date (`YYYY-MM-DD`), set once the issue is done.
    pub res: Option<String>,
    /// Activity Type value, which decides the tab (mostly set on Initiatives; descendants inherit it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Plain Jira labels.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lb: Vec<String>,
    /// Ids of the bubbles whose JQL returned this issue.
    pub bubbles: Vec<String>,
    /// Ancestor fetched only for hierarchy context (closed or outside every filter).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ctx: bool,
    /// Number of comments.
    pub nc: usize,
    /// The latest comment, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c: Option<Comment>,
}

/// The latest comment of an issue, for the hover popover.
#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    /// Author display name.
    pub a: Option<String>,
    /// Creation date (`YYYY-MM-DD`).
    pub d: Option<String>,
    /// Body as plain text (Jira wiki markup is not rendered), cut at [`COMMENT_MAX`] characters.
    pub b: String,
}

/// Long comments are cut for the popover; the full text is one click away in Jira.
const COMMENT_MAX: usize = 3000;

/// Field ids to request in searches: the standard ones jiraf shows plus the configured custom fields.
pub fn search_fields(f: &Fields) -> Vec<String> {
    let mut v: Vec<String> = ["summary", "issuetype", "status", "assignee", "duedate", "updated", "priority", "parent", "project", "resolutiondate", "comment", "labels"]
        .map(String::from)
        .to_vec();
    v.extend([&f.extended_status, &f.activity_type, &f.parent_link, &f.epic_link, &f.target_start, &f.target_end].map(|s| s.clone()));
    v
}

/// A non-empty JSON string as `String`; `None` for null, missing or empty values.
fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|x| !x.is_empty()).map(String::from)
}
/// The date part (`YYYY-MM-DD`) of a Jira date or datetime string.
fn day(v: &Value) -> Option<String> {
    v.as_str().map(|x| x.chars().take(10).collect())
}
/// Parent Link / Epic Link come back as a plain key on DC, but be lenient with objects too.
fn key_of(v: &Value) -> Option<String> {
    s(v).or_else(|| s(&v["key"]))
}

/// Map one raw issue (as returned by search or `GET /issue`) to the compact [`Issue`].
///
/// `ext_field_name` is the display name of the Extended Status field, used to spot it in changelogs
/// that only carry names. `bubbles` starts empty and `ctx` false; the caller fills them in.
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
        av: s(&x["assignee"]["avatarUrls"]["48x48"]),
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
        at: s(&x[&f.activity_type]["value"]),
        lb: x["labels"].as_array().into_iter().flatten().filter_map(s).collect(),
        bubbles: Vec::new(),
        ctx: false,
        nc,
        c,
    }
}

/// Most recent changelog entry touching the given field (matched by id, or by name on older DC versions).
///
/// Returns its date (`YYYY-MM-DD`) and author display name, or `(None, None)` if the field was never
/// changed after the issue was created (or the changelog wasn't requested).
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

/// Drop a trailing `ORDER BY …` (case-insensitive) and surrounding whitespace.
///
/// `ORDER BY` can't live inside the parentheses jiraf wraps user JQL in.
pub fn strip_order_by(jql: &str) -> &str {
    match jql.to_ascii_lowercase().rfind("order by") {
        Some(i) => jql[..i].trim_end(),
        None => jql.trim(),
    }
}

/// Fetches every bubble's issues (open + recently closed), tags each with its bubbles, then pulls in
/// missing ancestors so the tree is complete.
///
/// Bubble searches run in parallel and include the changelog. Ancestors are fetched up to four levels,
/// marked [`Issue::ctx`], and skipped (with a warning) if Jira won't return them.
///
/// # Errors
/// If any bubble's search fails; the error names the bubble.
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

/// One issue, freshly read with its changelog (used right after an edit).
///
/// # Errors
/// As [`Jira::get`]; `404` if the issue doesn't exist or isn't visible.
pub async fn load_issue(jira: &Jira, cfg: &Config, ext_field_name: &str, key: &str) -> Result<Issue> {
    let fields = search_fields(&cfg.fields).join(",");
    let raw = jira.get(&format!("/rest/api/2/issue/{key}?fields={fields}&expand=changelog")).await?;
    Ok(map_issue(&raw, &cfg.fields, ext_field_name))
}

/// Map of every field id to its display name (`GET /rest/api/2/field`).
///
/// Used to resolve the Extended Status field's name, because changelog items on some DC versions only
/// carry the name.
///
/// # Errors
/// As [`Jira::get`].
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
                "customfield_12303": {"value": "Nuevo Roadmap", "id": "1"}, "labels": ["CloudCompute", ""], "customfield_10267": "2026-10-30"
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
        assert_eq!(i.at.as_deref(), Some("Nuevo Roadmap"));
        assert_eq!(i.lb, vec!["CloudCompute"]);
        assert_eq!(i.nc, 0);
        assert!(i.c.is_none());
    }

    #[test]
    fn jira_error_text() {
        assert_eq!(error_text(r#"{"errorMessages":["No"],"errors":{"priority":"bad"}}"#), "No; priority: bad");
        assert_eq!(error_text("plain"), "plain");
    }
}
