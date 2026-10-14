//! HTTP handlers. Everything is read from / written to Jira on demand; the only memory is a short-lived
//! cache of the dashboard payload and of icons.

use crate::config::Config;
use crate::jira::{self, Issue, Jira, JiraError};
use actix_web::{HttpResponse, ResponseError, get, http::StatusCode, post, web};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct AppState {
    pub cfg: Config,
    pub icon_dir: std::path::PathBuf,
    pub jira: Jira,
    /// Display name of the Extended Status field (some changelogs only carry names).
    pub ext_field_name: String,
    pub cache: tokio::sync::Mutex<Option<(Instant, Arc<Value>)>>,
    pub icons: Mutex<HashMap<String, (String, Vec<u8>)>>,
}

impl AppState {
    async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}

/* ---------- errors ---------- */

#[derive(Debug)]
pub struct ApiError(anyhow::Error);

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self.0.downcast_ref::<JiraError>() {
            Some(j) if j.status.is_client_error() => StatusCode::from_u16(j.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Some(_) => StatusCode::BAD_GATEWAY,
            None if self.0.downcast_ref::<BadRequest>().is_some() => StatusCode::BAD_REQUEST,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    fn error_response(&self) -> HttpResponse {
        log::warn!("{self}");
        let msg = match self.0.downcast_ref::<JiraError>() {
            Some(j) => j.to_string(),
            None => format!("{:#}", self.0),
        };
        HttpResponse::build(self.status_code()).json(json!({ "error": msg }))
    }
}

#[derive(Debug)]
struct BadRequest(String);
impl std::fmt::Display for BadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BadRequest {}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(BadRequest(msg.into()).into())
}

type ApiResult<T = HttpResponse> = Result<T, ApiError>;

/// Issue keys go into Jira URL paths, so only accept the canonical shape.
fn check_key(key: &str) -> ApiResult<()> {
    let (proj, num) = key.split_once('-').ok_or_else(|| bad("bad issue key"))?;
    let ok = !proj.is_empty()
        && proj.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && proj.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit());
    if ok { Ok(()) } else { Err(bad("bad issue key")) }
}

/* ---------- page + icons ---------- */

/// Only our own script runs; Jira text is always rendered as text, but this is the safety net.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'";

#[get("/")]
pub async fn index() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Content-Security-Policy", CSP))
        .insert_header(("Cache-Control", "no-cache"))
        .body(include_str!("../static/index.html"))
}

#[get("/app.js")]
pub async fn app_js() -> HttpResponse {
    // no-cache: a rebuilt binary must never be paired with a stale script
    HttpResponse::Ok()
        .content_type("text/javascript; charset=utf-8")
        .insert_header(("Cache-Control", "no-cache"))
        .body(include_str!("../static/app.js"))
}

/// Jira-hosted icons/avatars, fetched with our token and kept in memory.
pub fn icon_url(base: &str, url: &str) -> String {
    match url.strip_prefix(base) {
        Some(path) => format!("/icon?u={}", pct(path)),
        None => url.to_string(),
    }
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
pub struct IconQ {
    u: String,
}

const ICON_PREFIXES: &[&str] = &["/secure/viewavatar", "/secure/projectavatar", "/images/", "/download/resources/", "/s/"];
const ICON_TYPES: &[(&str, &str)] = &[("png", "image/png"), ("svg", "image/svg+xml"), ("gif", "image/gif"), ("jpg", "image/jpeg"), ("ico", "image/x-icon")];

fn check_icon_path(u: &str) -> ApiResult<()> {
    if u.starts_with("//") || u.contains("..") || !ICON_PREFIXES.iter().any(|p| u.starts_with(p)) {
        return Err(bad("not an icon path"));
    }
    Ok(())
}

/// File name (without extension) for a Jira icon path: readable and collision-free enough.
fn icon_stem(u: &str) -> String {
    u.trim_start_matches('/').chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Memory, then the local icon folder, then Jira (saving the copy locally).
async fn get_icon(st: &AppState, u: &str) -> ApiResult<(String, Vec<u8>)> {
    if let Some(v) = st.icons.lock().unwrap().get(u).cloned() {
        return Ok(v);
    }
    let stem = icon_stem(u);
    for (ext, ct) in ICON_TYPES {
        if let Ok(bytes) = tokio::fs::read(st.icon_dir.join(format!("{stem}.{ext}"))).await {
            let v = (ct.to_string(), bytes);
            st.icons.lock().unwrap().insert(u.to_string(), v.clone());
            return Ok(v);
        }
    }
    let (ct, bytes) = st.jira.get_bytes(u).await?;
    let ct = ct.split(';').next().unwrap_or_default().trim().to_string();
    match ICON_TYPES.iter().find(|(_, t)| *t == ct) {
        Some((ext, _)) => {
            let path = st.icon_dir.join(format!("{stem}.{ext}"));
            let saved = async { tokio::fs::create_dir_all(&st.icon_dir).await?; tokio::fs::write(&path, &bytes).await }.await;
            if let Err(e) = saved {
                log::warn!("could not save icon {}: {e}", path.display());
            }
        }
        None => log::warn!("icon {u} has unexpected type {ct}; not saved locally"),
    }
    let v = (ct, bytes);
    st.icons.lock().unwrap().insert(u.to_string(), v.clone());
    Ok(v)
}

#[get("/icon")]
pub async fn icon(st: web::Data<AppState>, q: web::Query<IconQ>) -> ApiResult {
    check_icon_path(&q.u)?;
    let (ct, bytes) = get_icon(&st, &q.u).await?;
    Ok(HttpResponse::Ok().content_type(ct).insert_header(("Cache-Control", "max-age=86400")).body(bytes))
}

/// Make sure every icon the page will show is in the local folder before the page asks for it.
async fn prefetch_icons(st: &AppState, paths: Vec<String>) {
    let jobs = paths.iter().filter(|u| check_icon_path(u).is_ok()).map(|u| async move {
        if let Err(e) = get_icon(st, u).await {
            log::warn!("icon {u}: {e}");
        }
    });
    futures_util::future::join_all(jobs).await;
}

/* ---------- dashboard data ---------- */

#[derive(Deserialize)]
pub struct DataQ {
    #[serde(default)]
    refresh: Option<String>,
}

#[get("/api/data")]
pub async fn data(st: web::Data<AppState>, q: web::Query<DataQ>) -> ApiResult {
    let ttl = Duration::from_secs(st.cfg.jira.cache_seconds);
    // Held across the fetch so concurrent page loads share one round of Jira calls.
    let mut cache = st.cache.lock().await;
    if q.refresh.is_none()
        && let Some((at, v)) = cache.as_ref()
        && at.elapsed() < ttl
    {
        return Ok(HttpResponse::Ok().json(v.as_ref()));
    }
    let v = Arc::new(build_data(&st).await?);
    *cache = Some((Instant::now(), v.clone()));
    Ok(HttpResponse::Ok().json(v.as_ref()))
}

async fn build_data(st: &AppState) -> anyhow::Result<Value> {
    let (cfg, j) = (&st.cfg, &st.jira);
    let base = j.base();
    let (issues, types, prios) = tokio::try_join!(
        jira::load_issues(j, cfg, &st.ext_field_name),
        j.get("/rest/api/2/issuetype"),
        j.get("/rest/api/2/priority"),
    )?;

    // Only the issue types that actually show up (Jira may define hundreds).
    let used: std::collections::HashSet<&str> = issues.iter().map(|i| i.t.as_str()).chain(cfg.follow_up.hierarchy.iter().map(String::as_str)).collect();
    let mut icon_paths: Vec<String> = Vec::new();
    let mut local = |url: &str| {
        if let Some(p) = url.strip_prefix(base) {
            icon_paths.push(p.to_string());
        }
        icon_url(base, url)
    };
    let types: HashMap<String, String> = types
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["name"].as_str().is_some_and(|n| used.contains(n)))
        .filter_map(|t| Some((t["name"].as_str()?.to_string(), local(t["iconUrl"].as_str()?))))
        .collect();
    let mut prios: Vec<Value> = prios
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some(json!({ "name": p["name"].as_str()?, "icon": local(p["iconUrl"].as_str().unwrap_or_default()) })))
        .collect();
    let order = &cfg.follow_up.priority_order;
    if !order.is_empty() {
        let rank = |p: &Value| order.iter().position(|o| Some(o.as_str()) == p["name"].as_str()).unwrap_or(usize::MAX);
        prios.sort_by_key(rank);
    }

    let mut bubbles = Vec::new();
    for b in &cfg.bubbles {
        let avatar = match &b.icon_project {
            Some(key) => match j.get(&format!("/rest/api/2/project/{key}")).await {
                Ok(p) => p["avatarUrls"]["24x24"].as_str().map(&mut local),
                Err(e) => {
                    log::warn!("bubble `{}`: no avatar for project {key}: {e:#}", b.id);
                    None
                }
            },
            None => None,
        };
        bubbles.push(json!({ "id": b.id, "name": b.name, "jql": b.jql, "icon": avatar }));
    }

    prefetch_icons(st, icon_paths).await;

    let now = chrono::Local::now();
    Ok(json!({
        "today": now.format("%Y-%m-%d").to_string(),
        "fetchedAt": now.format("%H:%M").to_string(),
        "jiraBase": base,
        "followUp": cfg.follow_up,
        "tabs": cfg.tabs,
        "bubbles": bubbles,
        "types": types,
        "priorities": prios,
        "issues": issues,
    }))
}

/* ---------- reads for menus / popovers ---------- */

#[get("/api/issue/{key}/transitions")]
pub async fn transitions(st: web::Data<AppState>, key: web::Path<String>) -> ApiResult {
    check_key(&key)?;
    let v = st.jira.get(&format!("/rest/api/2/issue/{key}/transitions")).await?;
    let list: Vec<Value> = v["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| json!({ "id": t["id"], "name": t["name"], "to": t["to"]["name"], "cat": t["to"]["statusCategory"]["key"] }))
        .collect();
    Ok(HttpResponse::Ok().json(list))
}

#[derive(Deserialize)]
pub struct UserQ {
    #[serde(default)]
    q: String,
}

#[get("/api/issue/{key}/assignable")]
pub async fn assignable(st: web::Data<AppState>, key: web::Path<String>, q: web::Query<UserQ>) -> ApiResult {
    check_key(&key)?;
    let path = format!("/rest/api/2/user/assignable/search?issueKey={key}&maxResults=20&username={}", pct(&q.q));
    let v = st.jira.get(&path).await?;
    let users: Vec<Value> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| u["active"].as_bool().unwrap_or(true))
        .map(|u| json!({ "name": u["name"], "displayName": u["displayName"] }))
        .collect();
    Ok(HttpResponse::Ok().json(users))
}

/* ---------- writes: each one returns the issue as Jira now has it ---------- */

async fn after_write(st: &AppState, key: &str) -> ApiResult {
    st.invalidate().await;
    let issue: Issue = jira::load_issue(&st.jira, &st.cfg, &st.ext_field_name, key).await?;
    Ok(HttpResponse::Ok().json(issue))
}

#[derive(Deserialize)]
pub struct ExtBody {
    value: String,
}

#[post("/api/issue/{key}/ext")]
pub async fn set_ext(st: web::Data<AppState>, key: web::Path<String>, b: web::Json<ExtBody>) -> ApiResult {
    check_key(&key)?;
    // Single-line text field.
    let v = b.value.split_whitespace().collect::<Vec<_>>().join(" ");
    if v.chars().count() > st.cfg.follow_up.ext_max_len {
        return Err(bad(format!("Extended Status is limited to {} characters", st.cfg.follow_up.ext_max_len)));
    }
    let field = &st.cfg.fields.extended_status;
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { field: v } })).await?;
    after_write(&st, &key).await
}

#[derive(Deserialize)]
pub struct NameBody {
    name: Option<String>,
}

#[post("/api/issue/{key}/priority")]
pub async fn set_priority(st: web::Data<AppState>, key: web::Path<String>, b: web::Json<NameBody>) -> ApiResult {
    check_key(&key)?;
    let name = b.name.as_deref().filter(|n| !n.is_empty()).ok_or_else(|| bad("priority name required"))?;
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { "priority": { "name": name } } })).await?;
    after_write(&st, &key).await
}

#[post("/api/issue/{key}/assignee")]
pub async fn set_assignee(st: web::Data<AppState>, key: web::Path<String>, b: web::Json<NameBody>) -> ApiResult {
    check_key(&key)?;
    let name = b.name.as_deref().filter(|n| !n.is_empty());
    st.jira.put(&format!("/rest/api/2/issue/{key}/assignee"), &json!({ "name": name })).await?;
    after_write(&st, &key).await
}

#[derive(Deserialize)]
pub struct DateBody {
    date: Option<String>,
}

#[post("/api/issue/{key}/target")]
pub async fn set_target(st: web::Data<AppState>, key: web::Path<String>, b: web::Json<DateBody>) -> ApiResult {
    check_key(&key)?;
    let date = b.date.as_deref().filter(|d| !d.is_empty());
    if let Some(d) = date {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| bad("date must be YYYY-MM-DD"))?;
    }
    let field = &st.cfg.fields.target_end;
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { field: date } })).await?;
    after_write(&st, &key).await
}

#[derive(Deserialize)]
pub struct TransitionBody {
    id: String,
}

#[post("/api/issue/{key}/transition")]
pub async fn transition(st: web::Data<AppState>, key: web::Path<String>, b: web::Json<TransitionBody>) -> ApiResult {
    check_key(&key)?;
    if b.id.is_empty() || !b.id.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad("bad transition id"));
    }
    st.jira.post(&format!("/rest/api/2/issue/{key}/transitions"), &json!({ "transition": { "id": b.id } })).await?;
    after_write(&st, &key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_keys() {
        assert!(check_key("DV-1").is_ok());
        assert!(check_key("DPSVSH-12").is_ok());
        for k in ["dv-1", "DV-", "-1", "DV-1/../x", "DV-1?x", "DV"] {
            assert!(check_key(k).is_err(), "{k}");
        }
    }

    #[test]
    fn icon_urls() {
        let b = "https://jira.x";
        assert_eq!(icon_url(b, "https://jira.x/secure/viewavatar?size=xsmall&avatarId=1"), "/icon?u=/secure/viewavatar%3Fsize%3Dxsmall%26avatarId%3D1");
        assert_eq!(icon_url(b, "https://other/x.png"), "https://other/x.png");
    }
}
