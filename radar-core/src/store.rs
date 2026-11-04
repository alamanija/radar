//! JSON store compatible with the `radar.json` file written by Tauri's
//! `@tauri-apps/plugin-store`: a single top-level object whose keys are the
//! store keys and whose values are exactly what the JS frontend stored.

use crate::types::{Archive, Article, Category, Prefs, Profile, Source};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

const APP_DIR: &str = "com.radar.dev";
const FILE_NAME: &str = "radar.json";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub prefs: Prefs,
    pub sources: Vec<Source>,
    pub articles: Vec<Article>,
    pub archives: Vec<Archive>,
    pub categories: Vec<Category>,
    pub profile: Profile,
    /// Offline sync queue items, kept opaque (JS shape) for round-tripping.
    pub sync_queue: Vec<Value>,
}

/// `dirs::data_dir()/com.radar.dev/radar.json`.
pub fn default_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(FILE_NAME)
}

/// Deserialize one key independently; a missing or malformed key yields the
/// default (malformed ones are logged) so one bad key never loses the rest.
fn key<T: DeserializeOwned + Default>(map: &Map<String, Value>, name: &str) -> T {
    match map.get(name) {
        None | Some(Value::Null) => T::default(),
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            log::warn!("[store] key '{name}' is malformed, using default: {e}");
            T::default()
        }),
    }
}

/// Read the file into a top-level map. A missing file is `None`; an
/// unparseable one is backed up to `radar.json.corrupt-<ts>` and is `None`.
fn read_map(path: &Path) -> Option<Map<String, Value>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("[store] cannot read {}: {e}", path.display());
            }
            return None;
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(m)) => Some(m),
        _ => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let mut name = path.as_os_str().to_owned();
            name.push(format!(".corrupt-{ts}"));
            let backup = PathBuf::from(name);
            match std::fs::rename(path, &backup) {
                Ok(()) => log::warn!("[store] corrupt store moved to {}", backup.display()),
                Err(e) => log::warn!("[store] corrupt store, backup failed: {e}"),
            }
            None
        }
    }
}

pub fn load(path: &Path) -> Snapshot {
    let Some(map) = read_map(path) else {
        return Snapshot::default();
    };
    let categories: Vec<Category> = key(&map, "categories");
    let mut sources: Vec<Source> = key(&map, "sources");

    // Migrate legacy label-based source.category ("Packaging") to ids.
    // Orphaned values are left as-is (ingest treats them as uncategorized).
    for s in &mut sources {
        if categories.iter().any(|c| c.id == s.category) {
            continue;
        }
        if let Some(c) = categories.iter().find(|c| c.label == s.category) {
            s.category = c.id.clone();
        }
    }

    Snapshot {
        prefs: key(&map, "prefs"),
        sources,
        articles: key(&map, "articles"),
        archives: key(&map, "archives"),
        categories,
        profile: key(&map, "profile"),
        sync_queue: key(&map, "sync-queue"),
    }
}

pub fn save(path: &Path, snap: &Snapshot) -> io::Result<()> {
    // Re-read so unknown keys written by other versions survive. A corrupt
    // existing file is simply replaced.
    let mut map = std::fs::read_to_string(path)
        .ok()
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();

    fn val<T: serde::Serialize>(v: &T) -> io::Result<Value> {
        serde_json::to_value(v).map_err(io::Error::other)
    }
    map.insert("version".into(), Value::from(SCHEMA_VERSION));
    map.insert("prefs".into(), val(&snap.prefs)?);
    map.insert("sources".into(), val(&snap.sources)?);
    map.insert("articles".into(), val(&snap.articles)?);
    map.insert("archives".into(), val(&snap.archives)?);
    map.insert("categories".into(), val(&snap.categories)?);
    map.insert("profile".into(), val(&snap.profile)?);
    map.insert("sync-queue".into(), Value::Array(snap.sync_queue.clone()));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let body = serde_json::to_vec_pretty(&Value::Object(map)).map_err(io::Error::other)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tauri_radar.json")
    }

    #[test]
    fn loads_tauri_era_file() {
        let s = load(&fixture());
        assert_eq!(s.sources.len(), 2);
        assert_eq!(s.articles.len(), 2);
        assert_eq!(s.archives.len(), 1);
        assert_eq!(s.categories.len(), 2);
        assert_eq!(s.sync_queue.len(), 2);
        assert_eq!(s.profile.name, "Stefan");
        assert_eq!(s.prefs.schedule_time, "08:15");
        assert!(s.prefs.schedule_enabled);
        assert_eq!(s.archives[0].run_at, "2026-01-01T09:30:00.000Z");
        assert_eq!(s.sync_queue[0]["groupKey"], "PUT /sync/prefs");
    }

    #[test]
    fn migrates_legacy_category_labels() {
        let s = load(&fixture());
        assert_eq!(s.sources[0].category, "packaging");
        assert_eq!(s.sources[1].category, "branding");

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("radar.json");
        fs::write(
            &p,
            json!({
                "categories": [{"id":"packaging","label":"Packaging","description":"","accent":"camel"}],
                "sources": [
                    {"id":1,"name":"A","category":"Packaging","feedUrl":null,"enabled":true,"isDefault":false,"lastFetchAt":null},
                    {"id":2,"name":"B","category":"gone-id","feedUrl":null,"enabled":true,"isDefault":false,"lastFetchAt":null}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.sources[0].category, "packaging");
        assert_eq!(s.sources[1].category, "gone-id");
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = load(&dir.path().join("nope.json"));
        assert_eq!(s, Snapshot::default());
    }

    #[test]
    fn corrupt_file_backs_up_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("radar.json");
        fs::write(&p, "{not json").unwrap();
        let s = load(&p);
        assert_eq!(s, Snapshot::default());
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("radar.json.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "{not json");
    }

    #[test]
    fn malformed_key_only_defaults_that_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("radar.json");
        fs::write(&p, json!({"sources": "oops", "profile": {"name": "Z"}}).to_string()).unwrap();
        let s = load(&p);
        assert!(s.sources.is_empty());
        assert_eq!(s.profile.name, "Z");
    }

    #[test]
    fn save_is_atomic_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("radar.json");
        let snap = load(&fixture());
        save(&p, &snap).unwrap();
        assert!(!dir.path().join("sub").join("radar.json.tmp").exists());
        assert_eq!(load(&p), snap);
        let raw: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(raw["version"], SCHEMA_VERSION);
        assert_eq!(raw["sync-queue"].as_array().unwrap().len(), 2);
        assert_eq!(raw["prefs"]["viewMode"], "list");
    }

    #[test]
    fn save_preserves_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("radar.json");
        fs::write(&p, json!({"future-key": {"a": 1}, "profile": {"name": "Old"}}).to_string()).unwrap();
        let mut snap = load(&p);
        snap.profile.name = "New".into();
        save(&p, &snap).unwrap();
        let raw: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(raw["future-key"], json!({"a": 1}));
        assert_eq!(raw["profile"]["name"], "New");
    }

    #[test]
    fn empty_arrays_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("radar.json");
        fs::write(&p, json!({"sources": [], "categories": []}).to_string()).unwrap();
        let s = load(&p);
        assert!(s.sources.is_empty());
        assert!(s.categories.is_empty());
    }

    #[test]
    fn default_path_ends_with_app_dir() {
        let p = default_path();
        assert!(p.ends_with("com.radar.dev/radar.json"));
    }
}
