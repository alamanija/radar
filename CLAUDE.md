# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Radar is a Tauri v2 desktop app for a personalized design-news briefing. The UI was ported from an HTML/CSS/JSX prototype in `radar-handoff/` (a Claude Design export); it currently runs against in-memory mock data in `src/data.js`. There is no real ingestion, persistence, or AI pipeline yet — only UI state + `localStorage` for user preferences.

## Layout

**Cargo workspace** (root `Cargo.toml`, members: `src-tauri`, `radar-server`). Shared `target/` at repo root.

- `src/` — React + Vite frontend (JS, not TS).
  - `main.jsx`, `App.jsx`, `styles.css`, `data.js`, `time.js`, `external.js`, `storage.js`, `sync.js`
  - `components/` — `Icon`, `Sidebar`, `Header`, `Card`, `Toggle`, `Tweaks`, `SearchPalette`
  - `hooks/` — `useSyncedResource`
  - `views/` — `BriefingView`, `SavedView`, `ArchiveView`, `SourcesView`, `CategoriesView`, `SettingsView`, `EmptyState`
- `src-tauri/` — Tauri 2 desktop app, now purely local: feed ingestion + Claude summarization + Anthropic-key keychain. Modules: `ingest`, `summarize`, `keychain`. No auth/sync code — the frontend calls `radar-server` directly.
- `radar-server/` — Axum sync backend (Postgres via sqlx, Clerk JWT verification). Wire shapes live in `src/types.rs`. See `radar-server/README.md` for setup.
- `radar-handoff/` — original prototype. Read-only source of visual truth.
- `docker-compose.yml` — Postgres 16 on `localhost:5433` for dev.
- `dist/` (generated) — Vite production build; Tauri consumes it as `frontendDist`.

## Commands

Frontend only (no desktop window):
- `npm install` — bootstrap
- `npm run dev` — Vite dev server at http://localhost:1420
- `npm run build` — produces `dist/`

Tauri (desktop):
- `npm run tauri:dev` — launches the desktop app; Tauri starts `npm run dev` automatically per `tauri.conf.json`
- `npm run tauri:build` — release bundle (requires real icons; see note below)

Rust-only:
- `cargo check` / `cargo build` — run from `src-tauri/`

## Architecture notes

- **State lives in `App.jsx`.** All mutable state (view, theme, density, accent, articles, sources, tweaks) is owned there and threaded down as props. No context, no global store.
- **Persistence lives in `src/storage.js`** — thin async wrapper that uses `@tauri-apps/plugin-store` (file `radar.json`, 100ms autoSave) under Tauri, `localStorage` under plain Vite dev. Four independent keys: `prefs`, `sources`, `articles` (the current live briefing), and `archives` (past briefing snapshots). On mount, `App.jsx` hydrates all four; saves are gated behind a `ready` flag so defaults never clobber the persisted copy during hydration.
- **Archive** is an append-only list of `{id, runAt, articles, errors}` snapshots, pushed by `App.archiveBriefing()` at the end of each non-empty briefing. Capped at 90 (to match the UI's "90 days" copy). `ArchiveView` owns its list/drill-in state locally — `selectedId` picks one snapshot; the Card component is reused with no-op toggle handlers for read-only display. The current-live `articles` state and `archives[0]` diverge once the user toggles read/bookmark on live articles — by design, the archive is "what the briefing surfaced," not "what was read".
- **Profile + run-at-launch.** A separate `profile` storage key holds `{name, role, lens}`. `profile.name` drives the briefing greeting and the sidebar footer avatar/name; `profile.role` is the sidebar tagline; `profile.lens` is a free-text "professional lens" that's forwarded to `ingest_briefing` as the optional `lens: Option<String>` and injected into Claude's system prompt as a "Reader context" block when non-empty. `runAtLaunch` (in prefs) fires `onBriefing()` once after hydration if enabled and there's at least one usable source. A `launchFired` flag prevents repeat auto-fire if the user toggles the setting mid-session.
- **Daily schedule.** `scheduleEnabled` + `scheduleTime` (`HH:MM` local) in prefs. Two cooperating layers:
    - **In-process scheduler** (`src-tauri/src/scheduler.rs`): a `tokio` loop spawned from `lib.rs::run().setup()` that polls the Tauri store every 30s (after a 5s warmup so the webview can attach its event listener). When `now >= today's slot` and `archives[0].runAt < today's slot`, it calls `ingest::run_briefing` (the library-level function that `ingest_briefing` command wraps), writes `articles` + prepends an `archives` snapshot to the store, emits `briefing://completed` with the full response, and fires a `NotificationExt` banner. The webview listener in `App.jsx` overlays per-article read/bookmarked state, reloads `archives` from the store, and stamps per-source health — matching `onBriefing`'s manual path.
    - **OS relauncher** (`src-tauri/src/schedule_wake.rs`): on `scheduleEnabled` / `scheduleTime` change the frontend calls `sync_schedule_wake(enabled, hour, minute)`. Platform-dispatched inside Rust:
        - macOS → `~/Library/LaunchAgents/com.radar.scheduler.plist` with `StartCalendarInterval` + `ProgramArguments: [<current_exe>, "--autostart"]`, `launchctl load/unload -w`.
        - Windows → `schtasks /Create /SC DAILY /TN com.radar.scheduler /TR "<exe> --autostart" /ST HH:MM /F` (and `/Delete /F` on disable).
        - Linux → `~/.config/systemd/user/com.radar.scheduler.{service,timer}` with `OnCalendar=*-*-* HH:MM:00`, registered via `systemctl --user enable --now`.
      When the slot fires, the OS launches Radar with `--autostart` (window hidden); the in-process scheduler sees the slot is past and fires immediately. If Radar is already running, the OS scheduler skips the wake (or spawns a second short-lived process that exits without doing harm) — the running scheduler handles the briefing. All three paths are idempotent: identical on-disk config → no-op.
  - `runAtLaunch` stays frontend-only (it's an on-mount trigger, orthogonal to time-based scheduling).
- **Known-unfinished stubs.** "Staleness threshold" in Settings is displayed as `—` / "Not wired" because we don't cache articles across briefings. "Gmail — newsletter inbox" in Integrations is a disabled button — full OAuth flow + Gmail API + HTML-to-article parsing is a separate project, not shipped.

## Cross-device sync

Auth is Clerk. Profile, categories, sources, archives, article states, and prefs all sync end-to-end via the frontend calling `radar-server` directly — the Tauri process is no longer involved in auth or sync.

- **Auth (Clerk).** `main.jsx` wraps `<App />` in `<ClerkProvider publishableKey={VITE_CLERK_PUBLISHABLE_KEY} />`. In `App.jsx`, `useAuth()`/`useUser()` give `{isSignedIn, getToken, user}`; `account` is a thin derived object (`{id: clerk.sub, email, name}`) still threaded through components that used to take `UserInfo`. Sign-in is `SignedOut` + `<SignInButton mode="modal">` in `SettingsView`; signed-in users get Clerk's `<UserButton />`. There's no server-owned session, no opaque bearer token, no keychain session row — Clerk's session JWT is the only credential, held by `clerk-js` in the webview and refreshed automatically.
- **Server-side verification (radar-server/src/clerk.rs).** `session_layer` pulls the Clerk JWT from `Authorization: Bearer`, verifies against Clerk's JWKS (cached in `jwks.rs`, 1-hour TTL, generic over URL), and requires `alg: RS256` + `iss == CLERK_ISSUER`. `aud` is not checked because Clerk's default session template doesn't set one. The `sub` claim becomes `ClerkUser { id: String }` on request extensions; every handler extracts it via the `FromRequestParts` impl. No users table, no sessions table — all sync rows are keyed directly on `clerk_user_id text`.
- **Direct frontend → server HTTP (src/sync.js).** `makeSyncFetch(getToken)` returns a `fetch`-wrapper that attaches `Authorization: Bearer ${await getToken()}`. Base URL is `VITE_RADAR_SERVER_URL` (defaults to `http://127.0.0.1:8787`). GETs go straight through; mutations (PUT/PATCH/DELETE) route through `queuedMutation` so transient failures end up in the persistent retry queue. `radar-server` uses an env-driven CORS allow-list (`CORS_ORIGINS`, comma-separated; defaults to `http://localhost:1420,tauri://localhost`) and exposes the `ETag` response header to the webview.
- **Server hardening.** Every `/sync/*` handler runs inside a DB transaction with `FOR UPDATE` row-locking (prevents races between the If-Match read and the upsert). Every PUT handler validates shape and size limits (per-resource caps in each module — categories ≤64, sources ≤512, archives ≤90, 8 MiB archive payload, 64 KiB prefs). A per-clerk-user in-memory token bucket (`rate_limit.rs`, 40 capacity / 10 rps) sits in front of the sync routes; a `RequestBodyLimitLayer` caps incoming bodies at 10 MiB before handlers touch them.
- **Resource sync** follows a shared pattern lifted into `src/hooks/useSyncedResource.js`: pull-on-sign-in, debounced (800ms) push-on-change. Callers pass the endpoint + a `buildPushBody` transformer + a `fromRemote` merge function; the hook owns fetch, headers, ETag tracking, conflict retry, and flush-on-hide. Two guards prevent the sign-in race — `pulled` gates pushes until the pull completes, and a per-resource `serverKeyRef` holds the last-known server shape so no-op echoes are dropped. If the server has no row yet but local is non-empty, the pull path seeds the server from local instead of overwriting local with nothing. Account A→B transitions reset both guards so a stale key from A can't leak into B's session.
- **Conflict detection.** Every GET response carries `updated_at` (stored in `etagRef`) and sets an `ETag` header. Every PUT sends `If-Match: W/"<updatedAt>"`. On mismatch the server returns 412 Precondition Failed; the client refetches, runs `fromRemote` to merge, updates the etag, and stops. No silent overwrites.
- **Offline queue** lives in `src/syncQueue.js`. Every mutation routes through `queuedMutation`: on network failure / 5xx / 408 / 429 the request persists to the Tauri store (`sync-queue` key) and the caller sees a synthetic 202 so the UI doesn't bounce. Drains on `navigator.onLine → true`, on `installSyncQueue`, and when a fresh enqueue arrives. Dedup by `groupKey` (PUTs dedup per endpoint; article-state PATCHes dedup per article id). When a queued push eventually lands, `onDrainSuccess` listeners (registered by `useSyncedResource`) update the etag/serverKey so the client doesn't send stale If-Match headers.
- **Flush on hide.** `src/flushRegistry.js` fires registered flushers on `beforeunload`, `pagehide`, and `visibilitychange → hidden`. Each `useSyncedResource` push effect registers a flusher that skips the remaining debounce and pushes now (into the queue if offline). A quick edit + window-close no longer drops the write.
- **Profile sync (radar-server `/sync/profile`).** `profiles` table keyed on `clerk_user_id text`; columns are `name/role/lens` (nullable) + server-stamped `updated_at`. PUT upserts; GET returns the row or 404.
- **Categories sync (radar-server `/sync/categories`).** `user_categories` is one row per user holding a `jsonb` categories array + server-stamped `updated_at` — full-list replace semantics (small, bounded list; no per-row merge). The frontend treats remote and local as the same shape so `stableKey` is just `JSON.stringify`.
- **Sources sync (radar-server `/sync/sources`).** Same jsonb-per-user / full-list-replace shape as categories. Only user-editable fields travel — `{id, name, category, feedUrl, enabled, isDefault}` — because `lastFetchAt` and `health` are per-device observations, not cross-device truth. In `App.jsx`, `fromRemote` merges this device's prior `lastFetchAt`/`health` back onto remote rows by source id on pull; new sources from remote default to `health: 'ok', lastFetchAt: null`. `stableKey` and `invokePush` run through a shared `sourceWire` picker so `onBriefing` bumping `lastFetchAt` doesn't register as a dirty diff.
- **Archives sync (radar-server `/sync/archives`).** `user_archives` is jsonb-per-user. Archive contents round-trip as opaque `serde_json::Value` end-to-end — the server doesn't interpret them. Unlike the other list resources, `fromRemote` unions remote with prev (dedup by archive id, sorted by `runAt` desc, capped at 90) instead of overwriting, so signing in on device A after device B briefed doesn't lose A's local archives. Archives are immutable once created, so remote-wins on id collision is harmless.
- **Prefs sync (radar-server `/sync/prefs`).** `user_prefs` is jsonb-per-user with full-replace semantics; contents are opaque to the server. The synced subset is visual taste (`theme`, `viewMode`, `density`, `accent`, `useSerif`, `showAccent`) plus briefing behavior (`runAtLaunch`, `scheduleEnabled`, `scheduleTime`). Per-device UI state — `sidebarCollapsed` and the current `view` — is deliberately excluded and stays in local storage. The frontend composes the synced payload on the fly from the existing individual state slots (`syncedPrefs`/`setSyncedPrefs` in `App.jsx`), so the local `prefs` storage key continues to hold the full set for offline use. `fromRemote` merges over `DEFAULT_PREFS` so a missing key on an older server row falls through to the default rather than becoming undefined.
- **Article states sync (radar-server `/sync/article-states`).** Per-row table keyed on `(clerk_user_id, article_id)`; this is the one resource that doesn't ride `useSyncedResource` because full-list round-trips would be wasteful for rapid toggles. `articleStatesRef` (a plain `useRef(new Map())`) is the client-side cache. On sign-in, the effect pulls all rows, overlays them onto the current `articles`, then seeds any local-only `read`/`bookmarked` flags to the server (server-side rows win on conflict — more-recent cross-device edits aren't regressed). `toggleRead`/`toggleBookmark` update the ref and fire a PATCH individually (no debouncing for this slice — one PATCH per click). `applyArticleStates` re-overlays the ref whenever articles get rebuilt (briefing completion, mock path). Archives deliberately don't get the overlay — they're "what the briefing surfaced," not "what was read".
- **To boot the sync stack locally:** create a Clerk app, copy `.env.example` → `.env` at the repo root (fill in `VITE_CLERK_PUBLISHABLE_KEY`) and `radar-server/.env.example` → `radar-server/.env` (fill in `CLERK_JWKS_URL` + `CLERK_ISSUER`) → `docker compose up -d` (Postgres) → `cargo run -p radar-server` (migrations run at startup) → `npm run tauri:dev`. Full setup in `radar-server/README.md`.
- **Source health.** Each source carries `lastFetchAt: number | null` (ms epoch) and `health: 'ok' | 'warn' | 'stale'`. On briefing completion, `App.onBriefing()` stamps `ok + now` for every attempted (enabled + feed-URL'd) source, and `warn` for any source found in the Rust `errors` array (ignoring the synthetic `sourceId: 0` used for Claude-summarization failures). `SourcesView` renders via `relativeTime(lastFetchAt)` from `src/time.js`, or "never" when null.
- **Theming is CSS-variable-driven.** `styles.css` defines tokens under `:root` and `[data-theme="dark"]`; `App.jsx` sets `data-theme` and `data-density` on `<html>` and writes `--accent*` vars inline when the accent swatch changes.
- **The "Tweaks" panel** is a floating dev/designer control surface. In the prototype it was opened via a postMessage from Claude Design's iframe host (`__activate_edit_mode`); that protocol is stripped. A small FAB in `App.jsx` toggles it instead.
- **Ingestion** lives in `src-tauri/src/ingest.rs`. Exposes a single async command `ingest_briefing(sources: Vec<SourceInput>) -> BriefingResponse`, fetches feeds in parallel via `reqwest` (rustls), parses via `feed-rs`, maps into `Article` (camelCase-serialized to match frontend shape). Per-source errors are collected, not fatal. Articles are sorted newest-first by `publishedAt`.
- **Claude summarization** lives in `src-tauri/src/summarize.rs`. If the keychain holds an Anthropic API key AND the user has at least one category defined, `ingest_briefing` fans out one `claude-opus-4-7` request per article (raw HTTP via `reqwest` — Rust has no official Anthropic SDK). `summarize()` takes a `&[(id, description)]` slice; the prompt lists those, and the JSON schema's `enum` is built from them at request time. No prompt caching — the ~400-token system prompt is under Opus 4.7's 4096-token cache minimum. Claude failures are non-fatal; the raw feed excerpt survives and the failure count shows up in the response's `errors` array under a synthetic "Claude (summarization)" source.
- **Categories are user-owned.** `src/data.js` seeds 6 starter categories (packaging/branding/tools/campaigns/illustration/industry); the user can add/rename/delete at will via `CategoriesView`. Each category has `{id, label, description, accent}`. The description is what Claude reads to decide placement — editing a description changes the model's behavior. Sources reference categories by **id** (not label); stale ids fall through to an "Uncategorized" bucket in Rust's `resolve()`. A one-time migration in `App.jsx` hydration converts any legacy label-based `source.category` strings to ids. Deleting a category is blocked in the UI while any source still points at it.
- **API key storage** lives in `src-tauri/src/keychain.rs`. Uses the `keyring` crate under service `com.radar.dev`, username `anthropic`. Commands: `set_anthropic_api_key`, `clear_anthropic_api_key`, `anthropic_api_key_status` (returns `{present, preview}` — never the raw key). `keychain::read_api_key()` is the internal reader used by `ingest_briefing`.
- **Frontend → Rust bridge.** `App.onBriefing` calls `invoke('ingest_briefing', { sources })` under Tauri. Under plain Vite dev (`npm run dev` without Tauri), it short-circuits to the mock in `RADAR_DATA.articles` — `isTauri()` from `@tauri-apps/api/core` is the discriminator.
- **Source management.** `data.js` only seeds the first-run sources; after that the user owns the list via the Sources view (add/edit/delete). Defaults (`isDefault: true`) can be edited but not deleted; custom ones can be both. CRUD mutations (`addSource`/`updateSource`/`deleteSource`) live in `App.jsx`; persistence falls through the existing `sources` key. `SourcesView` uses inline editing — `Add source` reveals an edit row at the top; clicking the ⋮ on a source swaps that row into edit mode. Feed URL validation is minimal (must parse as `http(s)://…`). Sources with a null `feedUrl` are silently skipped by the Rust ingest command.
- **Capabilities.** `capabilities/default.json` grants `core:default` + `store:default` (needed by plugin-store). Custom commands defined in `invoke_handler` don't require explicit permission grants in Tauri v2; only plugin commands do.
- **Search.** `SearchPalette` is a ⌘K / Ctrl+K overlay opened from either the global key listener in `App.jsx` or a click on the header search bar. It searches current `articles` + all `archives` articles (deduped by id, current state wins) + `sources`, matching case-insensitive substring across title/source/summary and name/category/feedUrl respectively. Keyboard-navigable (↑↓/Enter/Esc); clicking an article opens its URL in a new window; clicking a source navigates to the Sources view.

## When porting further designs or touching the UI

- The prototype in `radar-handoff/project/` is the visual spec. Recreate pixel-for-pixel; don't copy its UMD/Babel-standalone structure.
- Inline style objects are the prototype's idiom — keep them rather than extracting to a CSS-in-JS library.
- `viewStyles` is exported from `views/BriefingView.jsx` and reused by `SavedView` / `ArchiveView` for the shared page frame. Keep that share-point if you add new views.

## Releases and updates

Signed Tauri updater over GitHub Releases. Tagging `vX.Y.Z` on `main` triggers `.github/workflows/release.yml`, which builds for macOS (arm64 + x86_64), Windows (NSIS), and Linux (AppImage), signs every artifact with the Ed25519 key, and publishes a GitHub Release with a `latest.json` manifest. The client checks `https://github.com/USER/radar/releases/latest/download/latest.json` on launch + on demand via `useUpdater`; downloads are verified against the public key in `tauri.conf.json > plugins.updater.pubkey` before install. `createUpdaterArtifacts: true` in `bundle` gates updater-artifact generation off the signing secrets being present.

**One-time setup before the first real release:**
1. `npm run tauri signer generate -- -w ~/.tauri/radar.key` — generates keypair. Pick a strong password.
2. Paste the printed public key into `src-tauri/tauri.conf.json > plugins.updater.pubkey` (replace `REPLACE_WITH_TAURI_SIGNING_PUBLIC_KEY`).
3. Edit the `endpoints` entry in the same block so the `USER/radar` path matches the actual GitHub owner/repo.
4. Add repo Actions secrets: `TAURI_SIGNING_PRIVATE_KEY` (contents of `~/.tauri/radar.key`), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit the private key.
5. (macOS only, optional but recommended) Add Apple-signing secrets (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`) so notarized builds don't trip Gatekeeper.

**Shipping a release:** `package.json` is the single source of truth for the app version. `tauri.conf.json` reads it via `"version": "../package.json"` (Tauri 2 resolves JSON/TOML paths in that field). `src-tauri/Cargo.toml` is mirrored automatically by `scripts/sync-version.mjs`, which runs in the `version` npm lifecycle. Use `npm version`:

```bash
npm version patch          # 0.2.4 → 0.2.5 (bug fix)
npm version minor          # 0.2.4 → 0.3.0 (feature)
npm version major          # 0.2.4 → 1.0.0 (breaking)
git push && git push --tags
```

`npm version` bumps `package.json`, runs the sync script (which updates + git-adds `src-tauri/Cargo.toml`), then commits and tags in one step. Pushing the tag triggers `.github/workflows/release.yml`. **Never edit the Cargo.toml or tauri.conf.json version fields by hand** — they'll either drift from package.json (Cargo) or fail to parse (Tauri's path string).

**Rolling back:** GitHub Releases → edit the bad release → change it back to draft (or delete it). The `releases/latest/download/latest.json` URL now resolves to the prior release, and clients that haven't yet downloaded will see the older version on their next check. Clients that already downloaded + applied the bad version need a fresh fix-forward release.

**Client-side schema migrations** belong in `src/storage.js`. The store currently stamps `version: 1` on every boot. Before changing a local data shape, read `version`, transform old → new in-place, then write the new version. Never blindly bump — the field exists specifically so you have something to branch on.

## Menu-bar presence

Radar is a hybrid dock + menu-bar app on macOS (tray on Windows/Linux). Wiring lives in `src-tauri/src/tray.rs`:

- **Tray icon** — embedded at compile time from `icons/64x64.png` via `include_bytes!` (so AppKit doesn't resize the 512×512 app icon every render). `icon_as_template(true)` uses the alpha channel as a monochrome mask; good enough for now, a dedicated monochrome design would be crisper.
- **Dynamic menu** — rebuilt whenever state changes. Top section is up to 5 recent article titles (click opens the URL via `tauri-plugin-opener`), middle is "Show/Hide Radar" (label tracks actual `is_visible()`) and "Run briefing now" (emits `tray://run-briefing`, frontend listens and fires `onBriefing`), bottom is "Quit Radar". Article URLs live in `TrayState` (`Mutex<Vec<ArticleLink>>` in `AppHandle` state); menu items have ids `tray:article:<index>` so the menu-event callback can look a URL up by index.
- **Close-to-tray** — `lib.rs` intercepts `WindowEvent::CloseRequested`, calls `prevent_close()`, hides the window, and calls `tray::on_window_hidden` so the Show/Hide label flips. Cmd+Q and tray → Quit still `app.exit(0)` normally.
- **Reopen handling** — `RunEvent::Reopen` (fired by macOS on notification-click / dock-click / activation) surfaces the hidden window. Keeps notification banners clickable without extra action wiring.
- **Status push** — `App.jsx` effects call `set_tray_status` (unread count + relative last-run, threads through macOS tray title + cross-platform tooltip) and `set_tray_articles` (top 5 article titles/urls) on every briefing or read-toggle.
- **Notifications** — `tauri-plugin-notification`. `src/notify.js` caches permission after first grant; banner fires from `onBriefing` when at least one new article id landed and the window is not currently focused (the rendered UI is notification enough when the user is already looking).
- **Open at login** — `tauri-plugin-autostart` with `MacosLauncher::LaunchAgent`, passing `--autostart` at OS-triggered launch. `lib.rs`'s `setup()` checks `std::env::args()` for that flag and calls `main.hide()` so Radar boots quietly to the menu bar. UI toggle is `AutostartToggle` in `SettingsView` — reads `isEnabled()` on mount, flips `enable()`/`disable()`, no React mirror (OS owns the state).

## Logging

`tauri-plugin-log` is the shared pipe. Rust code calls `log::info!`/`log::warn!`/`log::error!`; the frontend calls `logger.info/warn/error` (shim in `src/log.js`) which routes through the same plugin under Tauri and falls back to `console.*` under plain Vite. Three targets are configured in `lib.rs::run()`:

- `Stdout` — visible in the terminal during `npm run tauri:dev`.
- `LogDir` — rolling file, 5 MB cap per file. macOS: `~/Library/Logs/com.radar.dev/Radar.log`. Windows: `%LOCALAPPDATA%\com.radar.dev\logs\`. Linux: `$XDG_DATA_HOME/com.radar.dev/logs/`.
- `Webview` — forwarded to DevTools once `attachLogConsole()` runs (fires from `main.jsx` at boot).

`tao` and `wry` are clamped to `Warn` so UI event noise doesn't drown the interesting signal. The scheduler emits `log::info!("[scheduler] loop started")` on boot and `[scheduler] firing briefing …` / `briefing done: N new / M total, K errors` around each run — that's the first place to look when a scheduled slot seems to have gone silent.

## Known gaps

- Tray menu-bar icon uses the alpha mask of the color app icon. Functional, but a dedicated monochrome PNG would render crisper on light/dark menu bars.
- No first-class cross-platform testing pipeline — Windows / Linux `sync_schedule_wake` paths compile but haven't been runtime-exercised; they're reviewed by eye, not by a live system.
