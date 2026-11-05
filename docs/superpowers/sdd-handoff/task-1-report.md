# Task 1 report: radar-core

Status: DONE_WITH_CONCERNS. Commit 9f54ba0 "feat(core): extract radar-core from src-tauri".

## Implemented
- Root Cargo.toml: members = ["radar-core", "radar-server"]; `version = "0.2.22"` added to [workspace.package].
- radar-core crate (Cargo.toml with workspace inheritance; deps copied from src-tauri minus tauri, plus dirs, log, tokio rt-multi-thread).
- lib.rs: modules + `runtime() -> &'static Runtime` (LazyLock, multi-thread, enable_all).
- types.rs: Article, SourceError, BriefingResponse (Serialize+Deserialize+Clone+PartialEq), Source/SourceHealth, Category, Archive, Prefs (+ Theme/ViewMode/Density/Accent/View enums), Profile. Wire formats as specified (lowercase enums, capitalized Accent, camelCase fields).
- schedule.rs: `parse_time`, `should_fire` (pure).
- Moved via git mv (history kept as renames): ingest, summarize, keychain, schedule_wake. All tauri imports/attributes removed; `ingest_briefing` wrapper deleted (`run_briefing` kept); `*_clerk_db_jwt` functions and constants deleted. ingest.rs uses `crate::types::{Article, BriefingResponse, SourceError}`. Behavior constants unchanged.
- Small clippy-driven changes: `enhance` returns `Box<(Article, String)>` in Err (result_large_err); `#[allow(clippy::needless_return)]` on sync_schedule_wake (cfg-gated blocks); `PathBuf` import cfg-gated to macos/linux (unused on Windows).

## TDD evidence
- RED: `cargo test -p radar-core` with crate files present but not yet in workspace members -> `error: package ID specification 'radar-core' did not match any packages` (expected: crate missing from workspace).
- GREEN: `cargo test -p radar-core` -> 13 passed; 0 failed (types x4, schedule x5, ingest x4).
- `cargo clippy -p radar-core --all-targets` -> no warnings.

## Files
Cargo.toml, Cargo.lock, radar-core/{Cargo.toml, src/{lib,types,schedule,ingest,summarize,keychain,schedule_wake}.rs}. src-tauri/src files moved out (src-tauri/src/lib.rs, scheduler.rs, tray.rs left untouched; they now reference missing modules but src-tauri is out of the workspace).

## Self-review / concerns
- `dirs` dependency is added per brief but not yet used (schedule_wake still uses env vars for home); no warning for unused deps.
- schedule_wake macOS/Linux paths are not compiled on Windows, so unverified here (copied verbatim).
- src-tauri no longer builds from its own dir referencing moved modules (intended, reference-only).
- The ingest/schedule tests use only pure functions; run_briefing network path is untested (unchanged behavior).
- Did not run `cargo check -p radar-server` after workspace change.
