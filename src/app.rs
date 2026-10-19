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
        Self { dir, phase: RwLock::new(Phase::Starting) }
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

    /// Store the result of [`init`]: [`Phase::Ready`] or [`Phase::Failed`] (logged).
    pub fn set(&self, r: anyhow::Result<AppState>) {
        let phase = match r {
            Ok(st) => Phase::Ready(Arc::new(st)),
            Err(e) => {
                let msg = format!("{e:#}");
                log::error!("startup failed: {msg}");
                Phase::Failed(msg)
            }
        };
        *self.phase.write().unwrap() = phase;
    }

    /// Mark startup as running again (before a Retry / Reload config).
    pub fn restart(&self) {
        *self.phase.write().unwrap() = Phase::Starting;
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
    holder.restart();
    let r = init(&holder.dir).await;
    holder.set(r);
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
        h.set(Err(anyhow::anyhow!("boom")));
        let s = h.status();
        assert_eq!((s.state, s.error.as_deref()), ("failed", Some("boom")));
        assert!(matches!(h.ready(), Err(e) if e.to_string() == "jiraf is not ready"));
    }
}
