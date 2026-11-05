//! What each full-list resource last synced, per account, persisted beside
//! the store as `sync-state.json`. It is a file of its own because
//! `radar.json` keeps exactly its established keys.
//!
//! At the start of a sync session the digests seed each resource's
//! divergence baseline ([`SyncedResource::seed_last_synced`]). A local value
//! that differs from its last-synced digest may be an edit that never
//! reached the server (the app quit before the push landed, crashed, lost
//! power), or one whose push landed without the app recording it (a quit
//! push answered after shutdown). The first pull tells these apart by
//! checking the server ([`SyncedResource::pull`]): it pushes the edit only
//! if the server still holds the last-synced value, adopts the server's copy
//! without a push if it already equals local, and otherwise keeps the
//! server's copy, since another device wrote since. Archives, whose offline
//! merge is a union, always push the union. Only the account that wrote the
//! digests uses them.
//!
//! The file holds two digest sets, both written by the save job right after
//! the store, from the same snapshot:
//! - `digests`: per resource, the [`key_digest`] of the value last known to
//!   be on the server ([`SyncedResource::last_synced_digest`]).
//! - `saved`: per resource, the [`key_digest`] of the value that save wrote
//!   to `radar.json` ([`snapshot_digests`]).
//!
//! A seed treats any local value that differs from `digests` as a possible
//! unpushed edit, and pushes it over the server's row (which every other
//! device then pulls) when the server still holds the last-synced value.
//! That is only right when the local value is the one the app saved
//! alongside those digests. So at hydration ([`LastSynced::checked`]) a
//! resource keeps its `digests` entry only when the store loaded cleanly and
//! its loaded value matches its `saved` entry. Otherwise the entry is dropped
//! and that resource's first pull takes the server's copy: a missing,
//! unparseable or truncated `radar.json`, one another writer replaced (an
//! older build after a downgrade), or a sidecar whose write failed after the
//! store's.
//! A future change to a resource's `stable_key` also fails the `saved`
//! comparison and falls back to the server's copy, so the file needs no
//! format version.
//!
//! [`SyncedResource::seed_last_synced`]: super::resource::SyncedResource::seed_last_synced
//! [`SyncedResource::pull`]: super::resource::SyncedResource::pull
//! [`SyncedResource::last_synced_digest`]: super::resource::SyncedResource::last_synced_digest

use super::resource::{key_digest, SyncResource};
use super::resources::{ArchivesRes, CategoriesRes, PrefsRes, ProfileRes, SourcesRes};
use crate::store::Snapshot;
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

/// The [`key_digest`] of each resource's stable key in `snap`: what a save
/// of `snap` wrote to `radar.json`, or what a load of it read.
pub fn snapshot_digests(snap: &Snapshot) -> Digests {
    fn digest<R: SyncResource>(local: &R::Local) -> Option<String> {
        Some(key_digest(&R::stable_key(local)))
    }
    Digests {
        profile: digest::<ProfileRes>(&snap.profile),
        categories: digest::<CategoriesRes>(&snap.categories),
        sources: digest::<SourcesRes>(&snap.sources),
        prefs: digest::<PrefsRes>(&snap.prefs),
        archives: digest::<ArchivesRes>(&snap.archives),
    }
}

/// The file's contents: whose digests they are, what was last synced, and
/// what was saved with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSynced {
    pub account_id: String,
    /// What each resource last synced.
    #[serde(default)]
    pub digests: Digests,
    /// What the save that wrote this file wrote to `radar.json`
    /// ([`snapshot_digests`]). The writer sets it from the snapshot it
    /// writes. A file without it (written before it existed) seeds nothing.
    #[serde(default)]
    pub saved: Digests,
}

impl LastSynced {
    /// Keeps a resource's last-synced digest only when the store loaded
    /// cleanly (`store_clean`) and the value it loaded (`loaded`, the
    /// [`snapshot_digests`] of the loaded snapshot) is the one saved with
    /// the digest. Any other local value would look like an unpushed edit
    /// and be pushed over the server's row; with its digest dropped, that
    /// resource's first pull takes the server's copy instead. Call it once,
    /// at hydration, before anything edits the loaded values.
    pub fn checked(mut self, loaded: &Digests, store_clean: bool) -> LastSynced {
        let fields = [
            (
                "profile",
                &mut self.digests.profile,
                &self.saved.profile,
                &loaded.profile,
            ),
            (
                "categories",
                &mut self.digests.categories,
                &self.saved.categories,
                &loaded.categories,
            ),
            (
                "sources",
                &mut self.digests.sources,
                &self.saved.sources,
                &loaded.sources,
            ),
            (
                "prefs",
                &mut self.digests.prefs,
                &self.saved.prefs,
                &loaded.prefs,
            ),
            (
                "archives",
                &mut self.digests.archives,
                &self.saved.archives,
                &loaded.archives,
            ),
        ];
        for (name, digest, saved, loaded) in fields {
            if digest.is_none() {
                continue;
            }
            let reason = if !store_clean {
                "the store did not load cleanly"
            } else if saved.is_none() {
                "no saved digest"
            } else if saved != loaded {
                "the loaded value differs from the saved one"
            } else {
                continue;
            };
            log::warn!("[sync] not seeding {name} from the last sync: {reason}");
            *digest = None;
        }
        self
    }

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
            saved: Digests::default(),
        }
    }

    fn all(d: &str) -> Digests {
        Digests {
            profile: Some(format!("{d}-profile")),
            categories: Some(format!("{d}-categories")),
            sources: Some(format!("{d}-sources")),
            prefs: Some(format!("{d}-prefs")),
            archives: Some(format!("{d}-archives")),
        }
    }

    #[test]
    fn roundtrips_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_beside(&dir.path().join("radar.json"));
        assert_eq!(path, dir.path().join("sync-state.json"));
        assert_eq!(load(&path), None);

        let written = LastSynced {
            saved: Digests {
                profile: Some("s1".into()),
                ..Default::default()
            },
            ..state("user_1", "d1")
        };
        save(&path, Some(&written)).unwrap();
        assert_eq!(load(&path), Some(written));
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            raw,
            serde_json::json!({
                "accountId": "user_1",
                "digests": {"profile": "d1"},
                "saved": {"profile": "s1"}
            })
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

    #[test]
    fn file_without_saved_digests_still_parses() {
        // The shape b98db2b wrote: no `saved`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"accountId": "user_1", "digests": {"profile": "d1"}}"#,
        )
        .unwrap();
        assert_eq!(load(&path), Some(state("user_1", "d1")));
    }

    #[test]
    fn checked_keeps_only_resources_loaded_as_saved() {
        let synced = LastSynced {
            account_id: "u".into(),
            digests: all("synced"),
            saved: all("saved"),
        };
        let loaded = Digests {
            sources: Some("edited elsewhere".into()),
            archives: None,
            ..all("saved")
        };
        let kept = synced.clone().checked(&loaded, true);
        assert_eq!(
            kept.digests,
            Digests {
                sources: None,
                archives: None,
                ..all("synced")
            }
        );
        assert_eq!(kept.account_id, "u");
        assert_eq!(kept.saved, synced.saved);

        let partial = LastSynced {
            saved: Digests {
                prefs: None,
                ..all("saved")
            },
            ..synced
        };
        let kept = partial.checked(&all("saved"), true);
        assert_eq!(
            kept.digests,
            Digests {
                prefs: None,
                ..all("synced")
            },
            "a resource without a saved digest seeds nothing"
        );
    }

    #[test]
    fn checked_drops_everything_when_the_store_did_not_load_cleanly() {
        let synced = LastSynced {
            account_id: "u".into(),
            digests: all("synced"),
            saved: all("saved"),
        };
        let kept = synced.checked(&all("saved"), false);
        assert_eq!(kept.digests, Digests::default());
    }

    #[test]
    fn checked_drops_everything_without_saved_digests() {
        // A file b98db2b wrote.
        let old = LastSynced {
            account_id: "u".into(),
            digests: all("synced"),
            saved: Digests::default(),
        };
        let kept = old.checked(&all("saved"), true);
        assert_eq!(kept.digests, Digests::default());
    }

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy_store.json")
    }

    /// What a save wrote is what the next load reads: `saved` matches the
    /// loaded digests, and every resource can seed.
    fn assert_round_trips(snap: &Snapshot) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let saved = snapshot_digests(snap);
        for (name, d) in [
            ("profile", &saved.profile),
            ("categories", &saved.categories),
            ("sources", &saved.sources),
            ("prefs", &saved.prefs),
            ("archives", &saved.archives),
        ] {
            assert!(d.is_some(), "no digest for {name}");
        }
        crate::store::save(&path, snap).unwrap();
        let loaded = crate::store::load_checked(&path);
        assert!(loaded.clean);
        let read = snapshot_digests(&loaded.snap);
        assert_eq!(read, saved);
        let state = LastSynced {
            account_id: "u".into(),
            digests: all("synced"),
            saved,
        };
        assert_eq!(state.checked(&read, loaded.clean).digests, all("synced"));
    }

    #[test]
    fn legacy_store_digests_round_trip() {
        assert_round_trips(&crate::store::load(&fixture()));
    }

    #[test]
    fn populated_store_digests_round_trip() {
        use crate::types::{
            Accent, Archive, Article, Category, Density, Prefs, Profile, Source, SourceError,
            SourceHealth, SummaryModel, Theme, View, ViewMode,
        };
        let article = |id: u64, image: bool, read: bool| Article {
            id,
            title: format!("Article {id}"),
            source: "Dieline".into(),
            source_id: 1,
            category: "packaging".into(),
            category_label: "Packaging".into(),
            summary: "A studio \"rethinks\" the carton — é ✓".into(),
            published: "2h ago".into(),
            published_at: image.then(|| "2026-01-01T10:00:00.123Z".parse().unwrap()),
            read,
            bookmarked: !read,
            accent: "camel".into(),
            url: format!("https://x.test/{id}"),
            image_url: image.then(|| format!("https://x.test/{id}.png")),
            summarized_by: None,
        };
        let source = |id: u32, category: &str, last: Option<i64>, health| Source {
            id,
            name: format!("Source {id}"),
            category: category.into(),
            feed_url: last.map(|_| format!("https://x.test/{id}/feed")),
            enabled: last.is_some(),
            is_default: id == 1,
            last_fetch_at: last,
            health,
        };
        let ids = [
            13_612_357_342_556_406_000,
            15_651_859_121_617_930_000,
            9_007_199_254_740_993_000,
        ];
        let snap = Snapshot {
            prefs: Prefs {
                theme: Theme::Dark,
                view_mode: ViewMode::List,
                density: Density::Compact,
                accent: Accent::Ink,
                use_serif: false,
                show_accent: false,
                sidebar_collapsed: true,
                view: View::Settings,
                run_at_launch: true,
                schedule_enabled: true,
                schedule_time: "06:45".into(),
                summary_model: SummaryModel::Sonnet55,
                stale_after_days: 14,
                unread_only: true,
            },
            sources: vec![
                source(1, "packaging", Some(1_767_261_000_000), SourceHealth::Ok),
                source(2, "branding", None, SourceHealth::Warn),
                source(3, "gone-id", Some(1_767_000_000_000), SourceHealth::Stale),
            ],
            articles: vec![article(ids[0], true, false), article(ids[1], false, true)],
            archives: vec![
                Archive {
                    id: 1_767_261_000_000,
                    run_at: "2026-01-01T09:30:00.000Z".into(),
                    articles: ids
                        .iter()
                        .map(|&id| article(id, id % 2 == 0, false))
                        .collect(),
                    errors: vec![SourceError {
                        source_id: 2,
                        source_name: "Source 2".into(),
                        message: "timeout".into(),
                    }],
                },
                Archive {
                    id: 1_767_174_600_000,
                    run_at: "2025-12-31T09:30:00Z".into(),
                    articles: vec![article(ids[2], true, true), article(ids[0], false, false)],
                    errors: vec![],
                },
            ],
            categories: vec![
                Category {
                    id: "packaging".into(),
                    label: "Packaging".into(),
                    description: "Packaging design and dielines.".into(),
                    accent: "camel".into(),
                },
                Category {
                    id: "branding".into(),
                    label: "Branding".into(),
                    description: "Identity and rebrands.".into(),
                    accent: "olive".into(),
                },
            ],
            profile: Profile {
                name: "Stefan".into(),
                role: "Art director".into(),
                lens: "I run a small packaging studio.".into(),
            },
            sync_queue: vec![serde_json::json!({"groupKey": "PUT:/sync/prefs", "tries": 1})],
        };
        assert_round_trips(&snap);
    }
}
