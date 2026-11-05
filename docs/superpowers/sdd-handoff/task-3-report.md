# Task 3 report: RadarModel

Status: DONE (one minor concern below)

Commit: 9d14527 feat(core): RadarModel state + mutations

## Implemented
- radar-core/src/types.rs: `Account` (camelCase, per ruling R1).
- radar-core/src/model.rs: RadarModel, ArticleState, SourcePatch, CategoryPatch, BriefingOutcome and every method in the brief. File is 675 lines including ~250 lines of tests (impl ~420).
- radar-core/src/text.rs: greeting, relative_time_ms, truncate, tray_tooltip, tray_title.
- radar-core/src/search.rs: Hit, search (caps 12 articles / 8 sources, like the JS).
- lib.rs: registers the three modules.

Decisions: `new_count` compares against article ids captured in `begin_briefing` (a private `prev_ids`), because `begin_briefing` clears articles and the JS used the pre-clear list. Without a prior `begin_briefing` it uses the current articles. `from_snapshot` seeds `article_states` from persisted read/bookmarked flags. `active_category` is "all" after `from_snapshot`.

## TDD
- RED: `cargo test -p radar-core --lib` with test-only modules failed to compile: 31 errors (cannot find `greeting`, `relative_time_ms`, `search`, `tray_tooltip`, `ArticleState`, `SourcePatch`, ...).
- GREEN: `cargo test -p radar-core`: 45 passed, 0 failed.
- `cargo clippy -p radar-core --all-targets`: no warnings (after fixing 2 test-code `from_ref` lints).

## Concerns
- `cargo fmt -p radar-core` also reformatted existing files (ingest, store, schedule, schedule_wake, summarize, keychain, ...) and they went into this commit as whitespace-only changes.
