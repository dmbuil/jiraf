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
mod logging;
mod settings;

use std::path::PathBuf;
use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager};

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

/// The default app menu plus "Settings…" (⌘, / Ctrl+,) and "Reload config" (⌘⇧R / Ctrl+Shift+R) in its
/// first submenu.
///
/// # Errors
/// If the menu can't be built.
fn menu(app: &tauri::App) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app.handle())?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let reload = MenuItem::with_id(app, "reload-config", "Reload config", true, Some("CmdOrCtrl+Shift+R"))?;
    if let Some(first) = menu.items()?.first().and_then(|i| i.as_submenu()) {
        first.append(&settings)?;
        first.append(&reload)?;
    }
    Ok(menu)
}

/// Entry point: open the window at once and start [`app::init`] in the background.
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let dir = config_dir(app)?;
            logging::init(&dir)?;
            log::info!("jiraf {} starting; config folder: {}", env!("CARGO_PKG_VERSION"), dir.display());
            app.set_menu(menu(app)?)?;
            app.manage(app::Holder::new(dir.clone()));
            let handle = app.handle().clone();
            let generation = app.state::<app::Holder>().restart();
            tauri::async_runtime::spawn(async move {
                let r = app::init(&dir).await;
                handle.state::<app::Holder>().set(generation, r);
            });
            Ok(())
        })
        // Reload config: the page runs the same flow as its Retry button. Settings…: the page opens it.
        .on_menu_event(|app, event| {
            let name = match event.id().as_ref() {
                "reload-config" => "reload-config",
                "settings" => "open-settings",
                _ => return,
            };
            if let Err(e) = app.emit(name, ()) {
                log::warn!("{name}: {e}");
            }
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
            app::retry,
            app::open_config_dir,
            app::show_config_file,
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
            api::open_external,
            settings::settings,
            settings::save_settings,
            settings::save_token,
        ])
        .run(tauri::generate_context!())
        .expect("error while running jiraf");
}
