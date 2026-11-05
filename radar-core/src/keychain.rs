//! Secret storage. `KeyStore` abstracts the OS keychain so callers (and
//! tests) can swap in `MemKeyStore`; `OsKeyStore` is the real backend.

use keyring::Entry;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "com.radar.dev";
/// Keychain username holding the Anthropic API key.
pub const ANTHROPIC_USER: &str = "anthropic";
/// Keychain username holding the Clerk OAuth `Tokens` (as JSON).
pub const CLERK_OAUTH_USER: &str = "clerk_oauth";
/// Where older builds kept Clerk's `__clerk_db_jwt`.
/// Nothing reads it any more; [`delete_legacy_entries`] removes it.
pub const LEGACY_CLERK_DB_JWT_USER: &str = "clerk_db_jwt";

/// A string secret store keyed by username. Missing entries are `Ok(None)`
/// on read and a successful no-op on delete.
pub trait KeyStore: Send + Sync {
    fn get(&self, user: &str) -> Result<Option<String>, String>;
    fn set(&self, user: &str, value: &str) -> Result<(), String>;
    fn delete(&self, user: &str) -> Result<(), String>;
}

/// The OS keychain (service `com.radar.dev`).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyStore;

impl OsKeyStore {
    fn entry(user: &str) -> Result<Entry, String> {
        Entry::new(SERVICE, user).map_err(|e| format!("keyring open: {e}"))
    }
}

impl KeyStore for OsKeyStore {
    fn get(&self, user: &str) -> Result<Option<String>, String> {
        match Self::entry(user)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring read: {e}")),
        }
    }

    fn set(&self, user: &str, value: &str) -> Result<(), String> {
        Self::entry(user)?
            .set_password(value)
            .map_err(|e| format!("keyring write: {e}"))
    }

    fn delete(&self, user: &str) -> Result<(), String> {
        // delete_credential returns NoEntry when nothing is stored — treat as success.
        match Self::entry(user)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keyring delete: {e}")),
        }
    }
}

/// In-memory store for tests (here and in downstream crates).
#[derive(Debug, Default)]
pub struct MemKeyStore {
    entries: Mutex<HashMap<String, String>>,
}

impl MemKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for MemKeyStore {
    fn get(&self, user: &str) -> Result<Option<String>, String> {
        Ok(self.entries.lock().unwrap().get(user).cloned())
    }

    fn set(&self, user: &str, value: &str) -> Result<(), String> {
        self.entries
            .lock()
            .unwrap()
            .insert(user.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, user: &str) -> Result<(), String> {
        self.entries.lock().unwrap().remove(user);
        Ok(())
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub present: bool,
    pub preview: Option<String>,
}

pub fn set_anthropic_api_key(store: &dyn KeyStore, key: String) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("key is empty".to_string());
    }
    store.set(ANTHROPIC_USER, trimmed)
}

pub fn clear_anthropic_api_key(store: &dyn KeyStore) -> Result<(), String> {
    store.delete(ANTHROPIC_USER)
}

pub fn anthropic_api_key_status(store: &dyn KeyStore) -> Result<ApiKeyStatus, String> {
    Ok(match store.get(ANTHROPIC_USER)? {
        Some(key) => ApiKeyStatus {
            present: true,
            preview: Some(preview(&key)),
        },
        None => ApiKeyStatus {
            present: false,
            preview: None,
        },
    })
}

pub fn read_anthropic_api_key(store: &dyn KeyStore) -> Option<String> {
    store.get(ANTHROPIC_USER).ok().flatten()
}

/// Convenience reader against the OS keychain (used by `ingest`).
pub fn read_api_key() -> Option<String> {
    read_anthropic_api_key(&OsKeyStore)
}

/// Removes what older builds left in the keychain and this app never
/// reads (`clerk_db_jwt`). Best effort: a missing entry is success, and a
/// failure is only logged (never the value). Blocks; run it off the UI
/// thread.
pub fn delete_legacy_entries(store: &dyn KeyStore) {
    match store.delete(LEGACY_CLERK_DB_JWT_USER) {
        Ok(()) => log::debug!("[keychain] no legacy {LEGACY_CLERK_DB_JWT_USER} entry left"),
        Err(e) => log::warn!("[keychain] removing the legacy entry failed: {e}"),
    }
}

fn preview(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 10 {
        return "••••".to_string();
    }
    let prefix: String = chars.iter().take(7).collect();
    let suffix: String = chars
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}••••••{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_store_roundtrip_and_missing() {
        let s = MemKeyStore::new();
        assert_eq!(s.get("x").unwrap(), None);
        s.delete("x").unwrap(); // deleting a missing entry is fine
        s.set("x", "v").unwrap();
        assert_eq!(s.get("x").unwrap().as_deref(), Some("v"));
        s.delete("x").unwrap();
        assert_eq!(s.get("x").unwrap(), None);
    }

    #[test]
    fn legacy_cleanup_removes_only_the_old_jwt() {
        let s = MemKeyStore::new();
        s.set(LEGACY_CLERK_DB_JWT_USER, "dvb_x").unwrap();
        s.set(ANTHROPIC_USER, "sk-ant-x").unwrap();
        s.set(CLERK_OAUTH_USER, "{}").unwrap();
        delete_legacy_entries(&s);
        assert_eq!(s.get(LEGACY_CLERK_DB_JWT_USER).unwrap(), None);
        assert!(s.get(ANTHROPIC_USER).unwrap().is_some());
        assert!(s.get(CLERK_OAUTH_USER).unwrap().is_some());
        // Already gone: still fine.
        delete_legacy_entries(&s);
    }

    #[test]
    fn anthropic_key_fns_use_store() {
        let s = MemKeyStore::new();
        assert_eq!(
            anthropic_api_key_status(&s).unwrap(),
            ApiKeyStatus {
                present: false,
                preview: None
            }
        );
        assert!(set_anthropic_api_key(&s, "   ".into()).is_err());
        set_anthropic_api_key(&s, "  sk-ant-api03-abcdefXYZ  ".into()).unwrap();
        assert_eq!(
            read_anthropic_api_key(&s).as_deref(),
            Some("sk-ant-api03-abcdefXYZ")
        );
        let st = anthropic_api_key_status(&s).unwrap();
        assert!(st.present);
        assert_eq!(st.preview.as_deref(), Some("sk-ant-••••••XYZ"));
        // Stored under the `anthropic` user, not mixed with OAuth tokens.
        assert!(s.get(ANTHROPIC_USER).unwrap().is_some());
        assert!(s.get(CLERK_OAUTH_USER).unwrap().is_none());
        clear_anthropic_api_key(&s).unwrap();
        assert_eq!(read_anthropic_api_key(&s), None);
    }
}
