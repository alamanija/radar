//! "Open at login" through `auto-launch`, as `tauri-plugin-autostart` did
//! with `MacosLauncher::LaunchAgent`:
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

use anyhow::Context as _;
use auto_launch::{AutoLaunch, AutoLaunchBuilder, MacOSLaunchMode, WindowsEnableMode};
use std::sync::Arc;

/// The Tauri app's login item had the same name (`productName`), so an
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
