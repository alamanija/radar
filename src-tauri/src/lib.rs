mod ingest;
mod keychain;
mod schedule_plist;
mod scheduler;
mod summarize;
mod tray;

use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// CLI flag the autostart plugin passes when the app is launched by the OS
/// at login. We detect it and start with the window hidden to tray.
const AUTOSTART_FLAG: &str = "--autostart";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_FLAG]),
        ))
        // Updater + process: fetch signed releases from the configured
        // endpoint and, after a verified download, restart the app into the
        // new version via tauri_plugin_process::init()'s `restart` command.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Tray icon + menu. Failure here is fatal because a missing tray
            // icon would mean the user has no way to re-open the window
            // after closing it (see on_window_event below, which hides
            // instead of quitting).
            tray::init(app.handle())?;

            // Hide-to-tray on window close: prevent the default close,
            // hide the window instead. Cmd+Q / the tray's "Quit" still
            // exits normally (they call `app.exit(0)`, which doesn't go
            // through CloseRequested).
            let main = app
                .get_webview_window("main")
                .expect("tauri.conf.json must define a window with label `main`");

            // If the OS launched us at login, start hidden-to-tray so we're
            // quietly present rather than popping a window at boot. The user
            // can click the tray icon to bring Radar forward.
            if std::env::args().any(|a| a == AUTOSTART_FLAG) {
                let _ = main.hide();
            }

            // Kick off the daily-briefing scheduler. One in-process tokio
            // loop replaces the old JS setTimeout path in App.jsx, reading
            // sources/categories/prefs from the Tauri store directly.
            scheduler::spawn(app.handle().clone());

            let window = main.clone();
            let app_handle = app.handle().clone();
            main.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                    // Rebuild tray menu so the Show/Hide label follows the
                    // window's real state after close-to-tray.
                    tray::on_window_hidden(&app_handle);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ingest::ingest_briefing,
            keychain::set_anthropic_api_key,
            keychain::clear_anthropic_api_key,
            keychain::anthropic_api_key_status,
            keychain::set_clerk_db_jwt,
            keychain::get_clerk_db_jwt,
            keychain::clear_clerk_db_jwt,
            tray::set_tray_status,
            tray::set_tray_articles,
            schedule_plist::sync_schedule_plist,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // macOS: fired when the user clicks a notification banner, the
            // dock icon, or otherwise activates Radar while the window is
            // hidden. Surface the main window so they don't land on a
            // silent no-op.
            if let RunEvent::Reopen { has_visible_windows, .. } = event {
                if !has_visible_windows {
                    tray::surface_main_window(app);
                }
            }
        });
}
