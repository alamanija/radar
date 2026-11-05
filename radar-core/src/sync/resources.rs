//! The five full-list synced resources (profile, categories, sources, prefs,
//! archives): their bodies, merges and keys.
//!
//! Remote snapshots are parsed leniently: an item that doesn't fit the local
//! type is skipped, and a missing or non-array list falls back as noted per
//! resource, so a bad server row never panics or wipes local data.

use super::resource::SyncResource;
use crate::types::{Archive, Category, Prefs, Profile, Source, SourceHealth};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const ARCHIVE_CAP: usize = 90;

pub struct ProfileRes;
pub struct CategoriesRes;
pub struct SourcesRes;
pub struct PrefsRes;
pub struct ArchivesRes;

/// Items of `v[key]` that parse as `T`; `None` when it isn't an array.
fn items<T: DeserializeOwned>(v: &Value, key: &str) -> Option<Vec<T>> {
    let arr = v.get(key)?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| match serde_json::from_value(x.clone()) {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!("[sync] skipping malformed {key} item: {e}");
                    None
                }
            })
            .collect(),
    )
}

fn to_key<T: Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

// ---- profile ---------------------------------------------------------------

fn non_empty(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        Value::String(s.to_string())
    }
}

impl SyncResource for ProfileRes {
    const ENDPOINT: &'static str = "/sync/profile";
    type Local = Profile;

    /// Empty strings travel as null.
    fn push_body(p: &Profile) -> Value {
        json!({ "name": non_empty(&p.name), "role": non_empty(&p.role), "lens": non_empty(&p.lens) })
    }

    /// Server wins; null → "".
    fn from_remote(r: &Value, _prev: &Profile) -> Profile {
        let s = |k: &str| {
            r.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Profile {
            name: s("name"),
            role: s("role"),
            lens: s("lens"),
        }
    }

    fn stable_key(p: &Profile) -> String {
        to_key(p)
    }

    fn is_empty(p: &Profile) -> bool {
        p.name.is_empty() && p.role.is_empty() && p.lens.is_empty()
    }
}

// ---- categories ------------------------------------------------------------

impl SyncResource for CategoriesRes {
    const ENDPOINT: &'static str = "/sync/categories";
    type Local = Vec<Category>;

    fn push_body(c: &Vec<Category>) -> Value {
        json!({ "categories": c })
    }

    /// Full-list replace; a snapshot without a list keeps local.
    fn from_remote(r: &Value, prev: &Vec<Category>) -> Vec<Category> {
        items(r, "categories").unwrap_or_else(|| prev.clone())
    }

    fn stable_key(c: &Vec<Category>) -> String {
        to_key(c)
    }

    fn is_empty(c: &Vec<Category>) -> bool {
        c.is_empty()
    }
}

// ---- sources ---------------------------------------------------------------

/// Only user-editable fields travel; `lastFetchAt` and `health` are
/// per-device observations.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceWire {
    id: u32,
    name: String,
    category: String,
    #[serde(default)]
    feed_url: Option<String>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    is_default: bool,
}

impl From<&Source> for SourceWire {
    fn from(s: &Source) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            category: s.category.clone(),
            feed_url: s.feed_url.clone(),
            enabled: s.enabled,
            is_default: s.is_default,
        }
    }
}

fn wire(xs: &[Source]) -> Vec<SourceWire> {
    xs.iter().map(SourceWire::from).collect()
}

impl SyncResource for SourcesRes {
    const ENDPOINT: &'static str = "/sync/sources";
    type Local = Vec<Source>;

    fn push_body(xs: &Vec<Source>) -> Value {
        json!({ "sources": wire(xs) })
    }

    /// Server list, with this device's `lastFetchAt`/`health` carried over by
    /// id (new sources start `ok`, never fetched). No list keeps local.
    fn from_remote(r: &Value, prev: &Vec<Source>) -> Vec<Source> {
        let Some(remote) = items::<SourceWire>(r, "sources") else {
            return prev.clone();
        };
        let seen: HashMap<u32, &Source> = prev.iter().map(|s| (s.id, s)).collect();
        remote
            .into_iter()
            .map(|w| {
                let local = seen.get(&w.id);
                Source {
                    id: w.id,
                    name: w.name,
                    category: w.category,
                    feed_url: w.feed_url,
                    enabled: w.enabled,
                    is_default: w.is_default,
                    last_fetch_at: local.and_then(|s| s.last_fetch_at),
                    health: local.map(|s| s.health).unwrap_or(SourceHealth::Ok),
                }
            })
            .collect()
    }

    /// Wire shape only, so a briefing bumping `lastFetchAt` isn't a push.
    fn stable_key(xs: &Vec<Source>) -> String {
        to_key(&wire(xs))
    }

    fn is_empty(xs: &Vec<Source>) -> bool {
        xs.is_empty()
    }
}

// ---- prefs -----------------------------------------------------------------

/// The synced prefs subset. `sidebarCollapsed` and `view` stay per-device.
fn synced_prefs(p: &Prefs) -> Value {
    json!({
        "theme": p.theme,
        "viewMode": p.view_mode,
        "runAtLaunch": p.run_at_launch,
        "scheduleEnabled": p.schedule_enabled,
        "scheduleTime": p.schedule_time,
        "density": p.density,
        "accent": p.accent,
        "useSerif": p.use_serif,
        "showAccent": p.show_accent,
    })
}

impl SyncResource for PrefsRes {
    const ENDPOINT: &'static str = "/sync/prefs";
    type Local = Prefs;

    fn push_body(p: &Prefs) -> Value {
        json!({ "prefs": synced_prefs(p) })
    }

    /// The nine synced keys come from the server, each falling back to its
    /// default when missing or invalid; per-device keys keep the local value.
    fn from_remote(r: &Value, prev: &Prefs) -> Prefs {
        let remote = match r.get("prefs") {
            Some(v @ Value::Object(_)) => v.clone(),
            _ => json!({}),
        };
        let p: Prefs = serde_json::from_value(remote).unwrap_or_default();
        Prefs {
            sidebar_collapsed: prev.sidebar_collapsed,
            view: prev.view,
            ..p
        }
    }

    fn stable_key(p: &Prefs) -> String {
        to_key(&synced_prefs(p))
    }

    /// Never empty: once signed in the server always gets a prefs row.
    fn is_empty(_: &Prefs) -> bool {
        false
    }
}

// ---- archives --------------------------------------------------------------

fn run_at_ms(a: &Archive) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&a.run_at)
        .ok()
        .map(|t| t.timestamp_millis())
}

impl SyncResource for ArchivesRes {
    const ENDPOINT: &'static str = "/sync/archives";
    type Local = Vec<Archive>;

    fn push_body(xs: &Vec<Archive>) -> Value {
        json!({ "archives": xs })
    }

    /// Union of local and remote (remote wins on id; archives are immutable),
    /// newest `runAt` first, capped at [`ARCHIVE_CAP`]. Unparseable `runAt`
    /// sorts last.
    fn from_remote(r: &Value, prev: &Vec<Archive>) -> Vec<Archive> {
        let remote: Vec<Archive> = items(r, "archives").unwrap_or_default();
        let mut out: Vec<Archive> = Vec::with_capacity(prev.len() + remote.len());
        let mut index: HashMap<i64, usize> = HashMap::new();
        for a in prev.iter().cloned().chain(remote) {
            match index.get(&a.id) {
                Some(&i) => out[i] = a,
                None => {
                    index.insert(a.id, out.len());
                    out.push(a);
                }
            }
        }
        out.sort_by_key(|a| std::cmp::Reverse(run_at_ms(a)));
        out.truncate(ARCHIVE_CAP);
        out
    }

    fn stable_key(xs: &Vec<Archive>) -> String {
        to_key(xs)
    }

    /// Offline briefings join the server's archives rather than replace them.
    fn merge_offline(r: &Value, local: &Vec<Archive>) -> Vec<Archive> {
        Self::from_remote(r, local)
    }

    /// The union keeps every server archive, so an offline edit seeded from
    /// an earlier run is pushed whatever the server holds now.
    const OFFLINE_MERGE_UNIONS: bool = true;

    fn is_empty(xs: &Vec<Archive>) -> bool {
        xs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Accent, Density, SourceHealth, Theme, View, ViewMode};
    use serde_json::json;

    fn src(id: u32, last: Option<i64>, health: SourceHealth) -> Source {
        Source {
            id,
            name: format!("S{id}"),
            category: "tools".into(),
            feed_url: None,
            enabled: true,
            is_default: false,
            last_fetch_at: last,
            health,
        }
    }

    fn archive(id: i64, run_at: &str) -> Archive {
        Archive {
            id,
            run_at: run_at.into(),
            articles: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn profile_body_nulls_empty_fields() {
        let p = Profile {
            name: "Ada".into(),
            role: String::new(),
            lens: "L".into(),
        };
        assert_eq!(
            ProfileRes::push_body(&p),
            json!({"name": "Ada", "role": null, "lens": "L"})
        );
        assert!(ProfileRes::is_empty(&Profile::default()));
        assert!(!ProfileRes::is_empty(&p));
        let back =
            ProfileRes::from_remote(&json!({"name": null, "role": "R", "updatedAt": "t"}), &p);
        assert_eq!(
            back,
            Profile {
                name: String::new(),
                role: "R".into(),
                lens: String::new()
            }
        );
    }

    #[test]
    fn categories_roundtrip() {
        let cats = vec![Category {
            id: "tools".into(),
            label: "Tools".into(),
            description: "d".into(),
            accent: "olive".into(),
        }];
        let body = CategoriesRes::push_body(&cats);
        assert_eq!(body["categories"][0]["label"], "Tools");
        assert_eq!(CategoriesRes::from_remote(&body, &vec![]), cats);
        assert!(CategoriesRes::is_empty(&vec![]));
    }

    #[test]
    fn sources_wire_and_health_preserved_by_id() {
        let prev = vec![
            src(1, Some(5), SourceHealth::Warn),
            src(2, Some(7), SourceHealth::Stale),
        ];
        let body = SourcesRes::push_body(&prev);
        assert_eq!(
            body["sources"][0],
            json!({"id": 1, "name": "S1", "category": "tools", "feedUrl": null,
                   "enabled": true, "isDefault": false})
        );
        let remote = json!({"sources": [
            {"id": 2, "name": "Two", "category": "c", "feedUrl": "https://f", "enabled": false, "isDefault": true},
            {"id": 9, "name": "New", "category": "c", "enabled": true, "isDefault": false}
        ], "updatedAt": "t"});
        let got = SourcesRes::from_remote(&remote, &prev);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "Two");
        assert_eq!(got[0].feed_url.as_deref(), Some("https://f"));
        assert_eq!(got[0].last_fetch_at, Some(7));
        assert_eq!(got[0].health, SourceHealth::Stale);
        assert_eq!(got[1].last_fetch_at, None);
        assert_eq!(got[1].health, SourceHealth::Ok);
        assert_eq!(got[1].feed_url, None);
    }

    #[test]
    fn prefs_only_nine_keys_merged_over_defaults() {
        let local = Prefs {
            theme: Theme::Dark,
            sidebar_collapsed: true,
            view: View::Sources,
            ..Prefs::default()
        };
        let body = PrefsRes::push_body(&local);
        let keys: Vec<&String> = body["prefs"].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 9);
        assert!(body["prefs"].get("sidebarCollapsed").is_none());
        assert!(body["prefs"].get("view").is_none());
        assert_eq!(body["prefs"]["theme"], "dark");

        let merged = PrefsRes::from_remote(
            &json!({"prefs": {"viewMode": "list", "accent": "Rust", "density": "compact",
                              "sidebarCollapsed": false, "view": "settings"}, "updatedAt": "t"}),
            &local,
        );
        assert_eq!(merged.view_mode, ViewMode::List);
        assert_eq!(merged.accent, Accent::Rust);
        assert_eq!(merged.density, Density::Compact);
        // Missing synced key falls through to the default, not the local value.
        assert_eq!(merged.theme, Theme::Light);
        assert_eq!(merged.schedule_time, "07:30");
        // Per-device keys stay local.
        assert!(merged.sidebar_collapsed);
        assert_eq!(merged.view, View::Sources);
        // Changing a per-device key isn't a push.
        let mut moved = local.clone();
        moved.view = View::Archive;
        assert_eq!(PrefsRes::stable_key(&moved), PrefsRes::stable_key(&local));
        assert!(!PrefsRes::is_empty(&Prefs::default()));
    }

    #[test]
    fn archives_union_sorted_capped() {
        let prev = vec![
            archive(1, "2026-01-02T00:00:00Z"),
            archive(2, "2026-01-01T00:00:00Z"),
        ];
        let mut remote_list: Vec<Archive> = (100..195)
            .map(|i| archive(i, &format!("2025-06-{:02}T00:00:00Z", (i % 28) + 1)))
            .collect();
        let mut dup = archive(1, "2026-01-02T00:00:00Z");
        dup.errors = vec![];
        remote_list.push(dup);
        remote_list.push(archive(3, "2026-01-03T00:00:00Z"));
        let got =
            ArchivesRes::from_remote(&json!({"archives": remote_list, "updatedAt": "t"}), &prev);
        assert_eq!(got.len(), ARCHIVE_CAP);
        assert_eq!(got[0].id, 3);
        assert_eq!(got[1].id, 1);
        assert_eq!(got[2].id, 2);
        for w in got.windows(2) {
            assert!(w[0].run_at >= w[1].run_at);
        }
        // Non-array remote keeps local.
        assert_eq!(
            ArchivesRes::from_remote(&json!({"archives": null}), &prev),
            prev
        );
    }
}
