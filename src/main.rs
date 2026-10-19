//! jiraf: a lean, stateless follow-up dashboard over Jira (Data Center) Plans, as a desktop app.
//!
//! Startup: resolve the config folder, then run [`app::init`] in the background (config, token,
//! Jira, field ids) while the window shows the page. The page talks to the [`api`] commands over
//! Tauri's IPC; no port is opened. All Jira traffic goes through [`jira`].

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod config;
mod jira;

use std::path::PathBuf;
use tauri::Manager;

/// The config folder: `$JIRAF_CONFIG_DIR` if set (handy with `cargo tauri dev`), else the app's
/// config folder (`~/Library/Application Support/com.dmbuil.jiraf/` on macOS).
///
/// # Errors
/// If the platform has no config folder.
fn config_dir(app: &tauri::App) -> tauri::Result<PathBuf> {
    match std::env::var_os("JIRAF_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(d) => Ok(PathBuf::from(d)),
        None => app.path().app_config_dir(),
    }
}

/// Entry point: open the window at once and start [`app::init`] in the background.
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = config_dir(app)?;
            log::info!("config folder: {}", dir.display());
            app.manage(app::Holder::new(dir.clone()));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let r = app::init(&dir).await;
                handle.state::<app::Holder>().set(r);
            });
            Ok(())
        })
        // Jira icons and avatars, fetched with the token and cached (see `api::icon_response`).
        .register_asynchronous_uri_scheme_protocol("jiraf", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let uri = req.uri().to_string();
            tauri::async_runtime::spawn(async move {
                responder.respond(api::icon_response(&app.state::<app::Holder>(), &uri).await);
            });
        })
        .invoke_handler(tauri::generate_handler![
            app::app_status,
            api::data,
            api::transitions,
            api::priorities,
            api::assignable,
            api::set_ext,
            api::set_priority,
            api::set_assignee,
            api::set_target,
            api::set_due,
            api::transition,
        ])
        .run(tauri::generate_context!())
        .expect("error while running jiraf");
}
