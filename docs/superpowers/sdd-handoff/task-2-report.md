# Task 2 report: Tauri-compatible JSON store

Status: DONE. Commit 0fa090e "feat(core): tauri-compatible JSON store".

## Implemented
- `radar-core/src/store.rs`: `SCHEMA_VERSION=1`, `Snapshot`, `default_path()`, `load()`, `save()`.
  - load: reads top-level map, deserializes each key independently (`key<T>`), malformed key -> default + `log::warn!`. Missing file -> defaults. Unparseable / non-object file -> renamed to `radar.json.corrupt-<ms>` and defaults. Legacy category-label migration ported from App.jsx:147-157 (id match kept; label match -> id; orphan left as is).
  - save: re-reads existing map (unknown keys preserved), overwrites version + 7 known keys (`sync-queue` kept as opaque Values), writes `radar.json.tmp` then `fs::rename`; creates parent dir.
- Ruling R9: `Prefs`/`Profile` fields use `deserialize_with` lenient helpers (`lenient`, `lenient_true`, `lenient_time` in types.rs): wrong type/unknown enum/null falls to that field's default only.
- Fixture `radar-core/tests/fixtures/tauri_radar.json` (8 keys, 2 sources incl. legacy "Packaging", 2 categories, 2 articles, 1 archive with RFC3339 runAt, full prefs, profile, 2 sync-queue items in JS shape).
- Cargo: `tempfile = "3"` dev-dependency; `pub mod store` in lib.rs.

## Tests
`cargo test -p radar-core`: 24 passed, 0 failed. `cargo clippy -p radar-core --all-targets`: no warnings.
New: loads_tauri_era_file, migrates_legacy_category_labels, missing_file_yields_defaults, corrupt_file_backs_up_and_defaults, save_is_atomic_and_roundtrips, save_preserves_unknown_keys, empty_arrays_are_kept, plus malformed_key_only_defaults_that_key, default_path_ends_with_app_dir, prefs_unknown_enum_falls_back_per_field, profile_null_fields_default.

## TDD evidence
- RED (types): `cargo test -p radar-core types` -> `profile_null_fields_default ... FAILED`, `prefs_unknown_enum_falls_back_per_field ... FAILED` (2 failed, 4 passed).
- RED (store): with `todo!()` stubs, `cargo test -p radar-core store` -> 9 failed, 0 passed.
- GREEN: after implementation all 24 pass.

## Self-review / concerns
- Snapshot default has empty `categories`; the JS seeds 6 starter categories when the key is absent. Seeding is left to the UI layer (not in brief). When absent, no label migration occurs (matches JS effective behavior only if seeds are applied before; flag for the app-layer task).
- Lenient helpers silently coerce bad fields on load; a save then writes the defaulted value back (intended).
- `save` is not a cross-process lock; concurrent writers (e.g. scheduler + UI) are last-writer-wins on whole file.
- Windows `fs::rename` over an existing file succeeds on modern std (MoveFileEx replace); verified by roundtrip test.

## Fix round 1 (I2, I3)
Changes in radar-core/src/store.rs:
- I2/R11: `read_map` now returns `Read::{Missing,Map,Unreadable(io::Error),Unparseable}`. `save`: Unreadable -> Err, nothing written; Unparseable -> `backup_copy(.., "corrupt")?` (Err if backup fails) then overwrite. `load` on Unreadable: copy-backup (log on failure) + defaults; Unparseable keeps the rename to `.corrupt-<ts>`.
- I3/R12: new `items<T>` deserializes sources/articles/archives/categories per item, skipping+warning bad ones; `key` (prefs/profile/sync-queue) still per-key default. Any drop sets a flag and `load` copies the original file to `radar.json.bak-<ts>` (no backup for clean files).
- Tests added: save_refuses_to_overwrite_unreadable_file, save_backs_up_unparseable_existing_file_before_overwrite, bad_item_skipped_others_kept, dropped_data_triggers_backup_copy. malformed_key_only_defaults_that_key still passes. (Tests were added together with the implementation, not observed RED first.)
- Verification: `cargo test -p radar-core store` -> 13 passed, 0 failed; `cargo clippy -p radar-core --all-targets` -> clean.
