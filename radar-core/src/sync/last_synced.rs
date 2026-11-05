//! What each full-list resource last synced, per account, persisted beside
//! the store as `sync-state.json`. It is a file of its own because
//! `radar.json` keeps exactly the keys the Tauri app wrote.
//!
//! At the start of a sync session the digests seed each resource's
//! divergence baseline ([`SyncedResource::seed_last_synced`]). An edit that
//! never reached the server (the app quit before the push landed, crashed,
//! lost power) is then pushed by the first pull instead of being replaced by
//! the older server copy. Only the account that wrote the digests uses them.
//!
//! The app writes this file right after the store, from the same snapshot:
//! digests newer than the stored values would make a stale local copy look
//! like an edit, and push it over the server's.
//!
//! [`SyncedResource::seed_last_synced`]: super::resource::SyncedResource::seed_last_synced

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "sync-state.json";

/// `sync-state.json` in the store's directory.
pub fn path_beside(store_path: &Path) -> PathBuf {
    store_path.with_file_name(FILE_NAME)
}

/// One [`last_synced_digest`] per resource; `None` when not known.
///
/// [`last_synced_digest`]: super::resource::SyncedResource::last_synced_digest
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Digests {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archives: Option<String>,
}

impl Digests {
    /// Takes every digest `newer` knows; keeps the rest. Returns whether
    /// anything changed.
    pub fn update(&mut self, newer: Digests) -> bool {
        let before = self.clone();
        let fields = [
            (&mut self.profile, newer.profile),
            (&mut self.categories, newer.categories),
            (&mut self.sources, newer.sources),
            (&mut self.prefs, newer.prefs),
            (&mut self.archives, newer.archives),
        ];
        for (mine, theirs) in fields {
            if theirs.is_some() {
                *mine = theirs;
            }
        }
        *self != before
    }
}

/// The file's contents: whose digests they are, and the digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSynced {
    pub account_id: String,
    #[serde(default)]
    pub digests: Digests,
}

impl LastSynced {
    /// The digests a session of `account_id` may seed from: none when they
    /// were written for another account.
    pub fn seed_for(state: Option<LastSynced>, account_id: &str) -> Digests {
        state
            .filter(|s| s.account_id == account_id)
            .map(|s| s.digests)
            .unwrap_or_default()
    }
}

/// The persisted state; `None` when the file is missing or unreadable
/// (logged), which leaves the first pull to take the server's copy.
pub fn load(path: &Path) -> Option<LastSynced> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!("[sync] can't read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(e) => {
            log::warn!("[sync] ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

/// Writes `state` atomically; `None` removes the file (signed out).
pub fn save(path: &Path, state: Option<&LastSynced>) -> io::Result<()> {
    match state {
        Some(state) => {
            let body = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
            crate::store::write_atomic(path, &body)
        }
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(account: &str, profile: &str) -> LastSynced {
        LastSynced {
            account_id: account.into(),
            digests: Digests {
                profile: Some(profile.into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn roundtrips_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_beside(&dir.path().join("radar.json"));
        assert_eq!(path, dir.path().join("sync-state.json"));
        assert_eq!(load(&path), None);

        save(&path, Some(&state("user_1", "d1"))).unwrap();
        assert_eq!(load(&path), Some(state("user_1", "d1")));
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            raw,
            serde_json::json!({"accountId": "user_1", "digests": {"profile": "d1"}})
        );
        assert!(!dir.path().join("sync-state.json.tmp").exists());

        save(&path, None).unwrap();
        assert!(!path.exists());
        save(&path, None).unwrap();
    }

    #[test]
    fn unreadable_file_is_no_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn another_account_never_seeds() {
        let seed = LastSynced::seed_for(Some(state("user_a", "d1")), "user_a");
        assert_eq!(seed.profile.as_deref(), Some("d1"));
        assert_eq!(
            LastSynced::seed_for(Some(state("user_a", "d1")), "user_b"),
            Digests::default()
        );
        assert_eq!(LastSynced::seed_for(None, "user_a"), Digests::default());
    }

    #[test]
    fn update_keeps_digests_the_newer_set_lacks() {
        let mut d = state("u", "p1").digests;
        assert!(!d.update(Digests::default()));
        assert!(d.update(Digests {
            sources: Some("s1".into()),
            ..Default::default()
        }));
        assert_eq!(d.profile.as_deref(), Some("p1"));
        assert_eq!(d.sources.as_deref(), Some("s1"));
        assert!(!d.update(d.clone()));
    }
}
