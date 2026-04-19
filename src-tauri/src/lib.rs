mod ingest;
mod keychain;
mod summarize;
mod tray;

use tauri::{Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
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
            let window = main.clone();
            main.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
