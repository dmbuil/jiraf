//! Startup and the app state holder.
//!
//! [`init`] runs every check `jiraf` needs before it can serve the dashboard (config, token, Jira,
//! field ids). Its result lives in a [`Holder`], so the page can show a startup error, and Retry or
//! Reload config can run [`init`] again without restarting the app.

use crate::api::{self, ApiError, AppState};
use crate::{config, jira};
use anyhow::Context;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Load the config folder `dir`, check the token against Jira and resolve the Extended Status field
/// name, then build the [`AppState`].
///
/// # Errors
/// On a bad config, a missing or rejected token, Jira being unreachable, or an unknown Extended Status
/// field id.
pub async fn init(dir: &Path) -> anyhow::Result<AppState> {
    config::ensure_dir(dir)?;
    let cfg = config::Config::load(dir)?;
    let jira = jira::Jira::new(&cfg)?;
    let me = jira.get("/rest/api/2/myself").await.context("checking JIRA_TOKEN against Jira")?;
    log::info!("connected to {} as {}", jira.base(), me["displayName"].as_str().unwrap_or("?"));
    let names = jira::field_names(&jira).await?;
    let ext_field_name = names
        .get(&cfg.fields.extended_status)
        .cloned()
        .with_context(|| format!("field {} (extended_status) not found in Jira", cfg.fields.extended_status))?;
    let icon_dir = cfg.icon_dir.clone();
    Ok(AppState { cfg, jira, ext_field_name, icon_dir, cache: Default::default(), icons: Default::default() })
}

/// Where startup stands.
pub enum Phase {
    /// [`init`] is running.
    Starting,
    /// The dashboard can be served.
    Ready(Arc<AppState>),
    /// [`init`] failed with this message.
    Failed(String),
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
    /// `"starting"`, `"ready"` or `"failed"`.
    pub state: &'static str,
    /// The startup error, when `failed`.
    pub error: Option<String>,
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
    pub fn set(&self, generation: u64, r: anyhow::Result<AppState>) {
        let phase = match r {
            Ok(st) => Phase::Ready(Arc::new(st)),
            Err(e) => {
                let msg = format!("{e:#}");
                log::error!("startup failed: {msg}");
                Phase::Failed(msg)
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
        let (state, error) = match &*self.phase.read().unwrap() {
            Phase::Starting => ("starting", None),
            Phase::Ready(_) => ("ready", None),
            Phase::Failed(e) => ("failed", Some(e.clone())),
        };
        Status { state, error, config_dir: self.dir.display().to_string() }
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
    let generation = holder.restart();
    let r = init(&holder.dir).await;
    holder.set(generation, r);
    Ok(holder.status())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_ready_until_ready() {
        let h = Holder::new(PathBuf::from("/x"));
        let s = h.status();
        assert_eq!((s.state, s.error, s.config_dir.as_str()), ("starting", None, "/x"));
        assert!(matches!(h.ready(), Err(e) if e.to_string() == "jiraf is not ready"));
        h.set(h.restart(), Err(anyhow::anyhow!("boom")));
        let s = h.status();
        assert_eq!((s.state, s.error.as_deref()), ("failed", Some("boom")));
        assert!(matches!(h.ready(), Err(e) if e.to_string() == "jiraf is not ready"));
    }

    #[test]
    fn older_init_cannot_overwrite_newer() {
        let h = Holder::new(PathBuf::from("/x"));
        let first = h.restart();
        let second = h.restart();
        h.set(second, Err(anyhow::anyhow!("new")));
        h.set(first, Err(anyhow::anyhow!("old")));
        assert_eq!(h.status().error.as_deref(), Some("new"));
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
