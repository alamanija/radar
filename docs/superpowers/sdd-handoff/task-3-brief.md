### Task 3: `RadarModel`, the pure state and mutations

**Files:**
- Create: `radar-core/src/model.rs`, `radar-core/src/text.rs`, `radar-core/src/search.rs`

**Interfaces:**
- Consumes: `types::*`, `store::Snapshot`.
- Produces:
  - `model::RadarModel { prefs, profile, sources, categories, articles, archives, briefing_errors: Vec<SourceError>, scanning: bool, active_category: String /*"all"*/, article_states: HashMap<String, ArticleState> }`.
  - `ArticleState { read: bool, bookmarked: bool }`.
  - Methods:
    - `from_snapshot(Snapshot) -> Self`, `to_snapshot(&self, sync_queue) -> Snapshot`
    - `add_source(name, category, feed_url: Option<String>) -> u32` (id = max+1, enabled, not default, health Ok, `last_fetch_at` None)
    - `update_source(id, SourcePatch)`
    - `delete_source(id) -> Result<(), &'static str>` (refuses `is_default`)
    - `toggle_source(id)`
    - `add_category(label, description, accent) -> String`, which slugifies like `App.jsx:670-678`
    - `update_category(id, CategoryPatch)`
    - `delete_category(id) -> Result<(), usize>` (Err = count of sources still using it)
    - `toggle_read(id: u64) -> Option<(String, ArticleState)>` and `toggle_bookmark(id: u64) -> Option<(String, ArticleState)>`, returning the PATCH payload
    - `begin_briefing()` (scanning=true, clears articles and errors)
    - `finish_briefing(resp: BriefingResponse, attempted: &HashSet<u32>, now_ms: i64) -> BriefingOutcome { new_count, total, source_count }`: overlays `article_states`, archives with cap 90 when non-empty, stamps health (warn for failed ids except 0, ok + `now_ms` for the rest), and sets scanning=false
    - `fail_briefing(msg)`
    - `briefing_inputs(&self) -> (Vec<SourceInput>, Vec<CategoryInput>, Option<String>, HashSet<u32>)`
    - `unread_count()`, `saved()`, `last_run_at_ms()`, `display_name(account: Option<&Account>)` (port of the `profileName` expression at `App.jsx:734`)
    - `apply_article_states(&mut self)`
  - `text::{greeting(name: &str, now: DateTime<Local>) -> String, relative_time_ms(ms: Option<i64>, now_ms: i64) -> String, tray_tooltip(unread, last_run_ms, articles, now_ms) -> String, tray_title(unread) -> Option<String>, truncate(s, max) -> String}`. Ports of `src/time.js` and `src/tray.js`.
  - `search::{search(query, &articles, &archives, &sources) -> Vec<Hit>}`, with `Hit = Article(Article) | Source(Source)`, articles deduped by id with current state winning.

- [ ] **Step 1: Write failing tests.** Each one mirrors the JS logic it replaces:
  - `greeting_buckets_and_daily_seed`: 07:00 on 2026-11-04 with name "Maya" gives `"<earlyMorn phrase>, Maya."`, where the phrase is `GREETINGS.earlyMorn[20261104 % 3]`. An empty name gives `"<phrase>."`.
  - `relative_time_ms` cases: None → "never", 30 s → "just now", 5 min → "5m ago", 3 h → "3h ago", 10 d → "1w ago".
  - `tray_tooltip_format`: `"Radar — 2 unread · last run 5m ago\n\nDezeen — Title…"` with three lines, each ≤ 72 chars. The zero-unread case reads "all caught up".
  - `slugify_dedupes`: "Packaging" twice gives "packaging" then "packaging-2". "!!!" gives "category".
  - `delete_category_blocked_when_in_use`.
  - `delete_default_source_refused`.
  - `finish_briefing_stamps_health`: a source in errors gets Warn with `last_fetch_at` unchanged. The synthetic `source_id 0` is ignored. Attempted sources get Ok + now.
  - `finish_briefing_overlays_states_and_archives`: a pre-existing state marks the fresh article read, and archives grow by 1. An empty response doesn't archive.
  - `archive_cap_90`.
  - `search_dedups_current_wins`: the same id exists in archives (unread) and current (read), and the result is read.
- [ ] **Step 2: Run `cargo test -p radar-core model text search`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** No IO, no clocks: time is always passed in.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): RadarModel state + mutations"`.

