//! Startup and the app state holder.
//!
//! [`init`] runs every check `jiraf` needs before it can serve the dashboard (config, token, Jira,
//! field ids). Its result lives in a [`Holder`], so the page can show Settings (first run), a startup
//! error, or the dashboard, and Retry, Reload config or a Settings save can run [`init`] again
//! without restarting the app.

use crate::api::{self, ApiError, AppState};
use crate::{config, jira};
use anyhow::Context;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Why [`init`] couldn't get the app ready.
#[derive(Debug)]
pub enum Failure {
    /// There is no `jiraf.toml` yet: the page opens Settings.
    Setup,
    /// The token in `.env` is missing, empty or rejected by Jira: the error screen offers to paste a new
    /// one. (A token from the process environment is never reported this way: `.env` can't fix it.)
    Token(String),
    /// Anything else, with the message to show.
    Other(String),
}

/// `e` with its causes, as shown to the user.
fn msg(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

/// Load the config folder `dir`, check the token against Jira and resolve the Extended Status field
/// name, then build the [`AppState`].
///
/// # Errors
/// [`Failure::Setup`] without a `jiraf.toml`; [`Failure::Token`] for a missing, empty or rejected
/// token from `.env`; [`Failure::Other`] for anything else (bad config, Jira unreachable, unknown
/// Extended Status field id…).
pub async fn init(dir: &Path) -> Result<AppState, Failure> {
    config::ensure_dir(dir).map_err(|e| Failure::Other(msg(&e)))?;
    let path = dir.join(config::CONFIG_FILE);
    if !path.exists() {
        return Err(Failure::Setup);
    }
    let env = dir.join(config::ENV_FILE);
    let cfg = config::Config::load(dir).map_err(|e| {
        // a token problem only if the file itself is fine, so a parse error isn't hidden behind it
        let file_ok = std::fs::read_to_string(&path).is_ok_and(|raw| config::Config::parse(&raw).is_ok());
        if file_ok && !config::token_from_env() && config::read_token(&env).is_err() { Failure::Token(msg(&e)) } else { Failure::Other(msg(&e)) }
    })?;
    let jira = jira::Jira::new(&cfg).map_err(|e| Failure::Other(msg(&e)))?;
    let me = match jira.get("/rest/api/2/myself").await {
        Ok(me) => me,
        Err(e) => {
            let rejected = e.downcast_ref::<jira::JiraError>().is_some_and(|j| matches!(j.status.as_u16(), 401 | 403));
            let e = msg(&e.context("checking JIRA_TOKEN against Jira"));
            return Err(if rejected && !config::token_from_env() { Failure::Token(e) } else { Failure::Other(e) });
        }
    };
    log::info!("connected to {} as {}", jira.base(), me["displayName"].as_str().unwrap_or("?"));
    let names = jira::field_names(&jira).await.map_err(|e| Failure::Other(msg(&e)))?;
    let ext_field_name = names
        .get(&cfg.fields.extended_status)
        .cloned()
        .with_context(|| format!("field {} (extended_status) not found in Jira", cfg.fields.extended_status))
        .map_err(|e| Failure::Other(msg(&e)))?;
    let icon_dir = cfg.icon_dir.clone();
    Ok(AppState { cfg, jira, ext_field_name, icon_dir, cache: Default::default(), icons: Default::default() })
}

/// Where startup stands.
pub enum Phase {
    /// [`init`] is running.
    Starting,
    /// The dashboard can be served.
    Ready(Arc<AppState>),
    /// [`init`] failed.
    Failed(Failure),
}

/// The config folder and the current [`Phase`], shared by every command.
pub struct Holder {
    /// Config folder (`jiraf.toml`, `.env`, icons, log).
    pub dir: PathBuf,
    /// Current phase; swapped as a whole on Retry / Reload config.
    pub phase: RwLock<Phase>,
    /// Bumped by each [`Holder::restart`], so a slow, older [`init`] can't overwrite a newer result.
    generation: AtomicU64,
}

/// What the page needs to know about startup (see [`Holder::status`]).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `"starting"`, `"ready"`, `"failed"`, or `"setup"` (no `jiraf.toml` yet).
    pub state: &'static str,
    /// The startup error, when `failed`.
    pub error: Option<String>,
    /// Whether the failure is the `.env` token, so the error screen offers to paste a new one.
    pub token_problem: bool,
    /// Full path of the config folder, shown on the error screen.
    pub config_dir: String,
}

impl Holder {
    /// A holder for the config folder `dir`, in [`Phase::Starting`].
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, phase: RwLock::new(Phase::Starting), generation: AtomicU64::new(0) }
    }

    /// The app state, if startup succeeded.
    ///
    /// # Errors
    /// `"jiraf is not ready"` while starting or after a failure.
    pub fn ready(&self) -> Result<Arc<AppState>, ApiError> {
        match &*self.phase.read().unwrap() {
            Phase::Ready(st) => Ok(st.clone()),
            _ => Err(api::bad("jiraf is not ready")),
        }
    }

    /// Store the result of the [`init`] started at generation `generation` (from [`Holder::restart`]):
    /// [`Phase::Ready`] or [`Phase::Failed`] (logged). Ignored if a newer run has started since.
    pub fn set(&self, generation: u64, r: Result<AppState, Failure>) {
        let phase = match r {
            Ok(st) => Phase::Ready(Arc::new(st)),
            Err(f) => {
                match &f {
                    Failure::Setup => log::info!("no jiraf.toml yet: opening Settings"),
                    Failure::Token(m) | Failure::Other(m) => log::error!("startup failed: {m}"),
                }
                Phase::Failed(f)
            }
        };
        let mut cur = self.phase.write().unwrap();
        // checked under the write lock, so a restart() can't slip in between check and store
        if self.generation.load(Ordering::SeqCst) == generation {
            *cur = phase;
        } else {
            log::info!("dropping the result of an older startup run");
        }
    }

    /// Mark startup as running again (first start, Retry, Reload config) and return this run's generation
    /// for [`Holder::set`].
    pub fn restart(&self) -> u64 {
        let mut cur = self.phase.write().unwrap();
        *cur = Phase::Starting;
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The current [`Status`].
    pub fn status(&self) -> Status {
        let (state, error, token_problem) = match &*self.phase.read().unwrap() {
            Phase::Starting => ("starting", None, false),
            Phase::Ready(_) => ("ready", None, false),
            Phase::Failed(Failure::Setup) => ("setup", None, false),
            Phase::Failed(Failure::Token(e)) => ("failed", Some(e.clone()), true),
            Phase::Failed(Failure::Other(e)) => ("failed", Some(e.clone()), false),
        };
        Status { state, error, token_problem, config_dir: self.dir.display().to_string() }
    }

    /// Run [`init`] again (Retry, Reload config, after a Settings save) and return the new [`Status`].
    pub async fn reload(&self) -> Status {
        let generation = self.restart();
        let r = init(&self.dir).await;
        self.set(generation, r);
        self.status()
    }
}

/* ---------- commands ---------- */

/// Where startup stands, so the page knows whether to wait, load the dashboard or show the error.
#[tauri::command]
pub fn app_status(holder: tauri::State<'_, Holder>) -> Status {
    holder.status()
}

/// Run [`init`] again (after the config or token was fixed, or from Reload config) and return the
/// new [`Status`].
#[tauri::command]
pub async fn retry(holder: tauri::State<'_, Holder>) -> Result<Status, ()> {
    Ok(holder.reload().await)
}

/// Open the config folder in Finder (Explorer on Windows).
///
/// # Errors
/// If the system refuses to open it.
#[tauri::command]
pub fn open_config_dir(app: tauri::AppHandle, holder: tauri::State<'_, Holder>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(holder.dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// Show the config folder in Finder (Explorer on Windows) with `jiraf.toml` selected, for hand edits;
/// before the first save there is no file to select, so just open the folder.
///
/// # Errors
/// If the system refuses to show it.
#[tauri::command]
pub fn show_config_file(app: tauri::AppHandle, holder: tauri::State<'_, Holder>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = holder.dir.join(config::CONFIG_FILE);
    if path.exists() {
        app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string())
    } else {
        app.opener().open_path(holder.dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_ready_until_ready() {
        let h = Holder::new(PathBuf::from("/x"));
        let s = h.status();
        assert_eq!((s.state, s.error, s.config_dir.as_str()), ("starting", None, "/x"));
        assert!(matches!(h.ready(), Err(e) if e.to_string() == "jiraf is not ready"));
        h.set(h.restart(), Err(Failure::Other("boom".into())));
        let s = h.status();
        assert_eq!((s.state, s.error.as_deref()), ("failed", Some("boom")));
        assert!(matches!(h.ready(), Err(e) if e.to_string() == "jiraf is not ready"));
    }

    #[test]
    fn older_init_cannot_overwrite_newer() {
        let h = Holder::new(PathBuf::from("/x"));
        let first = h.restart();
        let second = h.restart();
        h.set(second, Err(Failure::Other("new".into())));
        h.set(first, Err(Failure::Other("old".into())));
        assert_eq!(h.status().error.as_deref(), Some("new"));
    }

    #[test]
    fn setup_and_token_failures_reach_the_page() {
        let h = Holder::new(PathBuf::from("/x"));
        h.set(h.restart(), Err(Failure::Setup));
        let s = h.status();
        assert_eq!((s.state, s.error, s.token_problem), ("setup", None, false));
        h.set(h.restart(), Err(Failure::Token("JIRA_TOKEN is empty".into())));
        let s = h.status();
        assert_eq!((s.state, s.error.as_deref(), s.token_problem), ("failed", Some("JIRA_TOKEN is empty"), true));
    }

    #[tokio::test]
    async fn missing_config_means_setup() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(init(d.path()).await, Err(Failure::Setup)));
    }

    #[test]
    fn csp_keeps_unsafe_inline_styles() {
        // Tauri adds a nonce to style-src unless told not to, and a nonce disables 'unsafe-inline',
        // which would drop every style="" attribute the page uses (column widths, tree indent…).
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let off = &conf["app"]["security"]["dangerousDisableAssetCspModification"];
        assert!(off.as_array().is_some_and(|a| a.iter().any(|d| d == "style-src")), "{off}");
    }
}
