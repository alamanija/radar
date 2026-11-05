# Task 7 report: `radar-app` scaffold

**Status:** DONE_WITH_CONCERNS. The concerns are minor; see the end of this report.
**Commit:** `f2f8552 feat(app): GPUI Kit scaffold, theme tokens, persisted global model`

## What was implemented

### Workspace
- `members` now includes `radar-app`.
- Added `[profile.dev.package]` with `opt-level = 3`. The list follows the GPUI Kit install guide: `gpui-pre`, `gpui-pre-platform`, `gpui-component`, `gpui-kit`, `gpui-kit-assets`, `rustybuzz`, `taffy`, `ttf-parser`.
- I also added `gpui-base`, `gpui-pre-windows`, `gpui-pre-macos`, `gpui-pre-linux` and `gpui-pre-wgpu`, because rendering and text layout live in those crates.
- Proc-macro crates are left out, as the guide says.

### `radar-app/Cargo.toml`
- Uses `version`, `edition`, `license` and `publish` from the workspace (R7).
- Dependencies: gpui-kit 0.9.0, radar-core, anyhow, dirs 5, flexi_logger 0.29, futures, log, reqwest 0.12 (rustls, same features as radar-core), serde_json.
- Dev-dependencies: gpui-kit with `test-support`, httpmock, tempfile, tokio with `time`.
- Windows-only build-dependency: `embed-resource 3`.

### `build.rs`
- On Windows it generates `OUT_DIR/radar.rc` (`1 ICON "<abs path to assets/icons/icon.ico>"`) and compiles it with embed-resource.
- GPUI's Windows backend loads the window icon from resource id 1 (`LoadImageW(PCWSTR(1))` in gpui-pre-windows `platform.rs`).
- Verified in the built exe: icon group id 1 is present, and gpui's own `RT_MANIFEST` id 1 is also present. The two do not conflict.

### `src/theme.rs`
- `Tokens` (Copy) and `impl Global for Tokens`.
- `tokens(theme, accent, density)`: values copied verbatim from `styles.css:2-57` and from `ACCENTS` in `App.jsx`. Light uses `l/sl/tl`, dark uses `d/sd/td`. Dots are ordered olive, rust, slate, plum, sand, ink.
- `t(cx) -> &Tokens`.
- `apply(cx, &Prefs)`:
  - Sets the Tokens global.
  - If the kit `Theme` global exists, switches `ThemeMode` first in its own step (switching mode reloads the registered theme colors), then calls `Theme::update`.
  - The update sets `background`, `foreground`, `border`, `primary` (= accent), `primary_foreground` (= surface), `ring`, `muted_foreground`, `popover`/`popover_foreground`, `font_family = "Inter"`, `mono_font_family = "JetBrains Mono"`, `radius = 6` and `radius_lg = 10`.
  - The kit-theme part is skipped when `gpui_kit::init` hasn't run, as in the headless model tests.
- `register_fonts(cx)` and the `FONT_SANS` / `FONT_SERIF` / `FONT_MONO` constants.

### `src/app_model.rs`
- `AppModel { model, session: Option<Session>, account, sync: Option<Arc<SyncClient>>, sync_status, ready, store_path, sync_queue, save_task, + private dirty/save_gen/written_gen }`.
- `AppModel::new(store_path, session, cx)` hydrates with `store::load`. It only reads and never writes the store. `ready` is true when it returns.
- `AppModel::init(store_path, session, cx) -> Entity` creates the entity, applies the persisted theme, sets the `GlobalAppModel` global, and registers an `on_app_quit` flush.
- `AppModel::global(cx)`.
- `update_model(cx, f) -> R`:
  - Runs `f`.
  - Re-applies theme tokens if theme, accent or density changed.
  - Calls `cx.notify()`.
  - Replaces `save_task` with a new `cx.spawn` that waits `SAVE_DEBOUNCE` (100 ms) on the background executor timer, takes a snapshot on the foreground, then calls `store::save` inside `cx.background_spawn`.
- Write ordering: each write carries a generation number. A shared `Mutex<u64>` serializes writers and drops any write older than the last one written. This means an 8 MiB archive write that finishes late can never overwrite a newer snapshot.
- `sync_queue` is loaded and passed through unchanged until Task 12.

### `src/bridge.rs`
- `on_rt(fut)` calls `radar_core::runtime().spawn(fut)` right away and awaits the JoinHandle. If the task panicked, the panic is resumed in the awaiting task.
- `ReqwestHttpClient` implements GPUI's `HttpClient`:
  - Two reqwest clients: one that doesn't follow redirects, and one limited to 10 redirects. The client is picked from the request's `RedirectPolicy` extension; GPUI's default is NoFollow.
  - Honors the `RequestTimeout` extension, with a 30 s default.
  - Buffers bodies and runs the request through `on_rt`.
  - Installed with `Application::with_http_client`.

### `src/workspace.rs`
- `Workspace` observes the AppModel and renders a full-size frame with `bg(t.bg)`, `text_color(t.text)` and `font_family("Inter")`.
- The shell itself is Task 8 (R4).

### `src/main.rs`
- Startup order:
  1. Logging.
  2. `gpui_kit::application().with_assets(gpui_kit::assets::Assets).with_http_client(..).with_quit_mode(QuitMode::Explicit).run(..)`.
  3. `gpui_kit::init`, then register fonts.
  4. `Session` from `AuthConfig::from_env()` with `OsKeyStore`, or `None`, in which case it logs "running local-only".
  5. `AppModel::init(store::default_path(), ..)`.
  6. `open_main_window` unless `--autostart` is passed.
- `pub fn open_main_window(cx)`:
  - Idempotent. It keeps a `MainWindow(Option<AnyWindowHandle>)` global. If that handle is still in `cx.windows()`, it calls `activate_window()` and `cx.activate(true)`.
  - Otherwise it opens a window with 1280×820 centered bounds, minimum 920×620, title "Radar" and `app_id` "com.radar.dev".
  - `on_window_should_close` returns `true` and logs the close.
- Release builds on Windows use `windows_subsystem = "windows"`, so no console window appears.
- Logging uses flexi_logger:
  - `RUST_LOG` overrides the default level, which is `info`.
  - Files are `Radar_rCURRENT.log`, rotated at 5 MB, with 5 rotated files kept.
  - Log directories:
    - Windows: `dirs::data_local_dir()/com.radar.dev/logs`
    - macOS: `~/Library/Logs/com.radar.dev`
    - Linux: `dirs::data_dir()/com.radar.dev/logs`
  - Output is duplicated to stdout in debug builds.

### Assets
- `assets/icons/{64x64.png,icon.png,icon.ico,icon.icns}` were copied from `src-tauri/icons`.
- The font files are described in "Fonts result" below.

## Tests and results
- `cargo test -p radar-app`: **13 passed**:
  - The 5 the brief asked for: `theme::tests::{light_camel_tokens, dark_slate_tokens, compact_density}` and `app_model::tests::{mutation_persists_after_debounce, hydrate_does_not_overwrite_store}`.
  - `app_model::tests::rapid_mutations_coalesce_into_one_later_save`: the debounce restarts on every mutation.
  - `app_model::tests::quit_flushes_pending_save`.
  - `app_model::tests::hydrate_applies_persisted_theme_tokens`: the fixture is dark/Olive/compact.
  - `app_model::tests::pref_change_reapplies_tokens`.
  - `bridge::tests::on_rt_runs_inside_tokio`: `tokio::time::sleep` inside `futures::executor::block_on`.
  - `bridge::tests::http_client_fetches_through_runtime`: uses httpmock.
  - `tests::open_main_window_is_idempotent`.
  - `tests::reopens_after_close`: `remove_window`, then `windows()` is empty, then reopening gives a new handle.
- `mutation_persists_after_debounce` checks three points: no file at 50 ms, still no file after `run_until_parked`, and the file exists with `sources.len() == 1` at 150 ms.
- `cargo test -p radar-core`: 114 passed.
- `cargo clippy -p radar-app --all-targets`: no warnings.
- `cargo fmt --check`: clean.
- `cargo check -p radar-server`: OK.

## TDD evidence
I wrote the tests and the implementation in the same pass. RED was then captured by temporarily stubbing the implementation and running the real tests, and both stubs were removed afterwards:

- With `tokens()` stubbed as `unimplemented!("RED stub")`: 10 failed, 2 passed. The 2 that passed were the bridge tests. Every theme and app_model test, plus both window tests, panicked with "not implemented: RED stub".
- With `tokens` restored and `schedule_save` stubbed to return immediately:
  - `mutation_persists_after_debounce` FAILED with `left: 0, right: 1`.
  - `rapid_mutations_coalesce...` FAILED with `left: 0, right: 2`.
  - The other 10 passed, including `hydrate_does_not_overwrite_store`, which passes by construction.
- GREEN: after restoring both, 12 of 12 passed. 13 of 13 after adding `quit_flushes_pending_save`.

## GPUI Kit 0.9.0 API facts confirmed from source (for later tasks)

### Imports
- Never `use gpui_kit::*` inside test modules: with `test-support` on, the glob brings in GPUI's `test` attribute, which shadows `#[test]`.
- I import explicitly everywhere: `use gpui_kit::{div, px, App, AppContext as _, Context, Entity, Global, Hsla, IntoElement, Render, Styled, Window, ...}`.
- The kit theme is `gpui_kit::component::{Theme, ThemeMode, ActiveTheme}`. `Theme` derefs to `ThemeColor`.
- `ThemeColor` fields are all `Hsla`. Among them: `background`, `foreground`, `border`, `primary`, `primary_foreground`, `primary_hover`, `primary_active`, `secondary*`, `muted`, `muted_foreground`, `accent`, `accent_foreground`, `popover`, `popover_foreground`, `ring`, `input`, `list*`, `table*`, `sidebar*`, `tab*`, `title_bar*`, `scrollbar*`, `danger*`, `success*`, `warning*`, `info*`, `link*`, `selection`, `caret`, `skeleton`, `switch`, `slider_*`.
- `Theme` fields: `font_family`, `font_size`, `mono_font_family`, `mono_font_size`, `radius`, `radius_lg`, `shadow`, `mode`.
- `Theme::update(cx, |th| ..)` edits colors and fields, re-syncs the Base layer and refreshes windows. Setting `mode` inside `update` reloads the registered theme over the colors, so switch mode with `Theme::change(mode, None, cx)` in a separate step first. `theme::apply` does this.
- Assets: `gpui_kit::assets::Assets` (Lucide icons, RustEmbed). Task 8 can wrap it in its own `AssetSource` that serves `assets/icons/ui/*` and falls back to it.

### HTTP
- The client module is `gpui_kit::http_client::{HttpClient, AsyncBody, Request, Response, RedirectPolicy, RequestTimeout, Url, http::HeaderValue}`.
- Install it with `Application::with_http_client(Arc<dyn HttpClient>)`, or with `cx.set_http_client` at runtime.
- `gpui-pre-reqwest-client` exists, but it brings its own tokio runtime and a forked reqwest, so I didn't use it.

### Windows and quitting
- `QuitMode::Default` resolves to `Explicit` on macOS and to `LastWindowClosed` everywhere else (gpui-pre `app.rs`: `cfg!(not(target_os = "macos"))`).
- Hence `.with_quit_mode(QuitMode::Explicit)`. `cx.set_quit_mode(..)` also exists.
- `cx.on_app_quit` / `Context::on_app_quit` futures are polled for up to `SHUTDOWN_TIMEOUT`.
- `window.on_window_should_close(cx, Fn(&mut Window, &mut App) -> bool)`.
- `cx.windows() -> Vec<AnyWindowHandle>`. `AnyWindowHandle` is `Copy + Eq + Hash`.
- `handle.update(cx, |_, window, cx| ..)`, `window.activate_window()`, `window.remove_window()`, `cx.activate(bool)`.
- `gpui_kit::open_window(options, cx, |window, cx| cx.new(..)) -> Result<(AnyWindowHandle, Entity<V>)>` wraps the view in `base::Root` itself.

### Tests
- Add a dev-dependency on `gpui-kit = { version = "0.9.0", features = ["test-support"] }`.
- Write tests as `#[gpui_kit::test] fn name(cx: &mut TestAppContext)`; `async fn` also works. `TestAppContext` is `gpui_kit::TestAppContext`.
- Clock control:
  - `cx.executor().advance_clock(d)` runs tasks and fires timers up to `now + d`.
  - `cx.run_until_parked()` runs ready tasks and does **not** advance the clock (`TestScheduler::run`).
  - `cx.executor().tick()` runs one task.
- `cx.quit()` runs the `on_app_quit` observers.
- Windows: `cx.update_window(handle, |_, window, cx| ..)`, `cx.windows()`, `cx.dispatch_action(window, action)`, `cx.simulate_keystrokes(window, "ctrl-k")`.
- UI helpers live in `gpui_kit::test::{TestWindowExt, TestSupportExt}`: `find`, `click(id, cx)`, `press`, `input`, `render_frame`. Elements are made findable with `.id(..)` plus `.test_support()`.
- Calling `gpui_kit::init(cx)` inside `cx.update` works under the test platform.

### Actions and keybindings
- `gpui_kit::actions!(radar, [OpenSearch, ...])`. This is the kit's own macro, so no `gpui` crate name is needed.
- ~~`KeyBinding::new("secondary-k", …)`~~ **Corrected in fix round 1:** `KeyBinding::new` panics on `secondary-`. Use `KeyBinding::load("secondary-k", Box::new(OpenSearch), Some(predicate), false, None, cx)`, then `cx.bind_keys([binding])`. For a context predicate, use `KeyBindingContextPredicate::parse("Workspace")` wrapped in an `Rc`.
- On elements: `.key_context("Workspace").track_focus(&handle).on_action(cx.listener(|this, _: &OpenSearch, window, cx| ..))`.
- To dispatch: `window.dispatch_action(Box::new(OpenSearch), cx)`, or `cx.dispatch_action(window, OpenSearch)` in tests.

### Async
- `cx.spawn(async move |this: WeakEntity<T>, cx: &mut AsyncApp| ..)`, `cx.background_spawn(fut)`, `cx.background_executor().timer(d)`.
- Dropping a `Task` cancels it.
- Under the test executor, tokio-backed futures (`on_rt`) are awaited from another thread's waker. The debounced save therefore runs on GPUI's `background_spawn`, not on tokio, so tests stay deterministic.
- Later tasks that test `on_rt` paths in `#[gpui_kit::test]` may need `cx.executor().allow_parking()` plus real waiting.

### Fonts
- `cx.text_system().add_fonts(Vec<Cow<'static, [u8]>>)` and `cx.text_system().all_font_names()`.

## Fonts result
- google/fonts upstream ships **only variable TTFs** for all three families. I embedded `Inter[opsz,wght]`, `Newsreader[opsz,wght]` and `JetBrainsMono[wght]`, renamed to `*-Variable.ttf`.
- Each family has its own OFL licence file (`OFL-Inter.txt`, `OFL-Newsreader.txt`, `OFL-JetBrainsMono.txt`), because the copyright lines differ.
- At runtime on Windows the log shows `font family "Inter" available`, `"Newsreader" available` and `"JetBrains Mono" available` (checked against `all_font_names()`).
- **Not verified:** whether weights 500 and 600 render from the variable `wght` axis or fall back to synthesized or regular weights. DirectWrite normally exposes the named instances. Task 8/9 should check this visually; if weights look wrong, switch to static instances (for example from the rsms/inter and JetBrains releases).
- Italics were not embedded. `Card.jsx` uses Newsreader italic, which the old web build synthesized.

## Keepalive result (Windows, verified)
- Steps:
  1. Started `target/debug/radar-app.exe`. After 6 s the process was alive, the main window title was "Radar", and the window rect was 1296×859 (1280×820 client area plus frame).
  2. Called `Process.CloseMainWindow()`, which sends WM_CLOSE, the same as clicking ×. The log showed `[window] main window closed; staying alive`.
  3. 4 s and 10 s later the process was still running, with `MainWindowHandle = 0`.
  4. Then killed it.
- `--autostart`: the process is alive with no window, and the log shows `autostart: true`.
- No hidden 1×1 window fallback was needed: `QuitMode::Explicit` is enough.
- macOS: **unverified**. `Explicit` is already the macOS default there.
- GPUI logs 3 errors on window destroy (`gpui-pre-windows window.rs:616/628/629 "Invalid window handle"`). They come from `WindowsWindow::drop` calling `ShowWindowAsync`/`RevokeDragDrop` on an HWND that the default WM_CLOSE handling already destroyed. This is upstream noise and harmless.
- Startup also logs an ERROR about the DXGI debug interface, which is the missing Graphics Tools SDK component. Also harmless.

## Background-color check (not verified)
- I tried to sample the window's pixels with a screen grab. Another full-screen application was in the foreground, so the grab captured that application instead of Radar, and the result is meaningless.
- I deleted the capture and stopped launching GUI windows, so the user's foreground session would not be disrupted.
- Background-color correctness rests on the token tests and `Workspace` painting `bg(t.bg)`. The visual check belongs on the R8 manual checklist.

## Files
- Modified:
  - `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\Cargo.toml`
  - `Cargo.lock`
- Created under `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-app\`:
  - `Cargo.toml`
  - `build.rs`
  - `src\main.rs`
  - `src\app_model.rs`
  - `src\bridge.rs`
  - `src\theme.rs`
  - `src\workspace.rs`
  - `assets\fonts\` (3 TTF files + 3 OFL files)
  - `assets\icons\` (4 files)

## Deviations from the brief
- `session: Option<Session>`, per the controller's decision; the brief had `Session`.
- `sync: Option<Arc<SyncClient>>` instead of `Option<SyncClient>`. `SyncClient` isn't `Clone`, and its async methods must outlive a borrow inside `on_rt`.
- Added a public `sync_queue: Vec<Value>` field (pass-through) and private save bookkeeping.
- `update_model` is generic over its return value (`-> R`), so callers can get ids back from `add_source` and similar. Calls that return `()` work unchanged.
- `update_model` also re-applies theme tokens when the visual prefs change. Task 8's `header_theme_toggle...` test gets this for free.
- Added the quit flush, which the brief didn't ask for. Without it, the tray's Quit (Task 14) could lose the last 100 ms of edits.
- `update_model`, `schedule_save` and `SAVE_DEBOUNCE` carry `#[allow(dead_code)]`, and the AppModel struct carries one for its future-use fields. They have no non-test caller until Task 8 and Task 12. Remove the allows once those callers exist.

## Self-review findings
- A failed `store::save` is logged but doesn't set `dirty` again. The next mutation retries, but a lone failed write isn't retried on its own. This is acceptable for now; Task 12/14 may want a retry or an indicator in the UI.
- When a new mutation replaces `save_task`, a write that was taken but not yet started gets cancelled. That's safe: `dirty` is true again, so the newer debounce (or the quit flush) writes a superset.
- Adding gpui-kit to `Cargo.lock` bumped some shared crates within semver (futures 0.3.32 → newer, icu 2.2 → newer, wasm-bindgen 0.2.118 → newer). radar-core tests and the radar-server check stay green.
- The dev build of `radar-app.exe` is about 53 MB.

## Concerns
1. The visual check (background color, font weights) was not done at runtime, because the screen was occupied. It goes on the R8 manual checklist.
2. Variable-font weights 500 and 600 are unverified.
3. Keepalive on macOS is unverified.

---

# Fix round 1 (R22, R23)

## I1 / R22: the quit flush and failed saves no longer drop edits
File: `radar-app/src/app_model.rs`.

### Changes
- `SaveJob::run` now returns `io::Result<()>`.
- A new `run_locked(&mut written)` does the write while the caller holds the `written_gen` lock.
- The debounced task is now a loop:
  1. Wait on the timer.
  2. Take the job, which captures a fresh snapshot.
  3. Run it on `background_spawn`.
  4. On `Err`: log the error, re-mark the entity `dirty = true`, and retry with backoff. The first retry waits `SAVE_RETRY_MIN` (1 s); the delay doubles up to `SAVE_RETRY_MAX` (60 s).
  - Each retry takes a new snapshot, so later edits are included.
  - A newer mutation still replaces the task, and the newer task writes a superset.
- `flush_now`, which runs from `on_app_quit`:
  1. Drops `save_task`. This cancels a pending timer or a write that hasn't started.
  2. Locks `written_gen`. This waits for any write already running on the background executor.
  3. If `save_gen > written`, a taken snapshot never landed (it was cancelled or failed), so it sets `dirty = true`.
  4. Takes the job and writes it synchronously with `run_locked`, still holding the lock.

### Why the quit write is synchronous
GPUI polls quit futures for only `SHUTDOWN_TIMEOUT`, which is 200 ms (gpui-pre `app.rs`). That is too short to trust with an 8 MiB archive write. So the write finishes inside the callback, and the future it returns has nothing left to await.

### How the three loss paths are covered
- **(a) Job taken, then cancelled:** the generation check re-marks the entity dirty.
- **(b) Write in flight at quit:** the lock waits for it, then the generation check decides whether anything is still unwritten.
- **(c) Save returned `Err`:** the retry loop handles it, and the quit flush also catches it through the generation check.

### Tests
- `quit_flush_writes_job_taken_but_cancelled`: takes the job and drops it (`save_task = None`), then calls `cx.quit()`. The file holds the edit.
  - The review asked for "advance 100 ms, tick only until the job is taken". That isn't possible: `TestScheduler::advance_clock` calls `run()` the moment the timer expires, which drains every runnable task, and there's no public way to advance the clock alone.
  - The test therefore hits the same seam directly (`take_save_job` plus cancel). A doc comment on the test explains this.
- `failed_save_is_retried`:
  - A directory at the store path makes `store::save` fail.
  - After 100 ms the path is still a directory, so the first attempt failed.
  - The test removes the directory and advances `SAVE_RETRY_MIN`; then `sources.len() == 1`.
- `quit_after_failed_save_writes_it`: same failure, then `cx.quit()` before the retry timer fires. The edit lands.

### RED before the fix (`cargo test -p radar-app app_model`)
```
test app_model::tests::quit_flush_writes_job_taken_but_cancelled ... FAILED   (left: 0, right: 1)
test app_model::tests::failed_save_is_retried ... FAILED                      (left: 0, right: 1)
test result: FAILED. 6 passed; 2 failed
```
After the fix, all app_model tests pass.

## I2 / R23: Quit action, app menu, single instance
Files: `radar-app/src/main.rs` and the new `radar-app/src/single_instance.rs`.

### Quit
- `gpui_kit::actions!(radar, [Quit])`.
- `register_app_actions(cx, quit: fn(&mut App))` sets up three things:
  - A global `cx.on_action::<Quit>` handler that logs and then calls `quit`. The app passes `|cx| cx.quit()`.
  - A `secondary-q` binding created with `KeyBinding::load(.., cx)`.
  - The app menu: `cx.set_menus([Menu::new("Radar").items([MenuItem::action("Quit Radar", Quit)])])`. On Windows GPUI only stores menus; on macOS it shows them.
- **Finding for Task 8:** `KeyBinding::new("secondary-…")` panics with `InvalidKeystrokeError`, because only `KeyBinding::load` resolves `secondary-` for the current OS. The test caught this, and it would have panicked at startup. Task 8's `secondary-k` binding must use `load` as well.

### Single instance: startup negotiation
The check runs in `main` before file logging starts. `acquire(name, msg)` runs on `radar_core::runtime()` and does the following:
- It tries `Stream::connect` first.
- **If the connect succeeds,** another instance is running. The process writes `show\n` (normal launch) or `ping\n` (`--autostart`), prints a line to stderr, and exits 0.
- **If the connect fails,** it tries to create the listener.
  - On Windows, creation fails when the name is already in use, because interprocess opens the pipe with `FILE_FLAG_FIRST_PIPE_INSTANCE`.
  - After a failed creation it retries: connect, then listen, 3 attempts in total, 100 ms apart.
- **If every attempt fails,** the app fails open: it logs a warning and keeps running.

### Single instance: message handling in the running instance
- `serve(listener, tx)` runs on the runtime.
- Each connection gets a 2 s read timeout and at most 17 bytes.
- `parse_message` accepts only the exact strings `show\n` and `ping\n`. Anything else is logged as "ignored" and dropped; nothing is executed or interpreted.
- Valid messages travel over a `futures` mpsc channel to `listen_for_instances`, a GPUI foreground task:
  - `Show` calls `open_main_window`, which is idempotent: it opens the window or focuses the existing one.
  - `Ping` is only logged.

### Single instance: access control
- **Windows:**
  - The pipe is `\\.\pipe\com.radar.dev.<USERNAME>.instance`.
  - Its DACL is protected: `D:P(A;;GA;;;SY)(A;;GA;;;OW)`. Only SYSTEM and the pipe owner (Owner Rights) get access.
  - The DACL is built with interprocess `SecurityDescriptor::deserialize` from a `widestring` string; `widestring` is a new Windows-only dependency.
  - Same-user connects are verified. Cross-user denial is not runtime-tested, because this machine has a single user.
- **Unix** (only Windows is compiled here, so this code is unverified):
  - The socket is `<runtime_dir or data_local_dir>/com.radar.dev/instance.sock`.
  - Its directory is created and forced to mode 0700.
  - `try_overwrite(true)` is used only after a connect has failed, so any socket file it overwrites belongs to a dead instance.

### Logging changes
- **Problem found in the first manual run:** each second process rotated the running instance's `rCURRENT` log file. flexi_logger with `Naming::Numbers` and no `.append()` rotates on every start.
- **Fix:**
  - The second process now exits before `init_logging` runs.
  - Logging now uses `.append()`, so restarts continue `rCURRENT` and the file rotates only at 5 MB.

### New dependencies
- `interprocess 2.4`, with the `tokio` feature.
- `tokio`, with `io-util`, `time` and `rt`.
- `widestring 1`, Windows only.

### Tests
- `tests::quit_action_quits`:
  - Registers the app's real Quit handler, with `App::shutdown` as the terminal step. That's what the real platform runs on quit; the test platform's `quit()` is a no-op.
  - Makes a model edit, then dispatches the `secondary-q` keystroke (parsed for the current OS) to the window.
  - Afterwards the store file holds the edit. That proves the chain keystroke → binding → Quit → shutdown → flush hook.
- `tests::quit_is_bound_and_in_app_menu`: there is one keymap binding for `Quit`, and the app menu has one item.
- `tests::second_instance_show_message_opens_window`: `Ping` opens no window, `Show` opens one, and a second `Show` still leaves exactly one.
- `single_instance::tests::parses_only_exact_messages`: covers garbage, wrong case, a missing newline, two messages, oversized input, and an empty message.
- `single_instance::tests::second_instance_hands_message_to_first`: uses a real pipe with a unique name. The first `acquire` becomes Primary and later ones Secondary; `Show` and `Ping` both reach the server.
- `single_instance::tests::garbage_is_not_forwarded`: `calc.exe\n` and 64 junk bytes are dropped. Only the `show\n` sent after them is forwarded, and nothing else arrives within 300 ms.
- **RED for the Quit and Show wiring:** I stubbed out the handler's `quit(cx)` call and the `open_main_window` call in `listen_for_instances`. `quit_action_quits` and `second_instance_show_message_opens_window` then FAILED (20 passed, 2 failed). Both stubs were restored.

## Commands and output
```
$ cargo test -p radar-app
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
$ cargo clippy -p radar-app --all-targets      (after touch, so radar-app was re-checked)
    Checking radar-app v0.2.22 ...
    Finished `dev` profile [unoptimized + debuginfo] target(s)   (no warnings)
```

## Manual Windows check (debug binary)
```
1) primary(--autostart) pid=12452 exited=False hwnd=0
2) second --autostart exited=True exit=0; primary hwnd=0          (no window change)
3) second plain exited=True exit=0; primary hwnd=61409680 title='Radar'   (first instance showed its window)
4) after close: primary exited=False hwnd=0; processes=1
5) --autostart while running headless: exited=True exit=0; primary hwnd=0
log files before=6 after=6   (second processes no longer rotate the log)
```
Primary log:
```
Radar 0.2.22 starting (autostart: true)
[instance] another launch sent Ping
[instance] another launch sent Show
[window] main window closed; staying alive
[instance] another launch sent Ping
```
- An earlier run, made before the logging change, covered the case where the window is already open: a third plain launch exited 0, and exactly 1 radar-app process remained.
- Ctrl+Q was not tried against the live window. That needs foreground keyboard focus, which I avoided taking. The headless keystroke test covers it.

## Corrections to the earlier self-review
- The finding "a failed save isn't retried" is fixed by R22.

## Still unverified
- The macOS menu and quit path.
- The Unix single-instance code, which does not compile on this host.
- Whether another user is actually denied access to the pipe.
