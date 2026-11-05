### Task 2: JSON store compatible with Tauri's plugin-store file

**Files:**
- Create: `radar-core/src/store.rs`, `radar-core/tests/fixtures/tauri_radar.json` (hand-built to match plugin-store output: top-level object with all 8 keys, camelCase, including a legacy label-based `source.category`)

**Interfaces:**
- Consumes: `types::*`.
- Produces:
  - `store::default_path() -> PathBuf`, which is `dirs::data_dir()/com.radar.dev/radar.json`.
  - `store::Snapshot { prefs: Prefs, sources: Vec<Source>, articles: Vec<Article>, archives: Vec<Archive>, categories: Vec<Category>, profile: Profile, sync_queue: Vec<serde_json::Value> }`.
  - `store::load(path: &Path) -> Snapshot`.
  - `store::save(path: &Path, snap: &Snapshot) -> io::Result<()>`.
  - `store::SCHEMA_VERSION: u32 = 1`.

- [ ] **Step 1: Write failing tests.**
  - `loads_tauri_era_file`: the fixture loads, all collections have their fixture lengths, and `sync_queue.len() == 2`.
  - `migrates_legacy_category_labels`: a source with `category: "Packaging"` and a category `{id:"packaging",label:"Packaging"}` comes back with `category == "packaging"`. An orphan id is left as-is (port of `App.jsx:147-157`).
  - `missing_file_yields_defaults`.
  - `corrupt_file_backs_up_and_defaults`: the file `"{not json"` gives defaults, and `radar.json.corrupt-<ts>` exists.
  - `save_is_atomic_and_roundtrips`: the write goes to `radar.json.tmp`, gets renamed, and reloads equal.
  - `save_preserves_unknown_keys`: an unknown top-level key in the file survives a save.
  - `empty_arrays_are_kept`: persisted `sources: []` loads as empty rather than seeded.
- [ ] **Step 2: Run `cargo test -p radar-core store`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Load into `serde_json::Map`, deserialize each key independently, and fall back to the default when a single key is malformed. Log that key with `log::warn!` instead of failing the whole file. `save` re-reads the existing map, overwrites the known keys plus `version`, and writes to the tmp file followed by `fs::rename`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): tauri-compatible JSON store"`.

