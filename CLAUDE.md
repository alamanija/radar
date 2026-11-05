# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Radar is a native desktop app (GPUI Kit, Rust) for a personalized design-news briefing: it ingests RSS feeds, optionally has Claude summarize and categorize each article, and syncs state across devices through `radar-server`. It was migrated from a Tauri 2 + React/Vite app. The UI was ported from a Claude Design export (`radar-handoff`) that is not in this repo. The pre-GPUI React/Tauri app (including the React sources that were the visual reference: `src/components`, `src/views`, `src/styles.css`) lives on `main` before the cutover merge, and in git history at `5531ce5` (the parent of the cutover commit).

## Layout

**Cargo workspace** (root `Cargo.toml`, members: `radar-core`, `radar-server`, `radar-app`; version in `[workspace.package]`). Shared `target/` at repo root.

- `radar-core/` — UI-free library. Modules (`src/`):
  - `model.rs` — `RadarModel`: all mutable app state (articles, sources, categories, archives, prefs, profile) and its mutations. `types.rs` — wire/storage types (`Article`, `Source`, `Category`, `Archive`, `Prefs`, `Profile`, `Account`, `BriefingResponse`).
  - `store.rs` — JSON persistence (see Store compatibility).
  - `ingest.rs` — feed fetching + parsing. `summarize.rs` — Claude summarization. `keychain.rs` — Anthropic key + Clerk token storage. `text.rs` — text helpers. `search.rs` — palette search.
  - `schedule.rs` — pure "is the daily slot due" logic. `schedule_wake.rs` — OS relauncher (launchd / schtasks / systemd).
  - `auth.rs` — Clerk OAuth (PKCE) `Session`.
  - `sync/` — `mod.rs` (`SyncClient`, `server_url()`), `resource.rs` + `resources.rs` (profile/categories/sources/archives/prefs), `article_states.rs`, `queue.rs` (offline queue).
  - `radar_core::runtime()` is the shared tokio runtime. `examples/sign_in.rs` exercises the OAuth flow from the CLI.
  - `tests/fixtures/tauri_radar.json` is a hand-written store file in exactly the shape the Tauri app wrote (19-digit article ids as its webview rounded them, `syncQueue.js` queue items with their real group keys and object bodies), used by compatibility tests.
- `radar-app/` — the GPUI Kit desktop app. Its binary is named `radar`, as the Tauri app's was, so existing login items, the Windows install and the Tauri updater's relaunch all find it (`src/`):
  - `main.rs` (boot order is in its header comment), `app_model.rs` (`AppModel`), `sync_lifecycle.rs` (sign-in/out + live sync loop, child module of `app_model`), `scheduler.rs`, `single_instance.rs`, `bridge.rs` (GPUI executors <-> tokio, GPUI `HttpClient`), `theme.rs` (tokens, fonts), `workspace.rs` (window shell, routing).
  - `components/` — `card`, `header`, `sidebar`, `icon`, `toggle`, `tweaks`, `search_palette`, `row`, `skeleton`, `sync_indicator`.
  - `views/` — `briefing`, `saved`, `archive`, `sources`, `categories`, `settings`, `onboarding`, `empty_state`, `form`.
  - `platform/` — `tray`, `notify`, `autostart`, `updater`.
  - `assets/` — Inter / Newsreader / JetBrains Mono (OFL), `icons/ui/*.svg`, packaged app icons.
- `radar-server/` — Axum sync backend (Postgres via sqlx, Clerk token verification). Wire shapes in `src/types.rs`. See `radar-server/README.md`.
- `scripts/make-latest-json.sh` — builds the updater manifest in the release workflow.
- `docker-compose.yml` — Postgres 16 on `localhost:5433` for dev. `Dockerfile` + `render.yaml` — radar-server deploy.

## Commands

`radar-app` needs Rust 1.99+ (its `rust-version`; the pre-release `gpui-pre` crates require it). The user's default stable may be older, so use the pinned toolchain explicitly: `cargo +1.99.1 ...`.

- `cargo +1.99.1 run -p radar-app` — run the desktop app
- `cargo +1.99.1 test --workspace` — all tests (radar-server tests need no database)
- `cargo +1.99.1 fmt --check && cargo +1.99.1 clippy --workspace --all-targets -- -D warnings`
- `cargo +1.99.1 run -p radar-server` — sync server (needs Postgres and `radar-server/.env`)
- `cargo +1.99.1 packager -p radar-app --release` — installer bundle (needs `cargo install cargo-packager --version 0.11.8 --locked`); config in `radar-app/Cargo.toml > [package.metadata.packager]`
- `cargo +1.99.1 run -p radar-core --example sign_in` — OAuth smoke test, prints decoded claims only

Client config (runtime env first, then compile-time `option_env!`, then default): `RADAR_SERVER_URL` (default `http://127.0.0.1:8787`), `RADAR_CLERK_FRONTEND_API`, `RADAR_CLERK_CLIENT_ID`. See `.env.example`; the app does not load `.env` files itself, export them in your shell. Without the Clerk variables the app runs local-only.

## Architecture notes

- **State lives in `AppModel`** (`radar-app/src/app_model.rs`), one global `Entity<AppModel>` wrapping `radar_core::model::RadarModel`. Get it with `AppModel::global(cx)`; views `cx.observe` it. Every change goes through `update_model` / `mutate` (never mutate `model` directly): it notifies observers, re-applies theme tokens when visual prefs changed, schedules one debounced save (100 ms), and restarts the sync push debounce while signed in. Hydration only reads the store; `ready` flips after it and saves are gated on it. Everything outside the process (briefing pipeline, clock, notifications, tray, OS wake entry, login item, updater) goes through `Hooks`; the model starts with `Hooks::inert`, `main.rs` installs the real ones (`os_hooks`), tests install fakes.
- **Store compatibility.** `radar-core/src/store.rs` reads and writes the same `radar.json` the Tauri app wrote with `@tauri-apps/plugin-store`: a single top-level JSON object with keys `prefs`, `sources`, `articles`, `archives`, `categories`, `profile`, `sync-queue`, plus a `version` stamp (`SCHEMA_VERSION`). Path is `dirs::data_dir()/com.radar.dev/radar.json`. Each key is deserialized independently so one malformed key (or array item) is logged and defaulted instead of losing the rest. Writes go through a temp file that is flushed (`sync_all`) before the rename. Do not rename keys or change shapes without a migration branching on `version`; existing installs read their old file.
- **Article ids** (`ingest.rs::hash_id`) are the SipHash of `(source_id, url)` rounded to the nearest f64, in the form the Tauri app's webview stored them (`JSON.stringify` / `String(id)`). That keeps upgraded installs' read/bookmark state, archives and server article-state rows matching. Never change the id derivation.
- **Archive** is an append-only list of `{id, runAt, articles, errors}` snapshots, capped at 90 (`ARCHIVE_CAP`). Live `articles` and `archives[0]` diverge once the user toggles read/bookmark on live articles — by design.
- **Profile + run-at-launch.** `profile` is `{name, role, lens}`. `lens` is injected into Claude's system prompt as a "Reader context" block when non-empty. `runAtLaunch` (prefs) fires one briefing after hydration if there is a usable source (`scheduler::run_at_launch`).
- **Daily schedule.** `scheduleEnabled` + `scheduleTime` (`HH:MM` local) in prefs. Two layers:
  - In-process: `radar-app/src/scheduler.rs` ticks every 30 s after a 5 s warmup and uses `radar_core::schedule::should_fire` (enabled, today's slot passed, no run since). It goes through `AppModel::run_briefing`, which persists the result itself.
  - OS relauncher: `radar-core/src/schedule_wake.rs::sync_schedule_wake(enabled, hour, minute)`, idempotent. macOS: `~/Library/LaunchAgents/com.radar.scheduler.plist` (`StartCalendarInterval`, `launchctl load/unload -w`). Windows: `schtasks /Create /SC DAILY` task `com.radar.scheduler`. Linux: `~/.config/systemd/user/com.radar.scheduler.{service,timer}`. All launch the exe with `--autostart` (window not opened), after which the in-process scheduler sees the slot is past and fires.
- **Single instance.** `radar-app/src/single_instance.rs`: first process listens on a per-user local socket (named pipe on Windows); a second launch sends `show` (or `ping` with `--autostart`) and exits.
- **Source health.** Each source carries `lastFetchAt` (ms epoch or null) and `health` (`ok`/`warn`/`stale`). On briefing completion the model stamps `ok + now` for attempted sources and `warn` for sources in the `errors` array (ignoring the synthetic `sourceId: 0` Claude-summarization entry). `SourcesView` shows relative time or "never".
- **Theming** is token-driven in `radar-app/src/theme.rs` (light/dark, density, accent). Fonts are embedded with `include_bytes!`. The Tweaks panel is a floating control surface toggled by a FAB.
- **Ingestion** (`radar-core/src/ingest.rs`): `run_briefing` fetches feeds in parallel via `reqwest` (rustls), parses with `feed-rs`, maps to `Article`, sorts newest-first. Per-source errors are collected, not fatal. Sources without a feed URL are skipped.
- **Claude summarization** (`summarize.rs`): with an Anthropic key in the keychain and at least one category, one `claude-opus-4-7` request per article (raw HTTP). The JSON schema `enum` is built from the user's categories at request time. No prompt caching (the system prompt is under the cache minimum). Failures are non-fatal; the raw excerpt survives and the failure count appears in `errors` under a synthetic "Claude (summarization)" source.
- **Categories are user-owned.** No starter set (the Tauri app's `data.js` seeded none); the onboarding walks the user through adding the first one, and they can add/rename/delete. `{id, label, description, accent}`; the description is what Claude reads to place articles. Sources reference categories by **id**; stale ids fall into "Uncategorized". Deleting a category is blocked while a source uses it.
- **Keychain** (`radar-core/src/keychain.rs`): service `com.radar.dev`; users `anthropic` (API key; the UI only ever gets `{present, preview}`) and `clerk_oauth` (Clerk token set). The Tauri app's `clerk_db_jwt` entry is deleted once per launch, best effort and off the UI thread (`delete_legacy_entries`, called only from `main.rs`). `OsKeyStore` is the real backend, tests use fakes.
- **Search.** `SearchPalette` (Cmd/Ctrl+K, or click the header search bar) searches current articles + all archive articles (deduped by id, current wins) + sources; logic in `radar-core/src/search.rs`.
- **Views.** The shared page frame is `page_frame` / `page_frame_with` in `views/briefing.rs`; reuse it for new views. `ArchiveView` reuses `Card` read-only.
- **Known-unfinished stubs.** "Staleness threshold" in Settings is "Not wired". "Gmail — newsletter inbox" is a disabled button.

## Auth (Clerk OAuth)

`radar-core/src/auth.rs`: OAuth 2.0 Authorization Code + PKCE (S256) for a public client, through the system browser with a loopback redirect `http://127.0.0.1:17823/callback`. Scopes `openid profile email offline_access`. `Session` is keychain-backed (`clerk_oauth`), refreshes the access token when under 60 s remain, and is what `SyncClient` asks for a bearer. Tokens, codes and the PKCE verifier are never logged or put in error messages. The id token is decoded without verification only to display who is signed in; authorization is the server's job. Sign-in lives in Settings; `sync_lifecycle.rs` runs sign-in/out. While the browser flow runs, Settings shows "Loading auth…" with a Cancel pill (`cancel_sign_in`): the flow is dropped inside its `block_on`, which frees port 17823 for the next attempt.

**Clerk dashboard setup:** create an OAuth application with client type **Public**, **Require PKCE**, redirect URI `http://127.0.0.1:17823/callback`, scopes `openid profile email offline_access`, access token format **JWT**. Put its client id in `RADAR_CLERK_CLIENT_ID` and the instance Frontend API URL in `RADAR_CLERK_FRONTEND_API`. Server side: `CLERK_JWKS_URL`, `CLERK_ISSUER`, and optionally `CLERK_OAUTH_ISSUER` / `CLERK_OAUTH_CLIENT_ID` (see `radar-server/README.md`).

## Cross-device sync

Profile, categories, sources, archives, article states and prefs sync via the app calling `radar-server` directly.

- **Server verification** (`radar-server/src/clerk.rs`): Bearer token verified against Clerk's JWKS (`jwks.rs`, 1 h cache), `alg: RS256`, `iss` checked, `aud` not. Accepts both OAuth access tokens (the native client) and legacy session JWTs. `sub` is the user key; all rows are keyed on `clerk_user_id text`. No users/sessions tables.
- **Server hardening.** Each `/sync/*` handler runs in a transaction with `FOR UPDATE` locking; per-resource shape/size limits (categories <= 64, sources <= 512, archives <= 90, 8 MiB archives, 64 KiB prefs); per-user token bucket (`rate_limit.rs`, 40 capacity / 10 rps); 10 MiB request body cap. CORS is env-driven (`CORS_ORIGINS`); the native client sends no Origin so it is irrelevant to it.
- **Client** (`radar-core/src/sync/`): `SyncClient` attaches the bearer, tracks `ETag`, and routes mutations through the offline queue. `SyncedResource` (`resource.rs`) implements pull-on-sign-in and push-on-change with the shared pattern; `AppModel` debounces pushes by 800 ms (`PUSH_DEBOUNCE`). Guards: a resource doesn't push until it has pulled, and a last-known server shape drops no-op echoes. If the server has no row yet but local is non-empty, the pull seeds the server from local. Account A -> B resets both guards, and `sync_epoch` aborts in-flight work from the old account so it never sends with B's token.
- **Conflict detection.** GET carries `updated_at` + `ETag`; PUT sends `If-Match: W/"<updatedAt>"`. A 412 makes the client refetch, merge, update the etag and stop. No silent overwrites.
- **Last-synced digests** (`sync/last_synced.rs`): `sync-state.json` beside `radar.json` (deliberately not a `radar.json` key) holds `{accountId, digests, saved}`: `digests` is the SHA-256 of each full-list resource's last-synced stable key, `saved` the SHA-256 of the stable key that same save wrote to `radar.json`. It is written with every store save, always after the store file. At hydration a resource keeps its digest only if the store loaded cleanly (`store::load_checked`) and its loaded value matches `saved` (`LastSynced::checked`); otherwise (missing, corrupt or replaced store, stale sidecar) its first pull takes the server row. At the next `start_sync` of the same account (never another) the digests seed each resource's divergence baseline (`SyncedResource::seed_last_synced`). A value edited since the last sync (the quit push never landed, a crash, a power loss) is then pushed by the first pull instead of being replaced by the older server row. The first pull pushes it only if the server still holds the last-synced value, adopts the server's copy if it already equals local, and otherwise keeps the server's (another device wrote since); archives always push their union. Sign-out deletes the file.
- **Offline queue** (`sync/queue.rs`, persisted under the `sync-queue` store key): network failure / 5xx / 408 / 429 enqueues the request (retry with backoff, `MAX_TRIES = 8`) and the caller sees a synthetic 202. Drains on connectivity return, on startup and on new enqueue. Dedup by group key (PUTs per endpoint, article-state PATCHes per article id). Eventual success updates etag/server shape so no stale `If-Match` is sent. The JS-era queue shape is still read.
- **Flush on close.** Closing the main window flushes the model and the pending push (the app keeps running in the tray); quit flushes via the model's quit hook.
- **Resource semantics.** Profile `{name, role, lens}` upsert. Categories and sources are full-list replace (jsonb per user); sources travel only `{id, name, category, feedUrl, enabled, isDefault}` and merge this device's `lastFetchAt`/`health` back by id on pull. Archives are opaque to the server and unioned on pull (dedup by id, sorted by `runAt` desc, capped at 90). Prefs are full replace of the visual + briefing-behavior subset; per-device UI state (sidebar collapse, current view) stays local. Article states are per-row `(clerk_user_id, article_id)` PATCHes (not full-list); server wins on conflict when seeding.

## Tray, notifications, autostart, updates

- **Tray** (`platform/tray.rs`): icon embedded from `assets/icons/64x64.png`. Static menu: Show Radar, Run briefing now, Quit Radar. Left-click toggles the window, right-click opens the menu. Status is in the hover tooltip (`N unread · last run …` plus top headlines) and the macOS title (unread count), pushed via `set_status`. Closing the window keeps the app alive (`QuitMode::Explicit`); Cmd/Ctrl+Q or the tray quits. macOS Dock click reopens the window.
- **Notifications** (`platform/notify.rs`): a banner when a briefing brings in new articles and the window isn't focused.
- **Open at login** (`platform/autostart.rs`, `auto-launch` crate): launches with `--autostart`, which boots to the tray without a window. At startup `main.rs` re-registers an enabled entry for the current executable (`repoint_login_item`, on the blocking pool; idempotent; never creates one), so an entry an older build wrote points here.
- **Updater** (`platform/updater.rs`): `cargo-packager-updater` against `https://github.com/alamanija/radar/releases/latest/download/latest.json`, verified with the existing minisign public key (`PUBKEY`), so Tauri-era installs and this app trust the same signatures. A silent check runs on launch; the model is flushed to disk between download and install.

## Logging

`flexi_logger` (`main.rs::init_logging`): rolling `Radar*.log` files (5 MB each), plus stdout in debug builds. macOS: `~/Library/Logs/com.radar.dev/`. Windows: `%LOCALAPPDATA%\com.radar.dev\logs\`. Linux: `$XDG_DATA_HOME/com.radar.dev/logs/`. Level via `RUST_LOG`, default `info`. Use `log::info!/warn!/error!`. The scheduler logs `[scheduler] …` lines; this is the first place to look when a scheduled slot goes quiet. A second launch exits before logging starts so it never rotates the running instance's file.

## Releases and updates

`.github/workflows/release.yml`, triggered by a `vX.Y.Z` tag. It first fails if a Clerk variable or a signing secret is empty and checks the tag against `[workspace.package] version`, then creates the draft release (reused on a re-run), builds with `cargo packager -p radar-app --release` (cargo-packager 0.11.8; macOS Apple Silicon: app + dmg; Windows x86_64: nsis), signs, uploads, then `scripts/make-latest-json.sh` builds `latest.json` and the draft is published. Intel macOS and Linux are not built. Running the workflow by hand on a branch is a dry run: same checks, build and signing, outputs attached to the run as artifacts, no release created.

Version bump (single source: `[workspace.package]` in the root `Cargo.toml`; needs `cargo install cargo-edit`):

```bash
cargo set-version --bump patch    # or minor / major
git commit -am "chore: release vX.Y.Z" && git tag vX.Y.Z
git push && git push --tags
```

Never edit crate versions by hand per crate; they inherit the workspace version.

**One-time setup:** repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the existing minisign key, which cargo-packager reads via `CARGO_PACKAGER_SIGN_PRIVATE_KEY*`); repo variables `RADAR_CLERK_FRONTEND_API` and `RADAR_CLERK_CLIENT_ID` (compile-time client config; a repo secret of the same name also works, the variable wins; `RADAR_SERVER_URL` is set in the workflow). Without Apple signing secrets macOS builds are unsigned (right-click > Open). Rolling back: edit the bad release back to draft or delete it so `releases/latest` resolves to the prior one; clients that already applied it need a fix-forward release.

## Known gaps

- Linux tray (AppIndicator on a GTK thread) compiles but is untested; the GTK path isn't compiled on the macOS dev machine.
- Windows-only code paths are not compiled on the dev Mac: the schtasks `CREATE_NO_WINDOW` handling, the tray tooltip length limit (`text::tray_tooltip`), the named-pipe single-instance DACL. Reviewed by eye, not run.
- Linux `sync_schedule_wake` (systemd units) is likewise unexercised.
- WebView is unused: sign-in goes through the system browser and nothing renders HTML inside the app.
- The tray icon uses the alpha mask of the color app icon; a dedicated monochrome PNG would be crisper.
- No cross-platform CI test pipeline; the release workflow only builds.
