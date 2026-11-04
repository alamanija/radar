//! Radar desktop app (GPUI Kit). Boot order: logging → application (assets,
//! HTTP client, explicit quit mode) → kit init → fonts → global model →
//! main window (skipped with `--autostart`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_model;
mod bridge;
mod theme;
mod workspace;

use app_model::AppModel;
use gpui_kit::{
    px, size, AnyWindowHandle, App, AppContext as _, Bounds, Global, QuitMode, TitlebarOptions,
    WindowBounds, WindowOptions,
};
use radar_core::auth::{AuthConfig, Session};
use radar_core::keychain::OsKeyStore;
use std::path::PathBuf;
use std::sync::Arc;
use workspace::Workspace;

/// Passed by the OS login item / scheduler wake: boot to the tray only.
const AUTOSTART_ARG: &str = "--autostart";
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;

fn main() {
    let _logger = init_logging();
    let autostart = std::env::args().any(|a| a == AUTOSTART_ARG);
    log::info!(
        "Radar {} starting (autostart: {autostart})",
        env!("CARGO_PKG_VERSION")
    );

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_http_client(Arc::new(bridge::ReqwestHttpClient::new()))
        // Closing the last window must not quit: the tray (Task 14) reopens
        // it. GPUI's default quits on last-window-close off macOS.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::register_fonts(cx);
            let session =
                AuthConfig::from_env().map(|cfg| Session::load(cfg, Arc::new(OsKeyStore)));
            if session.is_none() {
                log::info!("[auth] Clerk not configured; running local-only");
            }
            AppModel::init(radar_core::store::default_path(), session, cx);
            if !autostart {
                open_main_window(cx);
            }
        });
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
        window.on_window_should_close(cx, |_, _| {
            log::info!("[window] main window closed; staying alive");
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
}
