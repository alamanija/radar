# SDD ledger — plan: docs/superpowers/plans/2026-11-04-gpui-kit-migration.md

Branch: worktree-gpui-migration (worktree .claude/worktrees/gpui-migration). Branch base: 184288b. Plan commit: cc4415b.
Spec: none separate. Plan cites user decisions (full replacement; Clerk browser OAuth+PKCE) + CLAUDE.md + radar-handoff/. Rulings are provisional against that.
Env: toolchain installed this session (rustc 1.99.1 MSVC, VS 2022 BuildTools C++, CMake 4.3.3). Subagent shells need: export PATH="$HOME/.cargo/bin:/c/Program Files/CMake/bin:$PATH"

## Pre-flight scan

| Pair / Task | Produces → Consumes | Finding |
|---|---|---|
| T1 → T3 | types::{Source,Category,Article,Prefs,Profile}, ingest::{SourceInput,CategoryInput} → model | consistent |
| T3 → T4 | T3 `display_name(account: Option<&Account>)` uses Account, which T4 defines | ORDER CONFLICT → R1 |
| T1 → T4 | keychain fns (T1 signatures) → T4 refactors them onto `&dyn KeyStore` | consistent sequencing |
| T4 self | Session::load(cfg) listed, then Session::load(cfg, store) | self-inconsistent → R2 |
| T4 → T5 | T5 step 1 runs `cargo run -p radar-core --example sign_in`, which no task creates; also needs live Clerk dashboard | gap → R3 |
| T2 → T6 | Snapshot.sync_queue: Vec<Value> → QueuedReq deserialize (JS key names) | consistent |
| T3 → T6 | ArticleState → article_states | consistent |
| T4 → T6 | Session::access_token → SyncClient | consistent |
| T6 → T7/T12 | SyncClient/SyncedResource → AppModel.sync (T7 field), wired T12 | consistent |
| T7 self | "content arrives in Task 9" (stale numbering; shell is T8) | typo → R4 |
| T7 → T8..T14 | AppModel::update_model, theme::t, bridge::on_rt | consistent |
| T8 → T9/T14 | header "Run briefing" button needs a run path before T14 creates AppModel::run_briefing | gap → R5 |
| T8 → T11 | SourcesIntent (Workspace) → SourcesView | consistent; type lives in workspace.rs |
| T9 → T10 | page_frame, Card{read_only} → Saved/Archive | consistent |
| T9 test → T12 | card_toggle_read..._when_signed_in needs a SyncClient on AppModel before T12's sign-in wiring | resolvable: construct AppModel with SyncClient on httpmock in test; AppModel::toggle_read patches when sync is Some |
| T12 → T14/T15 | T12 uses platform::autostart/updater "stubs"; T14 lists autostart.rs as Create, T15 updater.rs as Create | file-ownership conflict → R6 |
| T15 self / T7 | T15 version source = [workspace.package] version, which doesn't exist; radar-app created in T7 | gap → R7 |
| T7,T8,T9,T12,T14,T15 | manual visual / OS checks (side-by-side visuals, tray clicks, reboot, Tauri→GPUI updater migration, live Clerk) | not executable by subagents → R8 |
| T1 self | tests vs code: consistent | ok |
| T2 self | ok | ok |
| T3 self | ok (time injected) | ok |
| T6 self | ok (sleep injectable) | ok |
| T13 self | ok | ok |
| T16 self | ok | ok |
| Review Focus | RF1→T2, RF2→T14, RF3→T4, RF4→T6, RF5→T9 tests all present | ok |

## Rulings

- R1 Ruling: `Account { id, email, name, image_url: Option<String> }` is defined in `radar-core/src/types.rs` by Task 3; Task 4's `auth` uses it (`pub use crate::types::Account`). — T3 needs it before T4 exists. — If wrong: a move of one struct.
- R2 Ruling: `Session::load(cfg: AuthConfig, store: Arc<dyn KeyStore>)` is the only signature. — the KeyStore line is the later, deliberate edit. — If wrong: trivial signature change.
- R3 Ruling: Task 5 implementer also creates `radar-core/examples/sign_in.rs` (prints decoded claims only, never tokens) and implements server changes against documented Clerk behavior: same JWKS, `iss` = CLERK_ISSUER or optional `CLERK_OAUTH_ISSUER`, optional `CLERK_OAUTH_CLIENT_ID` matched against `client_id`/`azp`. The live-claims check moves to the user's manual checklist. — subagents have no Clerk dashboard access. — If wrong: live OAuth tokens rejected until claims adjusted; surfaced at first manual e2e.
- R4 Ruling: Task 7 "content arrives in Task 9" means Task 8. — renumbering artifact. — If wrong: nothing.
- R5 Ruling: Task 8 creates the minimal real `AppModel::run_briefing(&mut self, cx)` (begin_briefing → on_rt(ingest::run_briefing) → finish_briefing / fail_briefing, no-op while scanning). Task 14 extends it with injection, notification, tray. — header button must work when shipped in T8. — If wrong: small refactor in T14.
- R6 Ruling: Task 12 creates `radar-app/src/platform/{mod,autostart,updater}.rs` as minimal stubs (autostart: is_enabled()->false, set()->Ok; updater: state Idle, check no-op). Task 14 and Task 15 *modify* them. — first consumer creates the file. — If wrong: nothing.
- R7 Ruling: Task 1 adds `version = "0.2.22"` under `[workspace.package]`; radar-core and radar-app use `version.workspace = true`. Task 15 documents the bump flow only. — keeps one version source from day one. — If wrong: one-line change.
- R8 Ruling: implementers do every automatable check (build, tests, launching the binary for a few seconds to confirm it starts and survives window close where scriptable). Visual side-by-side, tray/notification clicking, reboot/autostart, live Clerk claims, and the Tauri→GPUI updater migration go into a "Manual verification checklist" handed to the user at the end. — subagents can't see the screen or reboot. — If wrong: visual drift or platform bugs found later by the user rather than during the run.

## Model plan
Implementers: T1 sonnet, T2 sonnet, T3 sonnet, T4 opus, T5 sonnet, T6 opus, T7 opus, T8 opus, T9–T13 sonnet, T14 opus, T15 sonnet, T16 sonnet. Reviewers: sonnet, except T2/T4/T6/T7/T14 opus.

## Progress
Task 1: dispatched (BASE cc4415b, implementer sonnet)
Task 1: ⚠️ resolved by controller — radar-server `cargo check` passes post-workspace change; schedule_wake mac/linux branches moved verbatim (not compiled on Windows; accepted, covered by CI later)
Task 1: minor (deferred): types.rs Source/Category/Archive lack round-trip tests
Task 1: minor (deferred): schedule_wake.rs doc comments reference lib.rs/scheduler that no longer exist
Task 1: minor (deferred): dirs dep unused until Task 2
Task 1: complete (commits cc4415b..9f54ba0, review clean)
R9 Ruling: Task 2 makes Prefs/Profile deserialization lenient per field (unknown enum string or null → that field's default) so one bad value never wipes all prefs — raised by T1 review (minor) but load-bearing for RF1 store compatibility — If wrong: slightly more serde code.
Task 2: dispatched (BASE 9f54ba0, implementer sonnet)
Task 2: review 1 (opus): Needs fixes — I1 label migration vs seed, I2 unreadable file overwritten by save, I3 one bad item wipes collection (plan-mandated per-key)
R10 Ruling: T2-I1 rejected — current src/data.js seeds RADAR_DATA.categories = [], so JS effectiveCategories is [] when the key is missing; Rust matches current JS exactly — If wrong: very old label-based installs show sources as Uncategorized until the user re-picks a category.
R11 Ruling: T2-I2 accepted — save must never replace a file it could not read: non-NotFound read error in save → return Err; existing file that fails to parse → copy-backup to radar.json.corrupt-<ts> first, Err if backup fails. load on non-NotFound read error also attempts a copy-backup. — If wrong: app may refuse to save while the file is locked (surfaced as a logged error, not silent loss).
R12 Ruling: T2-I3 accepted, overriding plan's per-key-only default — Vec collections (sources, articles, archives, categories) deserialize item by item, skipping + log::warn! each bad item; whenever load drops any key or item it first copies the original file to radar.json.bak-<ts> so nothing is unrecoverable. — If wrong: extra backup files in the data dir.
Task 2: minor (deferred): BOM rejected by serde_json (strip U+FEFF); no sync_all before rename; leftover .tmp on failed write; backup name collision within same ms; roundtrip test's .tmp check weak; lenient field fallbacks not logged; unknown prefs/profile subfields dropped on save; keys written alphabetically (no preserve_order)
Task 2: fix round 1/5 (2 addressed, 0 open; commits 0fa090e..78ab9d6)
Task 2: minor (deferred): .bak/.corrupt backups unbounded (new .bak per load until a save cleans the data)
Task 2: complete (commits 9f54ba0..78ab9d6, review clean)
Task 3: dispatched (BASE 78ab9d6, implementer sonnet)
Task 3: ⚠️ resolved by controller — notification copy is Task 14's (notify::briefing_ready), not Task 3's
Task 3: minor (deferred): finish_briefing out-of-range now_ms → run_at "" via unwrap_or_default; from_snapshot seeds article_states from persisted flags (superset of JS; note for sync); truncate(s,0) returns "…"; test Article builder duplicated across model/search/text tests
Task 3: complete (commits 78ab9d6..9d14527, review clean)
Task 4: dispatched (BASE 9d14527, implementer opus)
Task 4: ⚠️ live Clerk behavior (denied-consent redirect, token formats) → manual checklist (R8/R3)
Task 4: minor (deferred): any /callback request ends the flow (check state before error; 400+keep waiting on mismatch) — local DoS only
Task 4: minor (deferred): code exchange can run 30s past sign-in timeout
Task 4: minor (deferred, CARRY TO TASK 12): Session::sign_in holds std tokens mutex during sync keychain write; persist() blocking keyring calls from async — use spawn_blocking / persist outside guard when the GPUI app calls it
Task 4: minor (deferred): no test for idle-connection / oversized-head loopback guarantee; unbounded connection count
Task 4: minor (deferred, controller-mandated): transient refresh failure in last 60s returns Err though token still valid — consider returning current token while expires_at > now
Task 4: minor (deferred): auth.rs ~590 code lines (>600 incl. loopback); loopback could move to auth/loopback.rs
Task 4: complete (commits 9d14527..8c32043, review clean)
R13 Ruling: Task 5 applies the CLERK_OAUTH_CLIENT_ID match only to OAuth-shaped tokens (have client_id or scope/scp) — Clerk session JWTs carry azp = frontend origin and would otherwise be rejected — If wrong: an OAuth token lacking both client_id and scope bypasses the client-id check (still needs valid sig + iss).
Task 5: dispatched (BASE 8c32043, implementer sonnet)
Task 5: review 1: Needs fixes — I1 client-id check fails open when OAuth-shaped token lacks client_id and azp
Task 5: ⚠️ resolved by controller — tests ran per report (5 passed); radar-core/examples/sign_in.rs exists (Task 4, 8c32043)
Task 5: minor (deferred): client_id/azp typed Option<String> → non-string claim now 401 (theoretical); no expired/wrong-alg tests
Task 5: fix round 1/5 (1 addressed, 0 open; commits 1fe6e1b..b038870)
Task 5: minor (deferred): OAuth token carrying azp but no client_id/scope/scp is classed as session token → client-id check skipped; confirm live Clerk claims (manual checklist)
Task 5: complete (commits 8c32043..b038870, review clean)
Task 6: dispatched (BASE b038870, implementer opus)
R14 Ruling: T6 deviation 1 accepted — failed pull leaves pulled=false (JS let a blind no-If-Match push overwrite unseen server data); Task 12 must retry pull on its drain/backoff schedule — If wrong: edits made while server unreachable at sign-in wait until a pull succeeds.
R15 Ruling: T6 deviation 2 accepted — network/token-refresh errors during drain stop the pass without counting a try (no navigator.onLine in Rust; offline would exhaust 8 tries and drop writes) — If wrong: a permanently unreachable server keeps items queued forever (desired).
R16 Ruling: T6 deviation 3 accepted — last_error also set on retryable failures (better sync-indicator signal) — If wrong: indicator shows transient errors.
R17 Ruling: T6 deviation 4 accepted — signed-out mutate returns NotSignedIn instead of queueing (app only mutates when signed in; local state persists and seeds on next sign-in) — If wrong: a write racing a sign-out is not queued (still saved locally).
Task 6: CARRY TO TASK 12: spawn pushes/PATCHes as detached tasks (mutate may wait on queue lock during a long drain); app schedules drain(), feeds DrainReport.succeeded to on_drained, calls reset()+pull on account change; retry failed pulls (R14); T4 minor re keychain blocking calls.
Task 6: minor (deferred): Task 2 fixture sync-queue items use string bodies; real JS queue stored objects — fix fixture fidelity
Task 6: review 1 (opus): Needs fixes — I1 401 retry resends same cached token; I2 queue mutex held across drain incl. backoff sleeps, no re-entrancy guard; I3 R14 (pulled=false on failed pull) lets a later successful pull overwrite offline edits
R18 Ruling: T6-I1 — add Session::force_refresh() (refresh regardless of expiry when a refresh_token exists; invalid_grant → signed out → Ok(None)); 401 retry uses it. In drain, a 401 after forced refresh stops the pass with last_error and counts a try (not dropped immediately) — If wrong: a server-side auth misconfig holds the queue up to 8 tries before dropping.
R19 Ruling: T6-I2 — SyncClient::drain uses try_lock (return early if a drain is running, JS `draining` guard); enqueue goes through a std::sync::Mutex inbox persisted immediately, merged by drain with group-key dedup (newer wins), so mutate never waits on a drain — If wrong: slightly more queue code.
R20 Ruling: T6-I3 SUPERSEDES R14 — keep pulled=false on failed pull, but record stable_key(local) at the failed pull; on a later successful pull, if local's key differs from the recorded one (user edited offline), push local with the fresh etag instead of replacing it with remote (archives: union then push). No divergence → replace with remote as before — If wrong: offline edits beat concurrent edits from another device (last-write-wins, same as the JS blind push).
R21 Ruling: server URL resolution (RADAR_SERVER_URL runtime → option_env! → http://127.0.0.1:8787) is not in any task's text; Task 12 implements it as radar_core::sync::server_url() when constructing SyncClient — If wrong: nothing; one fn.
Task 6: minor (deferred): direct mutate success doesn't remove same-groupKey queued item (stale PATCH can land later — JS parity); seed_local_only ignores non-2xx; malformed remote items dropped then removed server-side via 412 re-push; on_drained duplicates accept_snapshot; QueuedReq Vec load all-or-nothing (Task 12 loader should parse per item); network tests use 127.0.0.1:9 (~2s on Windows)
Task 6: fix round 1/5 (3 addressed, 0 open; commits 73352ac..aeab1af)
Task 6: minor (deferred): force_refresh compares against entry token → possible extra refresh on concurrent 401s; post-outage push that 4xx's (non-412) re-pushes on every retried pull; Queue::drain public but pulls client inbox (doc note); write after last absorb waits for next scheduled drain (app timer must drain while pending>0)
Task 6: CARRY TO TASK 12: account-boundary policy — mutate hitting invalid_grant mid-write returns NotSignedIn (not queued) and reset() clears failed_pull_key, so that edit is replaced by remote after re-sign-in; drain on a timer while pending>0
Task 6: complete (commits b038870..aeab1af, review clean)
Task 7: dispatched (BASE aeab1af, implementer opus)
Task 7: MANUAL CHECKLIST: window background color light/dark; variable-font weights 500/600 render; macOS keepalive (QuitMode::Explicit) after last window close
Task 7: review 1 (opus): Needs fixes — I1 quit flush loses edit (dirty cleared at snapshot not at write: cancelled-taken job, in-flight write at quit, failed save never retried); I2 QuitMode::Explicit with no reachable quit + no single-instance → orphan headless processes (plan-mandated ordering)
R22 Ruling: T7-I1 accepted as reviewer proposed — flush_now waits on written_gen, re-marks dirty if save_gen > written; on_app_quit awaits the in-flight/flush write; a failed background save re-marks dirty and reschedules — If wrong: none.
R23 Ruling: T7-I2 accepted, pulling single-instance forward from nowhere (no task owned it) — Task 7 adds a Quit action (secondary-q) + minimal macOS app menu "Quit Radar", and single-instance: a second launch exits 0 after (unless --autostart) asking the running instance to open/focus its main window over a local IPC channel bound to the current user — If wrong: extra IPC surface; local-only.
Task 7: minor (deferred): gpui_pre_windows ERROR log noise on window close/DXGI; ReqwestHttpClient FollowLimit approximated, unbounded body buffering, NoFollow path untested; on_rt panics if runtime shut down; ready gate in schedule_save is always true (doc overstates); to_snapshot deep-clones on UI thread; remove #[allow(dead_code)] once Task 8 adds callers; close handler path untested
Task 7: CARRY TO TASK 9: Newsreader italic not embedded but Card.jsx uses italic — embed italic variable TTF or match without it
Task 7: MANUAL CHECKLIST: macOS app menu "Quit Radar" + Cmd+Q; Unix single-instance socket path (compiled only on mac/linux) — run on a Mac; other-user denial of the Windows named pipe
Task 7: CARRY TO TASK 8: KeyBinding::new("secondary-…") panics — use KeyBinding::load (see task-7-report.md)
Task 7: fix round 1/5 (2 addressed, 0 open; commits f2f8552..d541e5d)
Task 7: minor (deferred): Unix concurrent-launch try_overwrite displacement race (use flock arbiter); Windows pipe squatting by another local user + no SECURITY_IDENTIFICATION on client; SDDL OW locks out elevated→non-elevated relaunch (grant user SID); /tmp fallback dir ownership unverified; unique_name test dirs not cleaned; path (b) in-flight write untested
Task 7: CARRY TO TASK 14: tray Quit must call cx.quit() so flush hook runs; WM_ENDSESSION path unverified
Task 7: complete (commits aeab1af..d541e5d, review clean)
Task 8: dispatched (BASE d541e5d, implementer opus)
Task 8: PAUSED by user (moving to another computer) — partial work committed as WIP (see next commit). Resume: re-dispatch Task 8 implementer with the WIP state described in docs/superpowers/sdd-handoff/task-8-pause-note.md; RED tests exist (16 failing in shell_tests.rs).
