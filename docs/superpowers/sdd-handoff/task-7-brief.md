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

