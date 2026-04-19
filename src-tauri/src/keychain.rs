use keyring::Entry;
use serde::Serialize;

const SERVICE: &str = "com.radar.dev";
const USER: &str = "anthropic";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub present: bool,
    pub preview: Option<String>,
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, USER).map_err(|e| format!("keyring open: {e}"))
}

#[tauri::command]
pub fn set_anthropic_api_key(key: String) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("key is empty".to_string());
    }
    entry()?
        .set_password(trimmed)
        .map_err(|e| format!("keyring write: {e}"))
}

#[tauri::command]
pub fn clear_anthropic_api_key() -> Result<(), String> {
    // delete_credential returns NoEntry when nothing is stored — treat as success.
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete: {e}")),
    }
}

#[tauri::command]
pub fn anthropic_api_key_status() -> Result<ApiKeyStatus, String> {
    match entry()?.get_password() {
        Ok(key) => Ok(ApiKeyStatus {
            present: true,
            preview: Some(preview(&key)),
        }),
        Err(keyring::Error::NoEntry) => Ok(ApiKeyStatus {
            present: false,
            preview: None,
        }),
        Err(e) => Err(format!("keyring read: {e}")),
    }
}

pub fn read_api_key() -> Option<String> {
    entry().ok()?.get_password().ok()
}

fn preview(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 10 {
        return "••••".to_string();
    }
    let prefix: String = chars.iter().take(7).collect();
    let suffix: String = chars.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{prefix}••••••{suffix}")
}
