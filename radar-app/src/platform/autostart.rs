//! "Open at login" through `auto-launch`, the same mechanisms earlier
//! releases registered:
//! - macOS: a LaunchAgent plist, `~/Library/LaunchAgents/Radar.plist`;
//! - Windows: the current user's `Run` registry key;
//! - Linux: an XDG autostart entry.
//!
//! The OS starts Radar with `--autostart`, which boots it to the tray
//! without a window.
//!
//! The OS owns the truth, so callers read [`is_enabled`] instead of
//! mirroring it. Both functions do blocking file or registry I/O, so they
//! must run off the UI thread. Settings reaches them through the model's
//! [`Backend`] on the blocking pool.

use crate::app_model::AppModel;
use crate::bridge;
use anyhow::Context as _;
use auto_launch::{AutoLaunch, AutoLaunchBuilder, MacOSLaunchMode, WindowsEnableMode};
use std::sync::Arc;

/// Earlier releases registered their login item under the same name, so an
/// existing entry is found and replaced rather than duplicated.
const APP_NAME: &str = "Radar";

/// The login item for this executable. Building it touches nothing.
fn launcher() -> anyhow::Result<AutoLaunch> {
    let exe = std::env::current_exe().context("locating the executable")?;
    let exe = exe.to_str().context("executable path is not UTF-8")?;
    Ok(AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(exe)
        .set_args(&[crate::AUTOSTART_ARG])
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        // `Dynamic` (the default) tries the machine-wide key first.
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .build()?)
}

/// Whether Radar is registered to start at login. Off when the OS can't
/// say (logged).
pub fn is_enabled() -> bool {
    let enabled = launcher().and_then(|l| Ok(l.is_enabled()?));
    enabled.unwrap_or_else(|e| {
        log::warn!("[autostart] status unavailable: {e:#}");
        false
    })
}

/// Registers (or removes) the login item.
pub fn set(enabled: bool) -> anyhow::Result<()> {
    let l = launcher()?;
    if enabled {
        l.enable()?;
    } else {
        l.disable()?;
    }
    log::info!("[autostart] open at login: {enabled}");
    Ok(())
}

/// Points an enabled login item at this executable: the entry an older
/// build registered (an earlier release, or an install elsewhere) may start
/// a path that is gone or holds the old app. Enabling rewrites the entry in
/// place, so running this on every launch changes nothing once it matches.
/// A disabled item stays off. Blocks; returns whether it rewrote the entry.
pub fn repoint(backend: &Backend) -> bool {
    if !(backend.is_enabled)() {
        return false;
    }
    match (backend.set)(true) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("[autostart] re-registering the login item failed: {e:#}");
            false
        }
    }
}

impl AppModel {
    /// [`repoint`] through the model's backend, on the blocking pool.
    /// `main.rs` calls it once at startup, after installing the real hooks.
    pub fn repoint_login_item(&self) {
        let backend = self.hooks.autostart.clone();
        // Runs to completion on its own; nothing waits for it.
        drop(bridge::on_blocking(move || repoint(&backend)));
    }
}

/// The login item as the model holds it: [`Backend::os`] in the app,
/// fakes in tests. Both functions block.
#[derive(Clone)]
pub struct Backend {
    pub is_enabled: Arc<dyn Fn() -> bool + Send + Sync>,
    pub set: Arc<dyn Fn(bool) -> anyhow::Result<()> + Send + Sync>,
}

impl Backend {
    /// The OS login item ([`is_enabled`], [`set`]).
    pub fn os() -> Self {
        Self {
            is_enabled: Arc::new(is_enabled),
            set: Arc::new(set),
        }
    }

    /// Always off; `set` changes nothing.
    pub fn off() -> Self {
        Self {
            is_enabled: Arc::new(|| false),
            set: Arc::new(|_| Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::sync::Mutex;

    /// A fake login item that is `enabled`, recording each `set`.
    fn fake(enabled: bool) -> (Backend, Arc<Mutex<Vec<bool>>>) {
        let sets = Arc::new(Mutex::new(Vec::new()));
        let recorded = sets.clone();
        let backend = Backend {
            is_enabled: Arc::new(move || enabled),
            set: Arc::new(move |on| {
                recorded.lock().unwrap().push(on);
                Ok(())
            }),
        };
        (backend, sets)
    }

    #[test]
    fn repoint_rewrites_only_an_enabled_item() {
        let (off, sets) = fake(false);
        assert!(!repoint(&off));
        assert!(sets.lock().unwrap().is_empty(), "never creates one");

        let (on, sets) = fake(true);
        assert!(repoint(&on));
        assert!(repoint(&on));
        assert_eq!(*sets.lock().unwrap(), [true, true], "only ever enables");

        let failing = Backend {
            is_enabled: Arc::new(|| true),
            set: Arc::new(|_| Err(anyhow::anyhow!("denied"))),
        };
        assert!(!repoint(&failing));
    }

    #[gpui_kit::test]
    fn startup_repoint_runs_off_the_ui_thread(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = cx.update(|cx| AppModel::init(dir.path().join("radar.json"), None, cx));
        let (backend, sets) = fake(true);
        let on_ui = Arc::new(AtomicBool::new(false));
        let ui_thread = std::thread::current().id();
        let seen = on_ui.clone();
        let set = backend.set.clone();
        model.update(cx, |m, _| {
            m.hooks.autostart = Backend {
                is_enabled: Arc::new(move || {
                    seen.store(std::thread::current().id() == ui_thread, SeqCst);
                    true
                }),
                set,
            };
            m.repoint_login_item();
        });
        crate::test_support::wait_until(cx, |_| sets.lock().unwrap().len() == 1);
        assert!(!on_ui.load(SeqCst), "the OS call ran on the UI thread");
    }

    /// Building the launcher only describes the login item; nothing is
    /// registered (`enable` is never called here).
    #[test]
    fn launcher_registers_radar_with_the_autostart_flag() {
        let l = launcher().unwrap();
        assert_eq!(l.get_app_name(), "Radar");
        assert_eq!(l.get_args(), ["--autostart"]);
        let exe = std::env::current_exe().unwrap();
        assert_eq!(l.get_app_path(), exe.to_str().unwrap());
    }
}
