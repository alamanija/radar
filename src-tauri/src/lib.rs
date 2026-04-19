mod ingest;
mod keychain;
mod summarize;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        // Updater + process: fetch signed releases from the configured
        // endpoint and, after a verified download, restart the app into the
        // new version via tauri_plugin_process::init()'s `restart` command.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            ingest::ingest_briefing,
            keychain::set_anthropic_api_key,
            keychain::clear_anthropic_api_key,
            keychain::anthropic_api_key_status,
            keychain::set_clerk_db_jwt,
            keychain::get_clerk_db_jwt,
            keychain::clear_clerk_db_jwt,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
