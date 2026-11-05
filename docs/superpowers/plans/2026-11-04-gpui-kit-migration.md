# GPUI Kit Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Radar's Tauri + React desktop app with a native Rust app built on GPUI Kit, keeping every current feature, the user's on-disk data, and cross-device sync.

**Architecture:** Two new workspace crates replace `src-tauri/` and `src/`:
- `radar-core` is a UI-free library. It holds the domain model, ingest, summarization, keychain, the JSON store, the sync client and offline queue, Clerk OAuth (PKCE over a loopback redirect), and the scheduling logic.
- `radar-app` is the GPUI Kit binary. It holds one global `Entity<AppModel>` that wraps `radar_core::model::RadarModel`, with views that observe it. It also owns the tray (`tray-icon`), system notifications, autostart (`auto-launch`), the updater (`cargo-packager-updater`), and packaging (`cargo-packager`).

Async network work runs on a shared tokio runtime owned by `radar-core`. GPUI tasks await it through a bridge, because GPUI's executors are not tokio.

**Tech Stack:**
- Rust 1.92+
- `gpui-kit = "0.9.0"` (backend `gpui-pre 0.3.11`)
- tokio, reqwest (rustls), feed-rs, keyring 3, serde_json, chrono, `dirs`
- `tray-icon`, `auto-launch`, `flexi_logger`, `oauth2`-free hand-rolled PKCE (`sha2` + `base64` + `rand`)
- `cargo-packager` + `cargo-packager-updater`
- `httpmock` for tests (dev-dependency)

**Spec:** There is no separate spec doc. The plan argues from:
1. The user's decisions in the planning session: **full replacement**, meaning Tauri and React are deleted with no parallel app, and **browser OAuth + PKCE** for Clerk.
2. `CLAUDE.md`, the behavioral reference for every existing feature.
3. `radar-handoff/project/` and the current `src/**/*.jsx` + `src/styles.css`, the visual reference. The JSX files are kept until Task 16 so porters can read them.

## Global Constraints

- Work on branch `gpui-migration` (in a worktree via superpowers:using-git-worktrees), not on `main`.
- `gpui-kit = "0.9.0"`. Imports are `use gpui_kit::*;` and `use gpui_kit::component::*;`. Call `gpui_kit::init(cx)` before anything else. Open windows with `gpui_kit::open_window(...)`, which supplies `Root` itself, so never wrap the view in `Root` by hand.
- Toolchain floors:
  - Rust 1.92+.
  - Windows: MSVC toolchain, VS 2022 Build Tools, and CMake on PATH.
  - macOS 15+.
- App identifier stays `com.radar.dev`. Keychain service stays `com.radar.dev`, and Anthropic key username stays `anthropic`.
- The store file stays `radar.json` in `dirs::data_dir()/com.radar.dev/`, the same path Tauri's plugin-store used. Existing users must keep all of their data.
- Store keys are unchanged: `version`, `prefs`, `sources`, `articles`, `archives`, `categories`, `profile`, `sync-queue`. JSON shapes stay camelCase and byte-compatible with what the React app wrote.
- Behavior constants carried over verbatim:
  - `ARCHIVE_CAP = 90`.
  - Sync push debounce 800 ms.
  - Queue `MAX_TRIES = 8`, backoff `min(60_000, 500 * 2^min(tries,6))` ms.
  - Retryable statuses: 408, 429, 5xx.
  - Scheduler tick 30 s, warmup 5 s.
  - Default `scheduleTime` is `"07:30"`.
  - Feed fetch timeout 15 s. Claude request timeout 30 s.
  - Model stays `claude-opus-4-7`, because the port does not change behavior.
- Copy strings come from the current JSX (titles, placeholders, button labels, tooltip format, notification text "Radar briefing ready" / `"{n} new · {total} total across {s} source(s)"`).
- Server URL: runtime env `RADAR_SERVER_URL`, then compile-time `option_env!("RADAR_SERVER_URL")`, then `http://127.0.0.1:8787`.
- Clerk config, resolved the same way (runtime env, then compile-time env): `RADAR_CLERK_FRONTEND_API` (e.g. `https://<app>.clerk.accounts.dev`) and `RADAR_CLERK_CLIENT_ID`. OAuth redirect: `http://127.0.0.1:17823/callback`. That fixed port must be registered on the Clerk OAuth app. Scopes: `openid profile email offline_access`.
- Never log access tokens, refresh tokens, or the Anthropic key.
- Fonts: Inter (400/500/600), Newsreader (400/500/600), and JetBrains Mono (400/500), embedded as TTF under `radar-app/assets/fonts/` (all OFL, include `OFL.txt`).

## Review Focus

1. **Existing Tauri-era `radar.json` loads intact.** Sources, categories, archives, prefs, profile and a non-empty `sync-queue` must all survive first launch of the new binary. Test: `store::tests::loads_tauri_era_file` (Task 2).
2. **Window closed while a briefing runs, or while the scheduler fires.** The model still updates, persists, and notifies, and reopening the window shows the results. Test: `app_model::tests::briefing_completes_with_no_window` (Task 14).
3. **The user abandons sign-in** (closes the browser tab, denies consent) **or port 17823 is taken.** The flow ends with a clear error within the timeout and the UI returns to signed-out instead of hanging. Tests: `auth::tests::callback_times_out`, `auth::tests::port_in_use_errors` (Task 4).
4. **Access token expires while mutations sit in the offline queue.** Drain refreshes the token first, and no write is dropped as a 401. Test: `sync::queue::tests::drain_refreshes_expired_token` (Task 6).
5. **Feeds or articles with bad data:** no image, a broken image URL, very long titles, or an empty summary. The card renders its placeholder and truncates, with no panic and no layout blowout. Test: `card::tests::renders_degraded_article` (Task 9).

---

## File Structure

```
Cargo.toml                       workspace: members = ["radar-core", "radar-app", "radar-server"]
radar-core/
  Cargo.toml
  src/lib.rs                     pub mods + `runtime()` (shared tokio Runtime) + `block_on_rt`
  src/types.rs                   wire/persisted shapes: Article, Source, Category, Archive, Prefs, Profile, SourceHealth, BriefingResponse, SourceError
  src/ingest.rs                  moved from src-tauri (tauri attrs removed)
  src/summarize.rs               moved verbatim
  src/keychain.rs                moved; Clerk dev-JWT fns deleted; OAuth token fns added
  src/schedule_wake.rs           moved; `#[tauri::command]` removed
  src/schedule.rs                pure `should_fire` + `parse_time` (from scheduler.rs)
  src/store.rs                   JSON store: load/migrate/save (atomic), path compat
  src/model.rs                   RadarModel: all state + mutations (pure, no IO)
  src/text.rs                    greeting, relative_time_ms, tray tooltip, slugify, truncate
  src/search.rs                  search over articles/archives/sources
  src/auth.rs                    PKCE, loopback listener, token exchange/refresh, Session
  src/sync/mod.rs                SyncClient (HTTP + bearer), SyncStatus
  src/sync/queue.rs              persisted offline queue + drain
  src/sync/resource.rs           SyncedResource<R> state machine (port of useSyncedResource)
  src/sync/resources.rs          ProfileRes, CategoriesRes, SourcesRes, PrefsRes, ArchivesRes
  src/sync/article_states.rs     pull/overlay/seed/patch
radar-app/
  Cargo.toml                     [package.metadata.packager] config
  build.rs                       Windows icon resource
  assets/fonts/*.ttf, assets/icons/*
  src/main.rs                    application(), init, fonts, logger, --autostart, keepalive
  src/app_model.rs               Entity<AppModel>: wraps RadarModel + Session + sync + persistence debounce + briefing runner
  src/bridge.rs                  `on_rt(cx, fut)` — await a tokio future from a GPUI task
  src/theme.rs                   Radar tokens (light/dark × 5 accents) → gpui-kit Theme
  src/workspace.rs               window root view: sidebar + header + content + overlays, Ctrl+K action
  src/components/{sidebar,header,card,row,skeleton,toggle,tweaks,search_palette,sync_indicator,icon}.rs
  src/views/{briefing,saved,archive,sources,categories,settings,empty_state,onboarding}.rs
  src/platform/{tray,notify,autostart,updater}.rs
  src/scheduler.rs               GPUI-side 30 s loop calling AppModel::run_briefing
radar-server/src/clerk.rs        accept Clerk OAuth JWT access tokens
.github/workflows/release.yml    cargo-packager matrix
```

Deleted in Task 16: `src-tauri/`, `src/`, `index.html`, `vite.config.*`, `package.json`, `package-lock.json`, `.nvmrc`, `scripts/sync-version.mjs`, `.env.example` (replaced).

---

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

### Task 4: Clerk OAuth (PKCE + loopback), session and token storage

**Files:**
- Create: `radar-core/src/auth.rs`
- Modify: `radar-core/src/keychain.rs` (add the token fns)

**Interfaces:**
- Produces:
  - `auth::AuthConfig { frontend_api: String, client_id: String, redirect_port: u16 /*17823*/ }` with `AuthConfig::from_env() -> Option<Self>`.
  - `auth::Tokens { access_token, refresh_token: Option<String>, expires_at: i64 /*unix s*/, id_token: Option<String> }` (Serialize, Deserialize).
  - `auth::Account { id: String /*sub*/, email: String, name: String, image_url: Option<String> }`, built from unverified `id_token` claims `sub,email,name,picture`. It is display-only, and the server verifies the access token.
  - `auth::Pkce::new() -> Pkce { verifier, challenge, state }` (S256, 64-char verifier).
  - `auth::sign_in(cfg: &AuthConfig, open: impl FnOnce(&str), timeout: Duration) -> Result<(Tokens, Account), AuthError>`. It binds `127.0.0.1:{port}`, then calls `open(authorize_url)`, waits for `GET /callback?code&state`, serves a small "You can close this tab" HTML page, and exchanges the code at `{frontend_api}/oauth/token`.
  - `auth::refresh(cfg, refresh_token) -> Result<Tokens, AuthError>`.
  - `auth::Session` is a `Clone` wrapper, `Arc<Mutex<Option<Tokens>>>` + cfg:
    - `Session::load(cfg) -> Self` reads the keychain.
    - `Session::access_token(&self) -> Result<Option<String>, AuthError>` refreshes when `expires_at - now < 60` and persists the result.
    - `Session::sign_out()` clears the keychain.
    - `Session::account() -> Option<Account>`.
  - `AuthError` = `PortInUse | Timeout | Denied(String) | StateMismatch | Http(String) | NotConfigured`.
  - `keychain::KeyStore` trait (`get(user) -> Result<Option<String>>`, `set(user, &str)`, `delete(user)`), with `OsKeyStore` (keyring, service `com.radar.dev`) and `MemKeyStore` (pub, for tests in both crates). The Anthropic-key fns take `&dyn KeyStore`, and `ingest::run_briefing` uses `OsKeyStore`. OAuth tokens are stored as JSON under username `clerk_oauth`.
  - `Session::load(cfg, store: Arc<dyn KeyStore>)`.

- [ ] **Step 1: Write failing tests:**
  - `pkce_rfc7636_vector`: the verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` gives the challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`.
  - `authorize_url_has_params`: the URL contains `response_type=code`, `code_challenge_method=S256`, the URL-encoded redirect `http%3A%2F%2F127.0.0.1%3A17823%2Fcallback`, and `scope=openid+profile+email+offline_access`.
  - `callback_exchanges_code`: `httpmock` serves `/oauth/token`, and the test simulates the browser by GETting the loopback with the correct state. The result is `Ok`, and the request body had `grant_type=authorization_code`, `code_verifier`, `client_id`, and no `client_secret`.
  - `callback_state_mismatch_errors` returns `StateMismatch`.
  - `callback_error_param_is_denied`: `?error=access_denied` returns `Denied("access_denied")`.
  - `callback_times_out`: with a 200 ms timeout and no request, the result is `Timeout`.
  - `port_in_use_errors`: the port is pre-bound, and the result is `PortInUse`.
  - `session_refreshes_when_near_expiry`: `expires_at = now+30` causes one refresh call, and the new token is returned.
  - `account_from_id_token`: a hand-built unsigned JWT payload maps to `Account`.
- [ ] **Step 2: Run `cargo test -p radar-core auth`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Run the loopback listener on `tokio::net::TcpListener` with a minimal HTTP request-line parse; don't add a server framework. The token endpoint is form-encoded through reqwest. Tests set `frontend_api` to the httpmock base URL and use an ephemeral port from `AuthConfig { redirect_port: 0 }`. With port 0, `sign_in` reports the bound port through the `open` URL.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): Clerk OAuth PKCE loopback sign-in"`.

### Task 5: Server accepts Clerk OAuth access tokens

**Files:**
- Modify: `radar-server/src/clerk.rs`, `radar-server/src/state.rs`, `radar-server/src/main.rs`, `radar-server/.env.example`, `radar-server/README.md`

**Interfaces:**
- Produces: the optional env `CLERK_OAUTH_CLIENT_ID`. When set, a token that carries `client_id` (or `azp`) must equal it. Tokens are still RS256, use the same JWKS, and must have `iss == CLERK_ISSUER`. `sub` becomes `ClerkUser.id`, unchanged.

- [ ] **Step 1: Manual check against a live Clerk dev instance.** Create the OAuth app as **Public** with **Require PKCE**, the redirect `http://127.0.0.1:17823/callback`, the `offline_access` scope, and the JWT access token format. Then sign in once with the `radar-core` example `cargo run -p radar-core --example sign_in`, which prints the decoded *claims* only, never the token. Record `iss`, `sub`, `aud`, `azp`, `client_id` and `scope` in the commit message. If `iss` differs from the session issuer, also accept a configured `CLERK_OAUTH_ISSUER`.
- [ ] **Step 2: Write failing tests** in `clerk.rs`, signing with a test RSA key and injecting the JWKS into `JwksCache` through a test constructor:
  - `accepts_oauth_jwt_with_matching_client_id`;
  - `rejects_oauth_jwt_with_other_client_id`;
  - `accepts_session_jwt_without_client_id`, which keeps today's behavior;
  - `rejects_wrong_issuer`.
- [ ] **Step 3: Run `cargo test -p radar-server clerk`.** Expected: FAIL.
- [ ] **Step 4: Implement it.** Extend `ClerkClaims` with `client_id: Option<String>` and `azp: Option<String>`. Update the README: bearer is now a Clerk OAuth access token from the desktop app, plus the dashboard setup steps from Step 1.
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit** with `git commit -m "feat(server): accept Clerk OAuth access tokens"`.

### Task 6: Sync client, offline queue, synced resources, article states

**Files:**
- Create: `radar-core/src/sync/{mod,queue,resource,resources,article_states}.rs`

**Interfaces:**
- Consumes:
  - `auth::Session::access_token`
  - `model::RadarModel` (resources read and write through closures, so sync never owns the model)
  - `store` (the queue persists through a callback)
- Produces:
  - `sync::SyncClient::new(base: String, session: Session) -> Self`
  - `get(path) -> Result<Resp, SyncError>`
  - `mutate(QueuedReq) -> Result<Resp, SyncError>`. On a retryable status or a network error it enqueues and returns `Resp::Accepted` (port of `syncQueue.queuedMutation`).
  - `Resp { status: u16, etag: Option<String>, body: serde_json::Value }`
  - `sync::queue::Queue`:
    - `{ items: Vec<QueuedReq> }`
    - `QueuedReq { id, path, method, body, headers: Vec<(String,String)>, group_key: Option<String>, account_id: Option<String>, tries: u32, queued_at: i64 }`. Serde keys match the JS (`groupKey`, `accountId`, `queuedAt`) so an old queue drains.
    - `Queue::enqueue(req)` dedups by `group_key`.
    - `Queue::drain(&mut self, client, account_id, persist: impl Fn(&[QueuedReq])) -> DrainReport { succeeded: Vec<(String /*group_key*/, Value)>, dropped, remaining }`.
  - `sync::SyncStatus { pending: usize, inflight: bool, last_error: Option<String>, last_success_at: Option<i64> }` through `SyncClient::status() -> watch::Receiver<SyncStatus>`.
  - `sync::resource::SyncResource` trait:
    - `const ENDPOINT: &str; type Local: Clone;`
    - `fn push_body(&Local) -> Value; fn from_remote(&Value, prev: &Local) -> Local; fn stable_key(&Local) -> String; fn is_empty(&Local) -> bool;`
  - `sync::resource::SyncedResource<R>`:
    - `{ pulled: bool, server_key: Option<String>, etag: Option<String> }`
    - `async fn pull(&mut self, client, local: &R::Local) -> Result<Option<R::Local>, SyncError>`, which seeds the server when it returns 404 and local is non-empty.
    - `fn needs_push(&self, local) -> bool`
    - `async fn push(&mut self, client, local) -> Result<Option<R::Local>, SyncError>`. `Some` is the merged value after a 412 refetch + merge + single retry.
    - `fn on_drained(&mut self, body: &Value, local)`
    - `fn reset()`, for account switches.
  - `resources::{ProfileRes, CategoriesRes, SourcesRes, PrefsRes, ArchivesRes}` implement the trait. Their bodies and merges are ported 1:1 from `App.jsx:193-381`: `sourceWire` fields only, health preserved by id, prefs limited to the 9 synced keys merged over defaults, and archives union-merged + sorted by `runAt` desc + capped at 90.
  - `article_states::{pull(client) -> HashMap<String, ArticleState>, seed_local_only(client, local_articles, server_map), patch(client, id, state)}`, where `group_key = "PATCH:/sync/article-states:{id}"`.

- [ ] **Step 1: Write failing tests** with `httpmock`, mirroring the JS:
  - `pull_404_seeds_from_local`: PUT is called with no `If-Match`.
  - `pull_404_empty_local_no_push`.
  - `push_sends_if_match_weak_etag`: the header is exactly `W/"<updatedAt>"`.
  - `push_412_refetches_merges_retries_once`, which also covers the archives union so local-only entries reach the server.
  - `noop_echo_not_pushed`: the stable key equals the server key, so `needs_push` is false.
  - `sources_stable_key_ignores_health`: bumping `last_fetch_at` doesn't trigger a push.
  - `mutate_503_enqueues_returns_accepted`.
  - `queue_dedups_by_group_key`.
  - `queue_skips_other_account_items`.
  - `queue_drops_non_retryable_4xx`: 412 and 400 are dropped, and `last_error` is set.
  - `queue_gives_up_after_8_tries`, using an injectable sleep fn so the test is instant.
  - `drain_refreshes_expired_token` (Review Focus 4): the session holds an expired token, and the mock token endpoint plus mock PUT see a fresh bearer.
  - `legacy_js_queue_item_deserializes`.
  - `article_states_seed_skips_server_known_ids`.
- [ ] **Step 2: Run `cargo test -p radar-core sync`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** The backoff formula is from Global Constraints. The sleep function is a field on `Queue` (`Arc<dyn Fn(Duration) -> BoxFuture>`), defaulting to `tokio::time::sleep`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): sync client, offline queue, synced resources"`.

### Task 7: `radar-app` scaffold: window, fonts, theme, global model, persistence, close-to-keepalive

**Files:**
- Create: `radar-app/Cargo.toml`, `radar-app/build.rs`, `radar-app/src/{main,app_model,bridge,theme,workspace}.rs`, `radar-app/assets/fonts/*` (+ `OFL.txt`), `radar-app/assets/icons/` (copy `src-tauri/icons/{64x64.png,icon.png,icon.ico,icon.icns}`)
- Modify: `Cargo.toml` (members add `radar-app`; add `[profile.dev.package]` `opt-level = 3` for the `gpui-pre*` and `gpui-kit*` crates per the GPUI Kit install docs)

**Interfaces:**
- Consumes: `RadarModel`, `store`, `Session`, `SyncClient`, `runtime()`.
- Produces:
  - `bridge::on_rt<T: Send + 'static>(fut: impl Future<Output=T> + Send + 'static) -> impl Future<Output=T>`. It does `runtime().spawn(fut)` and awaits the JoinHandle.
  - `theme::Tokens { bg, surface, surface_2, border, border_strong, text, text_2, text_3, accent, accent_soft, accent_text, dot: [Hsla; 6] /*olive,rust,slate,plum,sand,ink*/, row_h: Pixels, density_pad: Pixels, radius: Pixels, radius_lg: Pixels }`.
  - `theme::tokens(theme: types::Theme, accent: Accent, density: Density) -> Tokens`. Values come verbatim from `src/styles.css:2-57` and `ACCENTS` at `App.jsx:41-47`.
  - `theme::apply(cx: &mut App, &Prefs)`. It stores `Tokens` as a GPUI global and maps them onto gpui-kit `Theme` through `Theme::update` (background, foreground, border, primary = accent, font_family "Inter", mono "JetBrains Mono", radius 6). Read it with `theme::t(cx) -> &Tokens`.
  - `app_model::AppModel`:
    - `{ model: RadarModel, session: Session, account: Option<Account>, sync: Option<SyncClient>, sync_status: SyncStatus, ready: bool, store_path: PathBuf, save_task: Option<Task<()>> }`.
    - `AppModel::global(cx) -> Entity<AppModel>`.
    - `fn update_model(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut RadarModel))`. It calls `cx.notify()` and schedules a debounced save at 100 ms, matching plugin-store autoSave. Every mutation goes through this function.
  - `workspace::Workspace` is the root view that renders the shell; content arrives in Task 9.
  - `main.rs`:
    1. `gpui_kit::application().with_assets(..)`, installing a reqwest-backed `HttpClient` so `img(url)` works.
    2. `gpui_kit::init`, register fonts, init `flexi_logger` (5 MB rotation, `dirs::data_local_dir()/com.radar.dev/logs` on Windows, the matching OS log dirs elsewhere, `tao`/`wry` filters dropped).
    3. Create the global `AppModel` from `store::load`.
    4. Open the window: 1280×820, min 920×620, title "Radar". Skip it when `--autostart` is in args.
    5. `window.on_window_should_close` returns `true` and lets the window close. The app stays alive (no `cx.quit()` on last window) so the tray can reopen it. `open_main_window(cx)` is idempotent and focuses an existing window.

- [ ] **Step 1: Write failing tests:**
  - `theme::tests::light_camel_tokens`: `bg == #F5F3F0`, `accent == #9A7B5B`, `accent_soft == #EEE5D8`.
  - `theme::tests::dark_slate_tokens`: `accent == #8896A6`, `accent_text == #A8B4C2`.
  - `theme::tests::compact_density`: `row_h == 36`, `density_pad == 14`.
  - `app_model::tests::mutation_persists_after_debounce`, as a `#[gpui_kit::test]` with a temp store path: `update_model(add_source)`, advance the clock 150 ms, reload the file, and `sources.len() == 1`.
  - `app_model::tests::hydrate_does_not_overwrite_store`: load the fixture, take no action, and the file stays byte-identical.
- [ ] **Step 2: Run `cargo test -p radar-app`.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Manual check:** `cargo run -p radar-app`.
  - The window opens with the right background color.
  - Closing the window leaves the process alive (check Task Manager / `ps`).
  - On Windows and macOS the process does not exit by itself. If GPUI quits on last-window-close on either platform, record that in the commit and keep a hidden 1×1 window as the fallback.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): GPUI Kit scaffold, theme tokens, persisted global model"`.

### Task 8: Shell: sidebar, header, view routing, Tweaks panel, Ctrl+K action

**Files:**
- Create: `radar-app/src/components/{sidebar,header,icon,toggle,tweaks}.rs`
- Modify: `radar-app/src/workspace.rs`
- Reference: `src/components/{Sidebar,Header,Icon,Toggle,Tweaks}.jsx`, `src/styles.css`

**Interfaces:**
- Consumes: `AppModel`, `theme::t`.
- Produces:
  - `actions!(radar, [OpenSearch, RunBriefing, ToggleSidebar, ToggleTheme])`. Bindings: `secondary-k` → `OpenSearch`, context `"Workspace"`. The workspace focuses its handle on window open.
  - `icon::Icon::new(IconName)` covers the icon set in `Icon.jsx`, drawn as embedded SVG assets under `assets/icons/ui/` and copied from the JSX paths.
  - `Workspace { search_open: bool, tweaks_open: bool, sources_intent: Option<SourcesIntent> }`.

- [ ] **Step 1: Write failing headless tests** (`#[gpui_kit::test]`):
  - `sidebar_click_sets_view`: clicking "Saved" sets `prefs.view == Saved` and persists it.
  - `sidebar_category_filter_sets_active_category`.
  - `sidebar_counts`: the unread count badge equals `unread_count()`.
  - `header_theme_toggle_flips_and_reapplies_tokens`.
  - `ctrl_k_opens_search`: dispatch `OpenSearch` and `search_open == true`.
  - `toggle_sidebar_collapses` (`prefs.sidebar_collapsed`).
  - `tweaks_accent_swatch_updates_prefs_and_tokens`.
  - `sidebar_footer_shows_profile_name_or_account`.
- [ ] **Step 2: Run `cargo test -p radar-app shell`.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
  - Match the JSX layout: 260 px sidebar, collapsible to 0. Animation is optional.
  - Header: sidebar toggle, title per view, the grid/list segmented control shown only on the briefing view, a search bar that opens the palette, theme toggle, and a "Run briefing" button with a spinner while `scanning`.
  - Tweaks: a FAB at bottom-right 20 px with Theme, View, Density, Accent, Serif, Accent bars, and Sidebar collapsed.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Visual check.** Run the app side by side with `radar-handoff/project/` open in a browser, in light and dark. Fix any spacing or color drift over 2 px.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): sidebar, header, tweaks, keybindings"`.

### Task 9: Briefing view, Card, Row, Skeleton, onboarding, empty state

**Files:**
- Create: `radar-app/src/components/{card,row,skeleton}.rs`, `radar-app/src/views/{briefing,onboarding,empty_state}.rs`
- Reference: `src/views/BriefingView.jsx`, `src/components/{Card,OnboardingTutorial}.jsx`, `src/views/EmptyState.jsx`

**Interfaces:**
- Consumes: `AppModel`, `text::greeting`, `text::relative_time_ms`.
- Produces:
  - `card::Card::new(article: &Article, opts: CardOpts { show_accent, use_serif, read_only: bool })`. Clicks emit `CardEvent::{ToggleRead, ToggleBookmark, Open}`. `Open` calls `cx.open_url(&article.url)` and marks the article read, matching the JSX.
  - `row::Row`, the same as `Card` but for list mode.
  - `views::briefing::page_frame(title, children)`, the shared page frame that replaces the exported `viewStyles`. It is reused by Saved and Archive.
  - `VirtualList` for list mode, used when there are more than 50 articles.

- [ ] **Step 1: Write failing tests:**
  - `briefing_shows_greeting_with_display_name`.
  - `stats_row_values`: Fetched, Unread, Saved and Last run match the model.
  - `category_chips_filter_articles`.
  - `grid_vs_list_follows_prefs`.
  - `scanning_shows_skeletons`.
  - `errors_banner_lists_source_errors`.
  - `onboarding_shown_when_no_sources_and_hydrated` and `onboarding_hidden_before_hydrated`. Hydrated means `ready && (no account || all five resources pulled)`, per `App.jsx:388-396`.
  - `card_toggle_read_calls_model_and_queues_patch_when_signed_in`.
  - `card::tests::renders_degraded_article` (Review Focus 5): `image_url` None shows the placeholder, a 600-char title truncates without overflowing the card width, and an empty summary leaves no blank gap.
  - `card_broken_image_uses_fallback`: an `img(...)` fallback is wired in through `.with_fallback`.
- [ ] **Step 2: Run `cargo test -p radar-app briefing card`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Port the inline style objects from the JSX 1:1 into GPUI builder chains. Serif headlines use Newsreader at 19 px / weight 500 when `use_serif`, otherwise Inter at 17 px.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Visual check** against the handoff prototype, in grid and list, light and dark, cozy and compact.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): briefing view, cards, onboarding"`.

### Task 10: Saved and Archive views

**Files:**
- Create: `radar-app/src/views/{saved,archive}.rs`
- Reference: `src/views/{SavedView,ArchiveView}.jsx`

**Interfaces:**
- Consumes: `page_frame`, `Card` with `read_only: true` for archives.
- Produces: `ArchiveView { selected_id: Option<i64> }`, a local drill-in state.

- [ ] **Step 1: Write failing tests:**
  - `saved_lists_only_bookmarked`.
  - `archive_list_newest_first_with_counts`.
  - `archive_drill_in_and_back`: the "All briefings" link returns to the list.
  - `archive_cards_toggles_are_noops`.
  - `archive_empty_state_copy`: the copy reads "Your archive begins with your first briefing".
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Port `formatRunAt` from `ArchiveView.jsx` into `text.rs` as `format_run_at(&str) -> String`, with its own unit test.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(app): saved and archive views"`.

### Task 11: Sources and Categories views

**Files:**
- Create: `radar-app/src/views/{sources,categories}.rs`
- Reference: `src/views/{SourcesView,CategoriesView}.jsx`

**Interfaces:**
- Consumes:
  - `RadarModel::{add_source, update_source, delete_source, toggle_source, add_category, update_category, delete_category}`
  - gpui-kit `Input`, `Select`, `Switch`
- Produces: `SourcesView { editing: Option<Editing>, draft: SourceDraft }`, and the matching `CategoriesView`. The `SourcesIntent::Add` from the workspace opens the add row and is then cleared.

- [ ] **Step 1: Write failing tests:**
  - `add_source_row_validates_url`: `ftp://x` and `nope` are rejected with inline error text. `https://a.b/feed` is accepted. This mirrors the JS rule that the URL must parse as `http(s)://`.
  - `add_disabled_without_categories`, which shows the tooltip "Add a category first".
  - `edit_row_saves_patch`.
  - `default_source_has_no_delete_button`.
  - `last_fetch_shows_relative_or_never`.
  - `health_dot_colors`.
  - `category_add_slugifies_and_appears_in_source_select`.
  - `category_delete_blocked_shows_count_tooltip`: the tooltip reads `"{n} source(s) use this category"`.
  - `category_accent_swatches`.
  - `sources_intent_add_opens_row_once`.
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(app): sources and categories views"`.

### Task 12: Settings view, account sign-in, sync indicator

**Files:**
- Create: `radar-app/src/views/settings.rs`, `radar-app/src/components/sync_indicator.rs`
- Modify: `radar-app/src/app_model.rs` (sign-in/out, sync lifecycle)
- Reference: `src/views/SettingsView.jsx`, `src/components/SyncIndicator.jsx`, `src/hooks/useSyncStatus.js`

**Interfaces:**
- Consumes: `auth::{sign_in, Session}`, `SyncClient`, `SyncedResource<*>`, `article_states`, `keychain` API-key fns, `platform::autostart` and `platform::updater` (stubs here, filled in by Tasks 14 and 15).
- Produces, on `AppModel`:
  - `sign_in(cx)` runs `auth::sign_in` on the runtime with `open = cx.open_url` and a 5-minute timeout, then calls `start_sync(cx)`. Errors show as a gpui-kit `Notification` and the state returns to signed-out.
  - `sign_out(cx)` resets every `SyncedResource` (`reset()`) and drops the client.
  - `start_sync(cx)`:
    1. Pulls the five resources plus article states, in the order of `App.jsx` effects.
    2. Marks `pulled`.
    3. Spawns a push loop. On any model change it debounces 800 ms, then for each resource with `needs_push` calls `push`, applying a merged value when one is returned.
    4. Spawns a drain loop. It runs on enqueue, every 30 s while `pending > 0`, and on window activation.
  - `flush_now(cx)` pushes immediately, skipping the debounce. It is called from the window-close callback and from `cx.on_app_quit`. This replaces `flushRegistry.js`.
  - `SettingsView` sections, in order, with the exact titles and subtitles from the JSX:
    1. Account (sign in button / avatar + email + Sign out)
    2. Profile (name, role)
    3. Briefing (run at launch, daily schedule toggle + `TimeField`, open at login)
    4. Curation (Anthropic API key control with `sk-ant-…` placeholder and preview / Replace / Clear; lens textarea with its placeholder)
    5. Appearance
    6. Updates
    7. Integrations ("Gmail — newsletter inbox", disabled; "Not yet shipped")
  - "Staleness threshold" still displays `—` / "Not wired".

- [ ] **Step 1: Write failing tests:**
  - `api_key_save_shows_preview_never_raw`. It uses `MemKeyStore` from Task 4.
  - `profile_edit_persists_and_marks_profile_dirty`.
  - `schedule_toggle_updates_prefs`.
  - `sign_in_failure_returns_to_signed_out`, with `sign_in` injected to return `Timeout`.
  - `sign_out_resets_resources_and_hides_account`.
  - `account_switch_a_to_b_does_not_leak_server_key`: after a reset, `server_key` and `etag` are None.
  - `sync_indicator_states`: pending > 0 shows "Syncing…"/queued, `last_error` shows the error tooltip, and idle shows the last success time. Match the copy in `SyncIndicator.jsx`.
  - `hydrated_waits_for_all_pulls`.
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Manual end-to-end check:**
  1. `docker compose up -d` and `cargo run -p radar-server`.
  2. Run the app and sign in through the browser.
  3. Add a category, then confirm a row in `user_categories`.
  4. Stop the server, edit a source, and watch the indicator show queued.
  5. Restart the server and watch the queue drain.
  6. Sign in on a second OS user or machine and see the data.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): settings, Clerk sign-in, live sync"`.

### Task 13: Search palette

**Files:**
- Create: `radar-app/src/components/search_palette.rs`
- Reference: `src/components/SearchPalette.jsx`

**Interfaces:**
- Consumes: `search::search`, gpui-kit `Command` + `CommandState`, opened through `window.open_dialog`.
- Produces: `SearchPalette::open(window, cx)`. Choosing an article opens its URL. Choosing a source sets `view = Sources`.

- [ ] **Step 1: Write failing tests:**
  - `palette_filters_case_insensitive`, matching title, source, summary, name, category and feed URL.
  - `palette_enter_opens_article_url`: the `open_url` call is captured.
  - `palette_source_navigates`.
  - `palette_esc_closes`.
  - `palette_arrow_keys_move_selection`.
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(app): search palette"`.

### Task 14: Briefing runner, scheduler, tray, notifications, schedule-wake, run-at-launch, autostart

**Files:**
- Create: `radar-app/src/scheduler.rs`, `radar-app/src/platform/{tray,notify,autostart}.rs`
- Modify: `radar-app/src/app_model.rs`, `radar-app/src/main.rs`

**Interfaces:**
- Consumes: `ingest::run_briefing`, `schedule::should_fire`, `schedule_wake::sync_schedule_wake`, `text::{tray_tooltip, tray_title}`.
- Produces:
  - `AppModel::run_briefing(&mut self, cx)` is the single path for the header button, tray, run-at-launch and the scheduler. It does nothing if `scanning`. It calls `begin_briefing`, then `on_rt(run_briefing(..))`, then `finish_briefing`, then a notification when `new_count > 0` and no Radar window is active, then a tray refresh. It works with zero windows open.
  - `scheduler::spawn(cx)` is a `cx.spawn` loop: a 5 s warmup, then every 30 s it checks `should_fire(prefs, last_run_at, Local::now())` and calls `run_briefing`. `src-tauri/src/scheduler.rs`'s separate store-writing path is gone, because the model persists itself.
  - `tray::init(cx)` uses `tray-icon`:
    - Icon `assets/icons/64x64.png`; template mode on macOS.
    - Menu: "Show Radar", "Run briefing now", separator, "Quit Radar".
    - Left-click toggles: if a Radar window is active, close it; otherwise call `open_main_window`.
    - Events forward through `TrayIconEvent::set_event_handler` and `MenuEvent::set_event_handler` into an `async_channel`, which a `cx.spawn` loop consumes.
    - `tray::set_status(title: Option<String>, tooltip: String)` is called on every model change that touches articles or archives.
    - On Linux, tray creation runs on a dedicated GTK thread. It needs to compile, but isn't release-tested, which matches the current Linux status.
  - `notify::briefing_ready(cx, outcome)` calls `cx.show_system_notification(SystemNotification { tag: "briefing", title: "Radar briefing ready", body, actions: vec![] })`. Clicking it calls `open_main_window`.
  - `autostart::{is_enabled() -> bool, set(enabled) -> Result<()>}` uses `auto-launch`: app name "Radar", args `["--autostart"]`, LaunchAgent on macOS.
  - Changes to schedule prefs call `sync_schedule_wake` once `ready` is set, just as the effect at `App.jsx:431-441` did.
  - Run-at-launch fires once after `ready` if `prefs.run_at_launch` and there is a usable source.

- [ ] **Step 1: Write failing tests:**
  - `app_model::tests::briefing_completes_with_no_window` (Review Focus 2): no window is open, the briefing runner is injected with a fixture response, and after completion the model has the articles, the store file has them, and the notify hook was called once.
  - `run_briefing_ignored_while_scanning`.
  - `notification_suppressed_when_window_active`.
  - `run_at_launch_fires_once_and_only_with_usable_source`.
  - `scheduler_fires_when_due`: inject a clock, `should_fire` returns true, and the runner is called once. On the next tick `last_run` is after the slot, so it isn't called again.
  - `schedule_pref_change_calls_wake_sync_after_ready_only`, with `sync_schedule_wake` injected.
  - `tray_status_updates_on_toggle_read`, with a fake tray sink.
- [ ] **Step 2: Run `cargo test -p radar-app`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Inject the briefing runner, clock, notifier, tray sink and wake syncer as boxed fns on `AppModel`, with real defaults wired in `main.rs`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Manual check on Windows and macOS:**
  - The tray icon appears and its menu works.
  - Close the window, then click "Run briefing now" in the tray. The notification appears; clicking it reopens the window with the results.
  - Set the schedule for 2 minutes out with the window closed. The briefing fires, and on Windows `schtasks /Query /TN com.radar.scheduler` shows the task.
  - Turn on open at login, reboot, and the app starts hidden.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): briefing runner, scheduler, tray, notifications, autostart"`.

### Task 15: Updater, packaging, release workflow, versioning

**Files:**
- Create: `radar-app/src/platform/updater.rs`, `scripts/make-latest-json.sh`
- Modify: `radar-app/Cargo.toml` (`[package.metadata.packager]`), `.github/workflows/release.yml`, `radar-app/src/views/settings.rs` (Updates section)

**Interfaces:**
- Produces:
  - `updater::Updater` with a state enum `Idle | Checking | Available{version, notes} | Downloading{progress: Option<f32>} | Ready | UpToDate | Error(String)`. This is a port of the `useUpdater` state machine.
  - `check()`, `download_and_install()` and `restart()` use `cargo-packager-updater`, with the endpoint `https://github.com/alamanija/radar/releases/latest/download/latest.json` and the **existing** minisign pubkey from `src-tauri/tauri.conf.json > plugins.updater.pubkey`.
  - One silent check on launch.
  - Packager config:
    - `identifier = "com.radar.dev"`, `product-name = "Radar"`.
    - Formats `nsis` (Windows) and `app` + `dmg` (macOS arm64).
    - The same icon set.
    - Updater artifacts signed through `CARGO_PACKAGER_SIGN_PRIVATE_KEY` + `CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD`, which are fed from the existing `TAURI_SIGNING_*` secrets.
  - Version source of truth: the `[workspace.package] version` in the root `Cargo.toml`, inherited by `radar-app`. Release flow: `cargo set-version --bump patch` (cargo-edit), commit, `git tag vX.Y.Z`, then push the tag.

- [ ] **Step 1: Write failing tests:**
  - `updater_state_transitions`: mocked manifest via httpmock, newer version → `Available`, same version → `UpToDate`, HTTP 500 → `Error`.
  - `latest_json_shape`: run `scripts/make-latest-json.sh` on fixture artifact names. The output has `version`, `notes`, `pub_date`, and `platforms["windows-x86_64"|"darwin-aarch64"].{signature,url}`. That is the shape both the Tauri updater and cargo-packager-updater read, so already-installed Tauri builds can upgrade into the new app.
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Release workflow: a macos-latest + windows-latest matrix runs `cargo install cargo-packager --locked`, then `cargo packager -p radar-app --release`, then uploads the artifacts to a draft release. A final job builds `latest.json` and publishes the release, keeping the existing publish step.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Manual migration check before the first real release.** Install the current Tauri v0.2.22 build, point its endpoint at a test release made by this workflow, and run "Check for updates". It must download and install the GPUI build, and the GPUI build must open with the user's existing data intact (Review Focus 1). If the Tauri updater can't apply the cargo-packager artifact on a platform, record that and ship a final Tauri release whose notes link to the manual installer.
- [ ] **Step 6: Commit** with `git commit -m "feat(release): cargo-packager build, updater, release workflow"`.

### Task 16: Cutover cleanup and docs

**Files:**
- Delete: `src-tauri/`, `src/`, `index.html`, `vite.config.*`, `package.json`, `package-lock.json`, `.nvmrc`, `scripts/sync-version.mjs`, `node_modules/` (untracked)
- Modify: `.env.example` (now `RADAR_SERVER_URL`, `RADAR_CLERK_FRONTEND_API`, `RADAR_CLERK_CLIENT_ID`), `.gitignore`, `CLAUDE.md`, `radar-server/README.md`
- Keep: `radar-handoff/`, the visual spec, still read-only.

- [ ] **Step 1: Make sure nothing still references the deleted files.** `rg -n "tauri|@clerk|vite|npm run" --glob '!radar-handoff/**' --glob '!docs/**'` should return only intended mentions (history notes in CLAUDE.md).
- [ ] **Step 2: Rewrite `CLAUDE.md`** for the new layout:
  - Crates and their modules.
  - Commands: `cargo run -p radar-app`, `cargo test --workspace`, `cargo packager -p radar-app --release`.
  - State in `AppModel`.
  - Store compatibility.
  - The OAuth flow plus the Clerk dashboard setup.
  - The sync design (unchanged semantics, new file paths).
  - Tray, scheduler, logging paths.
  - The release flow with `cargo set-version`.
  - Known gaps: Linux tray untested, WebView unused.
- [ ] **Step 3: Run the full verification.** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` should all pass.
- [ ] **Step 4: Commit** with `git commit -m "chore: remove Tauri/React frontend, update docs"`.
