use keyring::Entry;
use serde::Serialize;

const SERVICE: &str = "com.radar.dev";
const USER: &str = "anthropic";
// Clerk's `__clerk_db_jwt` persisted across app restarts so the Clerk session
// survives an app close → reopen. Third-party cookies on *.clerk.accounts.dev
// don't persist reliably under `tauri://localhost` (WebKit ITP drops them),
// so we own the persistence here and re-inject the JWT into the URL before
// Clerk boots. The JWT itself is long-lived (Clerk dev instances issue ~2mo
// dev-browser JWTs) and identifies the browser, not a specific session.
const CLERK_JWT_USER: &str = "clerk_db_jwt";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub present: bool,
    pub preview: Option<String>,
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, USER).map_err(|e| format!("keyring open: {e}"))
}

fn clerk_jwt_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, CLERK_JWT_USER).map_err(|e| format!("keyring open: {e}"))
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

#[tauri::command]
pub fn set_clerk_db_jwt(jwt: String) -> Result<(), String> {
    let trimmed = jwt.trim();
    if trimmed.is_empty() {
        return Err("jwt is empty".to_string());
    }
    clerk_jwt_entry()?
        .set_password(trimmed)
        .map_err(|e| format!("keyring write: {e}"))
}

#[tauri::command]
pub fn get_clerk_db_jwt() -> Result<Option<String>, String> {
    match clerk_jwt_entry()?.get_password() {
        Ok(jwt) => Ok(Some(jwt)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring read: {e}")),
    }
}

#[tauri::command]
pub fn clear_clerk_db_jwt() -> Result<(), String> {
    match clerk_jwt_entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete: {e}")),
    }
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
