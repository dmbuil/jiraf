//! Tauri commands called by the page. Everything is read from / written to Jira on demand; the only
//! memory is a short-lived cache of the dashboard payload and of icons.
//!
//! | Command (args) | Returns |
//! |---|---|
//! | [`data`] `(refresh)` | the dashboard payload ([`build_data`]) |
//! | [`transitions`] · [`priorities`] `(key)`, [`assignable`] `(key, q)` | lists for the edit menus |
//! | [`set_ext`] `(key, value)` · [`set_priority`] `(key, name)` · [`set_assignee`] `(key, name)` · [`set_target`] `(key, date)` · [`set_due`] `(key, date)` · [`transition`] `(key, id)` | the updated [`Issue`] |
//!
//! Every command fails with "jiraf is not ready" until startup succeeded ([`Holder::ready`]). Errors
//! reach the page as their message (see [`ApiError`]).

use crate::app::Holder;
use crate::config::Config;
use crate::jira::{self, Issue, Jira, JiraError};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::State;
use std::time::{Duration, Instant};

/// State shared by all commands, built by [`crate::app::init`].
pub struct AppState {
    /// The loaded configuration.
    pub cfg: Config,
    /// Folder holding local copies of Jira icons ([`crate::config::Server::icon_dir`]).
    pub icon_dir: std::path::PathBuf,
    /// Jira client.
    pub jira: Jira,
    /// Display name of the Extended Status field (some changelogs only carry names).
    pub ext_field_name: String,
    /// The last dashboard payload and when it was built. A tokio mutex, so it can be held while
    /// fetching: concurrent page loads then share one round of Jira calls.
    pub cache: tokio::sync::Mutex<Option<(Instant, Arc<Value>)>>,
    /// Icons already served, keyed by Jira path: `(content type, bytes)`.
    pub icons: Mutex<HashMap<String, (String, Vec<u8>)>>,
}

impl AppState {
    /// Drop the cached dashboard payload, so the next [`data`] reads Jira again.
    async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}

/* ---------- errors ---------- */

/// Any command error. Reaches the page as its message: Jira's own text for a [`JiraError`], the full
/// error chain otherwise. Logged when sent.
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
impl Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        log::warn!("{self}");
        let msg = match self.0.downcast_ref::<JiraError>() {
            Some(j) => j.to_string(),
            None => format!("{:#}", self.0),
        };
        ser.serialize_str(&msg)
    }
}

/// Marker error for invalid input from the page (bad key, date, length…).
#[derive(Debug)]
struct BadRequest(String);
impl std::fmt::Display for BadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BadRequest {}

/// An [`ApiError`] for invalid input, with the given message.
pub fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(BadRequest(msg.into()).into())
}

/// Result type of commands and helpers (a JSON value by default).
pub type ApiResult<T = Value> = Result<T, ApiError>;

/// Issue keys go into Jira URL paths, so only accept the canonical shape: `PROJ-123`, where the
/// project starts with an uppercase letter and contains only `A-Z`, digits and `_`.
///
/// # Errors
/// An [`ApiError`] for anything else.
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

/// Origin of the `jiraf` URI scheme as the page sees it: Tauri exposes custom schemes as
/// `<scheme>://localhost` on macOS/Linux and `http://<scheme>.localhost` on Windows (WebView2).
#[cfg(windows)]
pub const ICON_ORIGIN: &str = "http://jiraf.localhost";
/// Origin of the `jiraf` URI scheme as the page sees it: Tauri exposes custom schemes as
/// `<scheme>://localhost` on macOS/Linux and `http://<scheme>.localhost` on Windows (WebView2).
#[cfg(not(windows))]
pub const ICON_ORIGIN: &str = "jiraf://localhost";

/// Rewrite a Jira-hosted icon/avatar URL to jiraf's own `<ICON_ORIGIN>/icon?u=<path>` scheme, so the
/// page never talks to Jira. URLs on other hosts are returned unchanged.
pub fn icon_url(base: &str, url: &str) -> String {
    match url.strip_prefix(base) {
        Some(path) => format!("{ICON_ORIGIN}/icon?u={}", pct(path)),
        None => url.to_string(),
    }
}

/// Avatar URL for the page: the icon scheme URL when Jira hosts it, `None` otherwise (other hosts
/// are blocked by the CSP, and we don't want the browser calling them anyway).
fn local_avatar(base: &str, url: &str) -> Option<String> {
    url.strip_prefix(base).filter(|p| check_icon_path(p).is_ok()).map(|_| icon_url(base, url))
}

/// Percent-encode everything except unreserved characters and `/` (for query-string values).
fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Jira paths the icon proxy is allowed to fetch: avatars, built-in icons and static resources.
const ICON_PREFIXES: &[&str] = &["/secure/viewavatar", "/secure/projectavatar", "/secure/useravatar", "/images/", "/download/resources/", "/s/"];
/// Content types saved to the icon folder, with the file extension used for each.
const ICON_TYPES: &[(&str, &str)] = &[("png", "image/png"), ("svg", "image/svg+xml"), ("gif", "image/gif"), ("jpg", "image/jpeg"), ("ico", "image/x-icon")];

/// Only let the proxy fetch paths under [`ICON_PREFIXES`], with no protocol-relative `//` and nothing
/// that could climb out of them: `..`, `\\`, or their percent-encoded forms (`%2e`, `%5c`), which the URL
/// parser would turn back into `..` and `/`.
///
/// # Errors
/// An [`ApiError`] for anything else.
fn check_icon_path(u: &str) -> ApiResult<()> {
    let lower = u.to_ascii_lowercase();
    let climbs = u.contains("..") || u.contains('\\') || lower.contains("%2e") || lower.contains("%5c");
    if u.starts_with("//") || climbs || !ICON_PREFIXES.iter().any(|p| u.starts_with(p)) {
        return Err(bad("not an icon path"));
    }
    Ok(())
}

/// File name (without extension) for a Jira icon path: readable and collision-free enough.
fn icon_stem(u: &str) -> String {
    u.trim_start_matches('/').chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Fetch an icon: from memory, then the local icon folder, then Jira (saving the copy locally).
///
/// A failed save only logs a warning; the icon is still served.
///
/// # Errors
/// If Jira won't return the icon.
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

/// The Jira path in an icon scheme request (`…/icon?u=<path>`, either origin), decoded.
fn icon_query(uri: &str) -> Option<String> {
    let url = reqwest::Url::parse(uri).ok()?;
    if url.path() != "/icon" {
        return None;
    }
    url.query_pairs().find(|(k, _)| k == "u").map(|(_, v)| v.into_owned())
}

/// Answer one `jiraf://` (or `http://jiraf.localhost`) request: the icon with its content type,
/// cached for a day by the webview (errors are never cached). `400` for anything but an allowed icon path, `503` before startup
/// succeeded, `502` if Jira won't return it.
pub async fn icon_response(holder: &Holder, uri: &str) -> tauri::http::Response<Vec<u8>> {
    let reply = |status: u16, ct: &str, body: Vec<u8>| {
        // only successes are cached: a transient failure must not stick for a day
        let cache = if status == 200 { "max-age=86400" } else { "no-store" };
        tauri::http::Response::builder().status(status).header("Content-Type", ct).header("Cache-Control", cache).body(body).unwrap()
    };
    let Some(u) = icon_query(uri).filter(|u| check_icon_path(u).is_ok()) else {
        log::warn!("icon request refused: {uri}");
        return reply(400, "text/plain", b"not an icon path".to_vec());
    };
    let Ok(st) = holder.ready() else {
        return reply(503, "text/plain", b"jiraf is not ready".to_vec());
    };
    match get_icon(&st, &u).await {
        Ok((ct, bytes)) => reply(200, &ct, bytes),
        Err(e) => {
            log::warn!("icon {u}: {e}");
            reply(502, "text/plain", e.to_string().into_bytes())
        }
    }
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

/// The dashboard payload (see [`build_data`]), served from the cache while it is younger than
/// `cache_seconds`, unless `refresh` is set.
#[tauri::command]
pub async fn data(holder: State<'_, Holder>, refresh: bool) -> ApiResult {
    let st = holder.ready()?;
    let ttl = Duration::from_secs(st.cfg.jira.cache_seconds);
    // Held across the fetch so concurrent page loads share one round of Jira calls.
    let mut cache = st.cache.lock().await;
    if !refresh
        && let Some((at, v)) = cache.as_ref()
        && at.elapsed() < ttl
    {
        return Ok(v.as_ref().clone());
    }
    let v = Arc::new(build_data(&st).await?);
    *cache = Some((Instant::now(), v.clone()));
    Ok(v.as_ref().clone())
}

/// Build the dashboard payload:
///
/// ```json
/// { "today": "YYYY-MM-DD", "fetchedAt": "HH:MM", "jiraBase": "…",
///   "followUp": {…}, "tabs": […], "bubbles": [{ "id", "name", "jql", "icon" }],
///   "types": { "Epic": "jiraf://localhost/icon?u=…" }, "priorities": [{ "name", "icon" }],
///   "issues": [Issue…] }
/// ```
///
/// Issue types are limited to those in use; priorities follow `priority_order` when set. Every icon
/// referenced is fetched into the icon folder before returning.
///
/// # Errors
/// If any bubble search, or the issue type / priority lists, can't be read.
async fn build_data(st: &AppState) -> anyhow::Result<Value> {
    let (cfg, j) = (&st.cfg, &st.jira);
    let base = j.base();
    let (mut issues, types, prios) = tokio::try_join!(
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

    // Assignee avatars: Jira-hosted ones go through the proxy (and get prefetched), others are dropped.
    for i in &mut issues {
        i.av = i.av.take().filter(|u| local_avatar(base, u).is_some()).map(|u| local(&u));
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

/// Transitions the workflow allows from the issue's current status, as `[{ "id", "name", "to", "cat" }]`
/// (`to` = target status, `cat` = its category key).
#[tauri::command]
pub async fn transitions(holder: State<'_, Holder>, key: String) -> ApiResult<Vec<Value>> {
    let st = holder.ready()?;
    check_key(&key)?;
    let v = st.jira.get(&format!("/rest/api/2/issue/{key}/transitions")).await?;
    let list: Vec<Value> = v["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| json!({ "id": t["id"], "name": t["name"], "to": t["to"]["name"], "cat": t["to"]["statusCategory"]["key"] }))
        .collect();
    Ok(list)
}

/// Names of the priorities this issue may take, in Jira's order.
///
/// Jira DC assigns priority schemes per project, so the global priority list can hold values a
/// given issue can't use; the issue's edit metadata has the real list.
#[tauri::command]
pub async fn priorities(holder: State<'_, Holder>, key: String) -> ApiResult<Vec<String>> {
    let st = holder.ready()?;
    check_key(&key)?;
    let v = st.jira.get(&format!("/rest/api/2/issue/{key}/editmeta")).await?;
    let names = v["fields"]["priority"]["allowedValues"].as_array().into_iter().flatten().filter_map(|p| p["name"].as_str().map(String::from)).collect();
    Ok(names)
}

/// Up to 20 active users who can be assigned the issue, matching `q` (usernames / display names; empty
/// lists the first matches), as `[{ "name", "displayName", "avatar" }]` (`avatar`: local icon URL, or
/// null when Jira doesn't host it).
#[tauri::command]
pub async fn assignable(holder: State<'_, Holder>, key: String, q: String) -> ApiResult<Vec<Value>> {
    let st = holder.ready()?;
    check_key(&key)?;
    let path = format!("/rest/api/2/user/assignable/search?issueKey={key}&maxResults=20&username={}", pct(&q));
    let v = st.jira.get(&path).await?;
    let users: Vec<Value> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| u["active"].as_bool().unwrap_or(true))
        .map(|u| {
            let avatar = u["avatarUrls"]["48x48"].as_str().and_then(|a| local_avatar(st.jira.base(), a));
            json!({ "name": u["name"], "displayName": u["displayName"], "avatar": avatar })
        })
        .collect();
    Ok(users)
}

/* ---------- writes: each one returns the issue as Jira now has it ---------- */

/// Common tail of every write: invalidate the cache and answer with the issue as Jira now has it.
async fn after_write(st: &AppState, key: &str) -> ApiResult<Issue> {
    st.invalidate().await;
    let mut issue: Issue = jira::load_issue(&st.jira, &st.cfg, &st.ext_field_name, key).await?;
    issue.av = issue.av.and_then(|u| local_avatar(st.jira.base(), &u));
    Ok(issue)
}

/// Set the Extended Status to `value` (whitespace, including newlines, collapsed to single spaces).
/// Returns the updated [`Issue`].
///
/// # Errors
/// If longer than `ext_max_len`, or Jira refuses.
#[tauri::command]
pub async fn set_ext(holder: State<'_, Holder>, key: String, value: String) -> ApiResult<Issue> {
    let st = holder.ready()?;
    check_key(&key)?;
    // Single-line text field.
    let v = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if v.chars().count() > st.cfg.follow_up.ext_max_len {
        return Err(bad(format!("Extended Status is limited to {} characters", st.cfg.follow_up.ext_max_len)));
    }
    let field = &st.cfg.fields.extended_status;
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { field: v } })).await?;
    after_write(&st, &key).await
}

/// Set the priority to `name`. Returns the updated [`Issue`].
#[tauri::command]
pub async fn set_priority(holder: State<'_, Holder>, key: String, name: Option<String>) -> ApiResult<Issue> {
    let st = holder.ready()?;
    check_key(&key)?;
    let name = name.as_deref().filter(|n| !n.is_empty()).ok_or_else(|| bad("priority name required"))?;
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { "priority": { "name": name } } })).await?;
    after_write(&st, &key).await
}

/// Assign the issue to username `name`, or unassign it (`None` / empty). Returns the updated [`Issue`].
#[tauri::command]
pub async fn set_assignee(holder: State<'_, Holder>, key: String, name: Option<String>) -> ApiResult<Issue> {
    let st = holder.ready()?;
    check_key(&key)?;
    let name = name.as_deref().filter(|n| !n.is_empty());
    st.jira.put(&format!("/rest/api/2/issue/{key}/assignee"), &json!({ "name": name })).await?;
    after_write(&st, &key).await
}

/// Set Target end to `date` (`YYYY-MM-DD`), or clear it (`None` / empty). Returns the updated [`Issue`].
#[tauri::command]
pub async fn set_target(holder: State<'_, Holder>, key: String, date: Option<String>) -> ApiResult<Issue> {
    let st = holder.ready()?;
    set_date(&st, &key, &st.cfg.fields.target_end, date.as_deref()).await
}

/// Set Jira's Due date to `date` (`YYYY-MM-DD`), or clear it (`None` / empty). Returns the updated [`Issue`].
#[tauri::command]
pub async fn set_due(holder: State<'_, Holder>, key: String, date: Option<String>) -> ApiResult<Issue> {
    let st = holder.ready()?;
    set_date(&st, &key, "duedate", date.as_deref()).await
}

/// Shared body of the date writes: validate the key and the date, then set `field` (or clear it).
///
/// # Errors
/// For a bad key or a date that isn't `YYYY-MM-DD`; Jira's own error otherwise.
async fn set_date(st: &AppState, key: &str, field: &str, date: Option<&str>) -> ApiResult<Issue> {
    check_key(key)?;
    let date = date.filter(|d| !d.is_empty());
    if let Some(d) = date {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| bad("date must be YYYY-MM-DD"))?;
    }
    st.jira.put(&format!("/rest/api/2/issue/{key}"), &json!({ "fields": { field: date } })).await?;
    after_write(st, key).await
}

/// Move the issue through workflow transition `id` (from [`transitions`], digits only). Returns the
/// updated [`Issue`].
#[tauri::command]
pub async fn transition(holder: State<'_, Holder>, key: String, id: String) -> ApiResult<Issue> {
    let st = holder.ready()?;
    check_key(&key)?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad("bad transition id"));
    }
    st.jira.post(&format!("/rest/api/2/issue/{key}/transitions"), &json!({ "transition": { "id": id } })).await?;
    after_write(&st, &key).await
}

/* ---------- links ---------- */

/// Whether the page may open `url` in the browser: only pages of the configured Jira (`base` + `/…`).
fn jira_link_allowed(base: &str, url: &str) -> bool {
    url.starts_with(&format!("{base}/"))
}

/// Open a Jira link (issue, search, comments) in the default browser.
///
/// # Errors
/// For any URL outside the configured Jira, or if the system can't open it.
#[tauri::command]
pub fn open_external(app: tauri::AppHandle, holder: State<'_, Holder>, url: String) -> ApiResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let st = holder.ready()?;
    if !jira_link_allowed(st.jira.base(), &url) {
        log::warn!("refused to open {url}");
        return Err(bad("only Jira links can be opened"));
    }
    app.opener().open_url(url, None::<&str>)?;
    Ok(())
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
        assert_eq!(icon_url(b, "https://jira.x/secure/viewavatar?size=xsmall&avatarId=1"), format!("{ICON_ORIGIN}/icon?u=/secure/viewavatar%3Fsize%3Dxsmall%26avatarId%3D1"));
        assert_eq!(icon_url(b, "https://other/x.png"), "https://other/x.png");
    }

    #[test]
    fn icon_paths_cannot_climb_out() {
        assert!(check_icon_path("/images/icons/priorities/major.svg").is_ok());
        assert!(check_icon_path("/secure/viewavatar?size=xsmall&avatarId=1").is_ok());
        // the URL parser treats %2e%2e (any case) as `..` and `\` as `/`
        for u in ["/images/../rest/api/2/myself", "/images/%2e%2e/%2e%2e/rest/api/2/myself", "/images/%2E%2e/x", "/images/.%2e/x", "/images\\..\\x", "/images/%5c..%5cx"] {
            assert!(check_icon_path(u).is_err(), "{u}");
        }
    }

    #[test]
    fn jira_links_only() {
        let b = "https://jira.x";
        for u in ["https://jira.x/browse/DV-1", "https://jira.x/issues/?jql=labels%20%3D%20%22a%22"] {
            assert!(jira_link_allowed(b, u), "{u}");
        }
        for u in ["https://jira.x", "https://jira.x.evil/browse/DV-1", "https://jira.x@evil/", "http://jira.x/browse/DV-1", "javascript:alert(1)"] {
            assert!(!jira_link_allowed(b, u), "{u}");
        }
    }

    #[tokio::test]
    async fn icon_errors_are_not_cached() {
        let h = Holder::new("/x".into());
        for uri in ["jiraf://localhost/icon?u=/rest/api/2/myself", "jiraf://localhost/icon?u=/images/x.png"] {
            let r = icon_response(&h, uri).await;
            assert!(r.status().as_u16() >= 400, "{uri}");
            assert_eq!(r.headers()["Cache-Control"], "no-store", "{uri}");
        }
    }

    #[test]
    fn icon_origin_per_platform() {
        assert_eq!(ICON_ORIGIN, if cfg!(windows) { "http://jiraf.localhost" } else { "jiraf://localhost" });
    }

    #[test]
    fn icon_query_round_trip() {
        let b = "https://jira.x";
        for p in ["/secure/viewavatar?size=xsmall&avatarId=1", "/images/icons/a b%20c.png"] {
            assert_eq!(icon_query(&icon_url(b, &format!("{b}{p}"))).as_deref(), Some(p));
        }
        assert_eq!(icon_query("jiraf://localhost/other?u=/images/x"), None);
        assert_eq!(icon_query("http://jiraf.localhost/icon?u=/images/x").as_deref(), Some("/images/x"));
    }
}
