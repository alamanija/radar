//! Radar desktop app (GPUI Kit). Boot order: logging → single-instance
//! check → application (assets, HTTP client, explicit quit mode, Dock
//! reopen) → kit init → app identity → fonts → global model (and, off the
//! UI thread, the Tauri-era keychain cleanup) → tray and notification
//! clicks → the real OS hooks, session sync and the login-item re-point →
//! app actions/menu → shell key bindings → scheduler and run-at-launch →
//! main window (skipped with `--autostart`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_model;
mod bridge;
mod components;
mod platform;
mod scheduler;
mod single_instance;
#[cfg(test)]
mod test_support;
mod theme;
mod views;
mod workspace;

use app_model::{AppModel, Hooks};
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt as _;
use gpui_kit::{
    px, size, AnyWindowHandle, App, AppContext as _, Bounds, Global, KeyBinding, Menu, MenuItem,
    QuitMode, TitlebarOptions, WindowBounds, WindowOptions,
};
use platform::{autostart, notify, tray, updater};
use radar_core::auth::{AuthConfig, Session};
use radar_core::keychain::OsKeyStore;
use single_instance::{InstanceName, Message, Role};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use workspace::Workspace;

gpui_kit::actions!(radar, [Quit]);

/// Passed by the OS login item / scheduler wake: boot to the tray only.
const AUTOSTART_ARG: &str = "--autostart";
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;
/// Socket/pipe id of the real app (tests use their own).
const INSTANCE_ID: &str = "com.radar.dev";

fn main() {
    let autostart = std::env::args().any(|a| a == AUTOSTART_ARG);

    // One process per user. A second launch asks the first to show its
    // window (unless it is an --autostart launch) and exits. This runs
    // before file logging starts so a short-lived second process never
    // touches (or rotates) the running instance's log file.
    let msg = if autostart {
        Message::Ping
    } else {
        Message::Show
    };
    let name = InstanceName::for_user(INSTANCE_ID);
    let role = radar_core::runtime().block_on(single_instance::acquire(&name, msg));
    if let Ok(Role::Secondary) = role {
        eprintln!("Radar is already running; sent {msg:?} to it, exiting");
        return;
    }

    let _logger = init_logging();
    log::info!(
        "Radar {} starting (autostart: {autostart})",
        env!("CARGO_PKG_VERSION")
    );
    let (tx, rx) = futures::channel::mpsc::unbounded();
    match role {
        Ok(Role::Secondary) => unreachable!("handled above"),
        Ok(Role::Primary(listener)) => {
            radar_core::runtime().spawn(single_instance::serve(listener, tx));
        }
        // Fail open: running twice beats not running at all.
        Err(e) => log::warn!("[instance] single-instance check failed: {e}; continuing"),
    }

    let app = gpui_kit::application()
        .with_assets(components::icon::AppAssets)
        .with_http_client(Arc::new(bridge::ReqwestHttpClient::new()))
        // Closing the last window must not quit: the tray reopens it.
        // GPUI's default quits on last-window-close off macOS. Quitting is
        // explicit: the Quit action (secondary-q, app menu) or the tray.
        .with_quit_mode(QuitMode::Explicit);
    // macOS: clicking the Dock icon with no window open brings it back
    // (Tauri `RunEvent::Reopen`).
    app.on_reopen(open_main_window);
    app.run(move |cx| {
        gpui_kit::init(cx);
        // Who posts notifications (the Windows AppUserModelID).
        cx.set_app_identity(INSTANCE_ID, "Radar");
        theme::register_fonts(cx);
        let session = AuthConfig::from_env().map(|cfg| Session::load(cfg, Arc::new(OsKeyStore)));
        if session.is_none() {
            log::info!("[auth] Clerk not configured; running local-only");
        }
        let model = AppModel::init(radar_core::store::default_path(), session, cx);
        // The Tauri app's `clerk_db_jwt` keychain entry is dead weight now.
        drop(bridge::on_blocking(|| {
            radar_core::keychain::delete_legacy_entries(&OsKeyStore)
        }));
        // Before the hooks, so their first status push lands on an icon.
        tray::init(cx);
        notify::init(cx);
        model.update(cx, |m, cx| {
            m.set_hooks(os_hooks(), cx);
            // A session restored from the keychain syncs right away.
            m.start_sync(cx);
            // An enabled "Open at login" entry from an older build (the
            // Tauri app) is pointed at this executable.
            m.repoint_login_item();
        });
        register_app_actions(cx, |cx| cx.quit());
        workspace::init(cx);
        listen_for_instances(rx, cx);
        scheduler::spawn(cx);
        scheduler::run_at_launch(cx);
        updater::check_on_launch(cx);
        if !autostart {
            open_main_window(cx);
        }
    });
}

/// The real integrations. Only the app installs them; tests use fakes.
fn os_hooks() -> Hooks {
    Hooks {
        briefing: app_model::ingest_runner(),
        clock: Rc::new(chrono::Local::now),
        notify: Rc::new(notify::briefing_ready),
        tray: Rc::new(tray::set_status),
        wake: Some(Arc::new(radar_core::schedule_wake::sync_schedule_wake)),
        autostart: autostart::Backend::os(),
        updater: match updater::Packager::production() {
            Ok(p) => Arc::new(p),
            Err(e) => {
                log::warn!("[updater] disabled: {e}");
                Arc::new(updater::Inert)
            }
        },
        quit: Rc::new(|cx| cx.quit()),
    }
}

/// Binds `Quit` to `secondary-q` (cmd-q / ctrl-q) and installs the app menu
/// ("Quit Radar"; GPUI shows it on macOS). `quit` is the terminal step:
/// `App::quit` in the app, after which GPUI runs the model's flush hook.
fn register_app_actions(cx: &mut App, quit: fn(&mut App)) {
    cx.on_action(move |_: &Quit, cx| {
        log::info!("[app] quit requested");
        quit(cx);
    });
    // `secondary-` is resolved per OS only by `KeyBinding::load`;
    // `KeyBinding::new` panics on it.
    match KeyBinding::load("secondary-q", Box::new(Quit), None, false, None, cx) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(e) => log::error!("[app] invalid Quit binding: {e}"),
    }
    cx.set_menus([Menu::new("Radar").items([MenuItem::action("Quit Radar", Quit)])]);
}

/// Handles messages from later launches: `Show` opens/focuses the main
/// window, `Ping` is only logged.
fn listen_for_instances(mut rx: UnboundedReceiver<Message>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(msg) = rx.next().await {
            log::info!("[instance] another launch sent {msg:?}");
            if msg == Message::Show {
                cx.update(open_main_window);
            }
        }
    })
    .detach();
}

/// The main window, if one is open.
#[derive(Default)]
struct MainWindow(Option<AnyWindowHandle>);

impl Global for MainWindow {}

/// Opens the main window, or focuses it if it is already open.
pub fn open_main_window(cx: &mut App) {
    let existing = cx.try_global::<MainWindow>().and_then(|w| w.0);
    if let Some(handle) = existing.filter(|h| cx.windows().contains(h)) {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        cx.activate(true);
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(820.)),
            cx,
        ))),
        window_min_size: Some(size(px(920.), px(620.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Radar".into()),
            ..Default::default()
        }),
        app_id: Some("com.radar.dev".into()),
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.on_window_should_close(cx, |_, cx| {
            flush_on_close(cx);
            true
        });
        cx.new(|cx| Workspace::new(window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            cx.set_global(MainWindow(Some(handle)));
            cx.activate(true);
        }
        Err(e) => log::error!("[window] failed to open main window: {e:#}"),
    }
}

/// The Radar window that has OS focus, if any (JS `document.hasFocus()`).
pub fn active_radar_window(cx: &mut App) -> Option<AnyWindowHandle> {
    cx.windows().into_iter().find(|handle| {
        handle
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false)
    })
}

/// The tray's left-click (Tauri `toggle_main_window`): closes the focused
/// Radar window, otherwise opens or focuses the main window.
pub fn toggle_main_window(cx: &mut App) {
    match active_radar_window(cx) {
        Some(handle) => {
            flush_on_close(cx);
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        None => open_main_window(cx),
    }
}

/// A closing window flushes: the store write and the sync push happen now
/// (JS: flush on `pagehide`). The app keeps running, so the push finishes
/// on its own.
fn flush_on_close(cx: &mut App) {
    log::info!("[window] main window closed; staying alive");
    drop(AppModel::global(cx).update(cx, |m, cx| m.flush_now(cx)));
}

fn log_dir() -> PathBuf {
    let fallback = || PathBuf::from(".");
    #[cfg(target_os = "macos")]
    let dir = dirs::home_dir()
        .unwrap_or_else(fallback)
        .join("Library/Logs/com.radar.dev");
    #[cfg(windows)]
    let dir = dirs::data_local_dir()
        .unwrap_or_else(fallback)
        .join("com.radar.dev")
        .join("logs");
    #[cfg(not(any(windows, target_os = "macos")))]
    let dir = dirs::data_dir()
        .unwrap_or_else(fallback)
        .join("com.radar.dev")
        .join("logs");
    dir
}

/// Rolling `Radar*.log` files (5 MB each) in the per-OS log dir, plus stdout
/// in debug builds. `RUST_LOG` overrides the default `info` level. The
/// handle must live as long as the process.
fn init_logging() -> Option<flexi_logger::LoggerHandle> {
    use flexi_logger::{Cleanup, Criterion, Duplicate, FileSpec, Logger, Naming};
    let dup = if cfg!(debug_assertions) {
        Duplicate::All
    } else {
        Duplicate::None
    };
    let started = Logger::try_with_env_or_str("info").and_then(|l| {
        l.log_to_file(FileSpec::default().directory(log_dir()).basename("Radar"))
            .rotate(
                Criterion::Size(LOG_ROTATE_BYTES),
                Naming::Numbers,
                Cleanup::KeepLogFiles(5),
            )
            // Keep writing to rCURRENT across restarts; rotate on size only.
            .append()
            .duplicate_to_stdout(dup)
            .format(flexi_logger::detailed_format)
            .start()
    });
    match started {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("logging disabled: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn boot(cx: &mut TestAppContext) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppModel::init(path, None, cx);
        });
        dir
    }

    #[gpui_kit::test]
    fn open_main_window_is_idempotent(cx: &mut TestAppContext) {
        let _dir = boot(cx);
        cx.update(open_main_window);
        cx.update(open_main_window);
        assert_eq!(cx.windows().len(), 1);
    }

    #[gpui_kit::test]
    fn reopens_after_close(cx: &mut TestAppContext) {
        let _dir = boot(cx);
        cx.update(open_main_window);
        let first = cx.windows()[0];
        cx.update_window(first, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert!(cx.windows().is_empty());
        cx.update(open_main_window);
        assert_eq!(cx.windows().len(), 1);
        assert_ne!(cx.windows()[0], first);
    }

    #[gpui_kit::test]
    fn quit_action_quits(cx: &mut TestAppContext) {
        let dir = boot(cx);
        let path = dir.path().join("radar.json");
        // The test platform's `quit` is a no-op; what the real platform does
        // on quit is `App::shutdown` (runs on_app_quit hooks), so the test
        // wires that in as the terminal step.
        cx.update(|cx| {
            register_app_actions(cx, |cx| cx.shutdown());
            open_main_window(cx);
            AppModel::global(cx).update(cx, |m, cx| {
                m.update_model(cx, |r| r.add_source("A".into(), "x".into(), None))
            });
        });
        assert!(!path.exists(), "nothing saved before quit");
        let window = cx.windows()[0];
        let os = cx.read(|cx| cx.platform_info().os);
        let keystroke = gpui_kit::Keystroke::parse_with_os("secondary-q", os).unwrap();
        // Shutdown closes the window inside this update, so the update
        // itself reports "window not found"; the file is the evidence.
        let _ = cx.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx)
        });
        assert_eq!(radar_core::store::load(&path).sources.len(), 1);
    }

    #[gpui_kit::test]
    fn quit_is_bound_and_in_app_menu(cx: &mut TestAppContext) {
        let _dir = boot(cx);
        cx.update(|cx| {
            register_app_actions(cx, |_| {});
            let keymap = cx.key_bindings();
            let bound = keymap.borrow().bindings_for_action(&Quit).count();
            assert_eq!(bound, 1, "Quit has a key binding");
            let menus = cx.get_menus().expect("menus set");
            assert_eq!(menus[0].items.len(), 1);
        });
    }

    #[gpui_kit::test]
    fn second_instance_show_message_opens_window(cx: &mut TestAppContext) {
        let _dir = boot(cx);
        let (tx, rx) = futures::channel::mpsc::unbounded();
        cx.update(|cx| listen_for_instances(rx, cx));

        tx.unbounded_send(Message::Ping).unwrap();
        cx.run_until_parked();
        assert!(cx.windows().is_empty(), "ping must not open a window");

        tx.unbounded_send(Message::Show).unwrap();
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);

        tx.unbounded_send(Message::Show).unwrap();
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1, "show focuses, never duplicates");
    }
}
