### Task 1: `radar-core` crate with moved backend modules

**Files:**
- Modify: `Cargo.toml` (members: `radar-core`, `radar-server`; keep `src-tauri` out of members from now on)
- Create: `radar-core/Cargo.toml`, `radar-core/src/lib.rs`, `radar-core/src/types.rs`
- Move: `src-tauri/src/{ingest,summarize,keychain,schedule_wake}.rs` → `radar-core/src/`
- Create: `radar-core/src/schedule.rs`

**Interfaces:**
- Produces:
  - `radar_core::runtime() -> &'static tokio::runtime::Runtime` (multi-thread, `LazyLock`).
  - `types::{Article, SourceError, BriefingResponse}`, unchanged fields, `Serialize + Deserialize + Clone + PartialEq`.
  - `types::Source { id: u32, name, category: String, feed_url: Option<String>, enabled: bool, is_default: bool, last_fetch_at: Option<i64>, health: SourceHealth }`. Here `SourceHealth` = `Ok|Warn|Stale`, serialized `"ok"|"warn"|"stale"`, `#[serde(default)]` → `Ok`.
  - `types::Category { id, label, description, accent }`.
  - `types::Archive { id: i64, run_at: String /*RFC3339*/, articles: Vec<Article>, errors: Vec<SourceError> }`.
  - `types::Prefs`: all 11 fields from `DEFAULT_PREFS` in `src/App.jsx:23-35`, `#[serde(default)]` per field with those defaults. `theme: Theme{Light,Dark}`, `view_mode: ViewMode{Grid,List}`, `density: Density{Cozy,Compact}`, `accent: Accent{Camel,Olive,Rust,Slate,Ink}`, `view: View{Briefing,Saved,Archive,Sources,Categories,Settings}`.
  - `types::Profile { name, role, lens }`.
  - `ingest::run_briefing(sources: Vec<SourceInput>, categories: Vec<CategoryInput>, lens: Option<String>) -> Result<BriefingResponse, String>`. Same signature, with the `#[tauri::command]` wrapper deleted.
  - `schedule::parse_time(&str) -> Option<(u32,u32)>`.
  - `schedule::should_fire(prefs: &Prefs, last_run_at: Option<DateTime<Local>>, now: DateTime<Local>) -> bool`.
  - `schedule_wake::sync_schedule_wake(enabled: bool, hour: Option<u32>, minute: Option<u32>) -> Result<(), String>`.
  - `keychain::{set_anthropic_api_key, clear_anthropic_api_key, anthropic_api_key_status, read_api_key}`.

- [ ] **Step 1: Write failing tests.**
  - `types::tests::article_roundtrips_camel_case`: deserializing a JSON article copied from a real store (keys `sourceId`, `categoryLabel`, `publishedAt`, `imageUrl`) and re-serializing gives equal JSON.
  - `types::tests::prefs_defaults_fill_missing`: `{}` gives `schedule_time == "07:30"`, `accent == Camel`, `use_serif == true`.
  - `types::tests::source_without_health_defaults_ok`.
  - `schedule::tests::should_fire_*`:
    - disabled → false;
    - before slot → false;
    - after slot with `last_run` earlier today but before the slot → true;
    - after slot with `last_run` after the slot → false;
    - bad time `"25:00"` → false.
  - `ingest::tests`:
    - `strip_html_and_truncate("<p>a  <b>b</b></p>", 280) == "a b"`;
    - truncation at 280 ends with `…`;
    - `first_img_src` skips `data:` and returns the next `src`;
    - `resolve` of an unknown id gives `("uncategorized","Uncategorized","ink")`.
- [ ] **Step 2: Run `cargo test -p radar-core`.** Expected: compile FAIL (crate missing).
- [ ] **Step 3: Create the crate and move the modules.** Delete every `tauri` import and attribute, and delete the `*_clerk_db_jwt` keychain functions. `ingest.rs` now uses `crate::types::{Article, …}` instead of defining them. Copy the dependencies from `src-tauri/Cargo.toml`, minus tauri, and add `dirs`, `tokio` with `rt-multi-thread`, and `log`.
- [ ] **Step 4: Run `cargo test -p radar-core`.** Expected: all PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): extract radar-core from src-tauri"`.

