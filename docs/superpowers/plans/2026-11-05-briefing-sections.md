# Briefing Sections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the briefing into Top stories (Claude-ranked against the lens), New, a caught-up line, and a collapsed Earlier section.

**Architecture:** `radar-core` derives which live articles are new from the archive's run history, carries a Claude `importance` score (1–5) through summarization and the summary cache, and splits the filtered briefing into sections. `radar-app` renders those sections in grid and list mode and keeps the Earlier expand state per view.

**Tech Stack:** Rust workspace (`radar-core`, `radar-app` on GPUI Kit), serde, httpmock, `gpui_kit::test`.

**Spec:** `docs/superpowers/specs/2026-11-05-briefing-sections-design.md`

**Deviation from the spec (decided while planning):** the spec keeps `new_ids` as an in-memory set recomputed at hydration, on `finish_briefing` and after an archive pull. This plan derives it on demand (`RadarModel::new_ids()`), because app tests and sync paths assign `articles`/`archives` directly and a cached set would go stale on any path that forgets to refresh it. Cost: one pass over the archive ids per call.

## Global Constraints

- Toolchain: every cargo command runs as `cargo +1.99.1 …`.
- Never change the article id derivation (`ingest.rs::hash_id`).
- New stored fields are `Option` with `#[serde(default, skip_serializing_if = "Option::is_none")]`; stored files without them must round-trip unchanged.
- No new synced prefs keys and no server changes.
- `TOP_STORIES_MAX = 3`, `TOP_STORY_MIN_IMPORTANCE = 4`.
- Copy, exactly: "Top stories", "New · {n}", "You're caught up.", "Nothing new since your last briefing.", "Earlier · {m}", aria "Show earlier articles ({m})" / "Hide earlier articles", subtitle segment " · {n} new" (only when n > 0).
- Before each commit: `cargo +1.99.1 fmt --check` and `cargo +1.99.1 clippy --workspace --all-targets -- -D warnings` are clean. `platform::updater::tests::latest_json_shape` fails on this Windows machine (WSL `bash`, no `jq`); that failure alone is expected.

## Review Focus

- A second run the same morning with identical feeds: everything is Earlier, the page says "Nothing new since your last briefing." (Task 4 test).
- Another device's later run sits at `archives[0]`: this device's articles still count as new (Task 1 test).
- A category filter hides every new article while the run did bring new ones: "You're caught up.", not "Nothing new…" (Task 4 test).
- Claude returns an out-of-range importance (0 or 9): the summary is kept, the score dropped (Task 2 test).
- Articles summarized before this change (`importance: None`) never become top stories, and a top story opened under "Unread only" stays in place (Task 3 tests).

---

### Task 1: Derive the "new" set from the archive history

**Files:**
- Modify: `radar-core/src/model.rs` (new method; `finish_briefing`; remove `prev_ids` and its uses in `begin_briefing`/`fail_briefing`)
- Test: `radar-core/src/model.rs` (`mod tests`)

**Interfaces:**
- Produces: `pub fn new_ids(&self) -> HashSet<u64>` on `RadarModel`. `BriefingOutcome::new_count == self.new_ids().len()` after `finish_briefing`.

- [ ] **Step 1: Write the failing tests** (helper: `fn run(id: i64, run_at: &str, ids: &[u64]) -> Archive` building articles with `art(id, 1)`):

```rust
#[test]
fn new_ids_exclude_articles_seen_in_earlier_runs() {
    let mut m = model();
    m.articles = vec![art(1, 1), art(2, 1), art(3, 1)];
    m.archives = vec![
        run(2, "2026-11-05T07:30:00.000Z", &[1, 2, 3]),
        run(1, "2026-11-04T07:30:00.000Z", &[2]),
    ];
    assert_eq!(m.new_ids(), HashSet::from([1, 3]));
}

#[test]
fn a_later_run_from_another_device_does_not_hide_new_articles() {
    let mut m = model();
    m.articles = vec![art(1, 1), art(2, 1), art(3, 1)];
    m.archives = vec![
        run(3, "2026-11-05T12:00:00.000Z", &[9]),
        run(2, "2026-11-05T07:30:00.000Z", &[1, 2, 3]),
        run(1, "2026-11-04T07:30:00.000Z", &[2]),
    ];
    assert_eq!(m.new_ids(), HashSet::from([1, 3]));
}

#[test]
fn without_a_matching_run_all_but_the_newest_count_as_earlier() {
    let mut m = model();
    m.articles = vec![art(1, 1), art(2, 1)];
    m.archives = vec![
        run(2, "2026-11-05T07:30:00.000Z", &[5]),
        run(1, "2026-11-04T07:30:00.000Z", &[1]),
    ];
    assert_eq!(m.new_ids(), HashSet::from([2]));
    assert!(model().new_ids().is_empty(), "no articles, nothing new");
}

#[test]
fn finish_briefing_counts_new_from_the_run_history() {
    let mut m = model();
    m.archives = vec![run(1, "2026-11-04T07:30:00.000Z", &[1])];
    m.begin_briefing();
    let resp = BriefingResponse { articles: vec![art(1, 1), art(2, 1)], errors: vec![], fetched_at: Utc::now() };
    let out = m.finish_briefing(resp, &HashSet::from([1]), 1_794_300_000_000);
    assert_eq!(out.new_count, 1);
    assert_eq!(m.new_ids(), HashSet::from([2]));
}
```

- [ ] **Step 2: Run** `cargo +1.99.1 test -p radar-core --lib new_ids finish_briefing_counts` — expect compile failure (`no method named new_ids`).

- [ ] **Step 3: Implement `new_ids`** per spec §1: live run = first archive (newest first) whose id set equals the live id set; earlier = archives with `run_at` strictly older than the live run's (compare parsed RFC 3339 timestamps, as `last_run_at_ms` does); fallback earlier = `archives[1..]`; empty live articles → empty set. In `finish_briefing`, compute `new_count` as `self.new_ids().len()` after the archive insert and `self.articles = resp.articles`; delete `prev_ids`.

- [ ] **Step 4: Run** `cargo +1.99.1 test -p radar-core` — all pass, including `ingest::tests::legacy_state_overlays_after_a_briefing` (its fixture archive already holds both ids, so `new_count` stays 0; if not, the fixture's run is the earlier run and the assertion must still hold — do not edit the fixture).

- [ ] **Step 5: Commit** `feat(core): derive new articles from the run history`.

---

### Task 2: Importance score through summarization and the cache

**Files:**
- Modify: `radar-core/src/types.rs` (`Article.importance`), `radar-core/src/summarize.rs` (schema, prompt, `Summary`, parsing), `radar-core/src/ingest.rs` (`KnownSummary`, `split_known`, `enhance`), `radar-core/src/model.rs` (`known_summaries`)
- Modify (struct literals gain `importance: None`): every `Article { … }` literal that lists all fields (the same files that gained `summarized_by: None`: `ingest.rs`, `model.rs`, `search.rs`, `sync/article_states.rs`, `sync/last_synced.rs`, `text.rs`, `radar-app/src/components/card.rs`, `radar-app/src/shell_tests.rs`), and every `KnownSummary { … }` literal in tests.
- Test: `types.rs`, `summarize.rs`, `ingest.rs`, `model.rs` test modules

**Interfaces:**
- Produces: `Article.importance: Option<u8>`; `Summary.importance: Option<u8>`; `KnownSummary.importance: Option<u8>`.

- [ ] **Step 1: Write the failing tests:**
  - `types.rs` `article_importance_is_optional_on_the_wire`: JSON without `importance` → `None` and re-serializes identically; `Some(4)` serializes as `"importance": 4`.
  - `summarize.rs` `body_requires_an_importance_score`: `request_body(...)["output_config"]["format"]["schema"]["properties"]["importance"]["enum"] == json!([1,2,3,4,5])` and `schema["required"]` contains `"importance"`.
  - `summarize.rs` `system_prompt_explains_importance`: prompt contains `"importance"` and `"5 = must-read"`.
  - `summarize.rs` `out_of_range_importance_keeps_the_summary`: mock returns text `{"summary":"S","category":"packaging","importance":9}` → `Ok`, `summary == "S"`, `importance == None`; with `4` → `Some(4)`.
  - `ingest.rs`: extend `known_summaries_are_reused_and_the_rest_go_to_claude` so the known entry has `importance: Some(5)` and the reused article ends with `importance == Some(5)`.
  - `model.rs`: extend `briefing_inputs_carry_model_age_limit_and_known_summaries` so a summarized article with `importance: Some(4)` yields `req.known[&id].importance == Some(4)`.

- [ ] **Step 2: Run** `cargo +1.99.1 test -p radar-core --lib` — expect compile failures on the missing fields.

- [ ] **Step 3: Implement.** Schema property `"importance": {"type": "integer", "enum": [1,2,3,4,5]}`, added to `required`. Prompt line (in the list after `category`): `- importance: integer 1-5, how much this reader would want to read it today. 5 = must-read, 3 = a typical item, 1 = skip. Judge against the Reader context when given; otherwise as a designer following the field.` `Parsed.importance: i64` (serde default 0 so a missing value parses); `Summary.importance = (1..=5).contains(&v).then_some(v as u8)`. `enhance` sets `article.importance`; `split_known` copies `k.importance`; `known_summaries` stores `a.importance`.

- [ ] **Step 4: Run** `cargo +1.99.1 test --workspace --no-fail-fast` — all pass (apart from the known updater test).

- [ ] **Step 5: Commit** `feat(core): score article importance against the lens`.

---

### Task 3: Split the briefing into sections

**Files:**
- Modify: `radar-core/src/model.rs`
- Test: `radar-core/src/model.rs`

**Interfaces:**
- Consumes: `new_ids()` (Task 1), `Article.importance` (Task 2), existing `briefing_articles(kept)`.
- Produces:
  ```rust
  pub const TOP_STORIES_MAX: usize = 3;
  pub const TOP_STORY_MIN_IMPORTANCE: u8 = 4;
  pub struct Sections<'a> { pub top: Vec<&'a Article>, pub new: Vec<&'a Article>, pub earlier: Vec<&'a Article> }
  pub fn briefing_sections(&self, kept: Option<&HashSet<u64>>) -> Sections<'_>
  ```

- [ ] **Step 1: Write the failing tests** (articles in live order; helper sets `importance` and `published_at`; archives built so ids 1–5 are new and 6–7 earlier):
  - `sections_lead_with_up_to_three_important_new_articles`: importances 1:Some(5), 2:Some(4), 3:Some(4), 4:Some(5), 5:Some(3), 6 (earlier):Some(5) → `top` ids `[1, 4, 2]` (score desc, then live order), `new` `[3, 5]`, `earlier` `[6, 7]`.
  - `unscored_and_low_scored_articles_are_never_top`: all new with `None` or `Some(3)` → `top` empty, all in `new`.
  - `sections_respect_category_and_unread_filters`: `active_category = "b"` keeps only `b` articles in every section; with `prefs.unread_only = true` a read top story is dropped, but with `kept` containing its id it stays in `top`.

- [ ] **Step 2: Run** `cargo +1.99.1 test -p radar-core --lib sections` — expect `no method named briefing_sections`.

- [ ] **Step 3: Implement** `briefing_sections` over `briefing_articles(kept)` and `new_ids()`: partition into new/earlier keeping live order; `top` = new articles with `importance >= TOP_STORY_MIN_IMPORTANCE`, stable-sorted by importance descending, truncated to `TOP_STORIES_MAX`; `new` = the other new articles in live order.

- [ ] **Step 4: Run** `cargo +1.99.1 test -p radar-core` — pass.

- [ ] **Step 5: Commit** `feat(core): split the briefing into top, new and earlier`.

---

### Task 4: Render the sections

**Files:**
- Modify: `radar-app/src/views/briefing.rs` (layout, `BriefingView.earlier_open`, subtitle), `radar-app/src/views/briefing_tests.rs`, `CLAUDE.md`
- Test: `radar-app/src/views/briefing_tests.rs`

**Interfaces:**
- Consumes: `briefing_sections(kept)`, `new_ids()` (for the subtitle count and the caught-up/nothing-new choice, which ignore filters).
- Produces: `BriefingView { kept, earlier_open: bool }` with `pub fn toggle_earlier(&mut self)`; `reset()` also collapses Earlier.

Element ids: section wrappers `briefing-section-top|new|earlier`; inside each, the grid is `briefing-{top|new|earlier}-grid`, the list `briefing-{…}-list`, the virtual list `briefing-{…}-vlist`; `briefing-caught-up` (label = its text); `briefing-earlier-toggle` (label "Show earlier articles (M)" / "Hide earlier articles", visible text "Earlier · M").

- [ ] **Step 1: Update and write the failing tests:**
  - Rewrite the view-mode and virtual-list tests (`briefing_tests.rs` ~lines 140–170) to the per-section ids: with no archives every article is new, so `briefing-new-grid` / `briefing-new-list` / `briefing-new-vlist`.
  - `sections_show_top_new_caught_up_and_collapsed_earlier`: archives make 1–3 new and 4–5 earlier; article 1 has `importance: Some(5)` → `card-1` inside `briefing-section-top`, `card-2` in `briefing-section-new`, `briefing-caught-up` label "You're caught up.", `briefing-earlier-toggle` label "Show earlier articles (2)", `card-4` absent; click the toggle → `card-4` present, label "Hide earlier articles"; set `prefs.view = View::Saved` then back to Briefing → `card-4` absent again.
  - `a_run_with_nothing_new_says_so`: live articles all present in an older archive → `briefing-caught-up` label "Nothing new since your last briefing.", no `briefing-section-new`.
  - `a_filter_hiding_all_new_articles_still_says_caught_up`: new articles only in category `a`, chip `chip-b` active → "You're caught up.".
  - `subtitle_counts_new_articles`: `three_articles` plus archives `[run at now−2h with ids {1,2,3}, run at now−1d with id {2}]` → "Your briefing surfaced 3 articles across 1 source · 2 new · 2 unread · updated 2h ago." With no archives at all every article is new; the existing `subtitle_follows_the_briefing_state` then expects "· 3 new" added to its two sentences.

- [ ] **Step 2: Run** `cargo +1.99.1 test -p radar-app --bin radar briefing` — expect the new tests to fail.

- [ ] **Step 3: Implement** in `briefing.rs`: render each non-empty section through one helper `section(id, heading, articles, grid, window, t, cx)` that reuses `grid_box` / `list` (give `list` an id prefix parameter); section headings 12 px, medium weight, `text_2`, 10 px below; the caught-up line 13 px in `text_3`; the toggle as a clickable row with a `ChevronRight`/`ChevronDown` icon and `cx.listener(|ws, _, _, cx| { ws.briefing.toggle_earlier(); cx.notify() })`. Subtitle gains " · {n} new" from `new_ids().len()`.

- [ ] **Step 4: Run** `cargo +1.99.1 test --workspace --no-fail-fast`, `cargo +1.99.1 fmt --check`, `cargo +1.99.1 clippy --workspace --all-targets -- -D warnings` — clean apart from the known updater test.

- [ ] **Step 5: Update `CLAUDE.md`** "Reading" note: sections (Top stories ≥ 4 importance, max 3; New; caught-up line; Earlier collapsed via `BriefingView.earlier_open`), "new" = not in any older run (`RadarModel::new_ids`, live run matched by id set), `importance` on `Article`/`KnownSummary`.

- [ ] **Step 6: Commit** `feat(app): briefing sections with top stories and collapsed earlier`.
