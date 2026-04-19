//! macOS `LaunchAgent` that wakes Radar at the scheduled briefing slot.
//!
//! The in-process `scheduler` fires briefings whenever Radar is alive. To
//! also fire after the user hits Quit, we lean on launchd: a plist under
//! `~/Library/LaunchAgents/com.radar.scheduler.plist` with a
//! `StartCalendarInterval` matching `prefs.scheduleTime` re-launches Radar
//! with `--autostart` at slot time. The autostart flag hides the main
//! window (see `lib.rs::setup()`), Radar mounts, the in-process scheduler
//! sees the slot is past and fires — just like any other wake path.
//!
//! If Radar is already running when the plist fires, launchd skips the
//! invocation (it's a scheduled wake, not a tick); the running scheduler
//! handles the briefing.
//!
//! Non-macOS builds keep the command as a no-op so the frontend can call
//! `sync_schedule_plist` unconditionally.

#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

const LABEL: &str = "com.radar.scheduler";

/// Idempotent: ensures the plist on disk matches the arguments, reloading
/// launchd if needed. Disabling removes the plist entirely.
#[tauri::command]
pub fn sync_schedule_plist(
    enabled: bool,
    hour: Option<u32>,
    minute: Option<u32>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::sync(enabled, hour, minute)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows/Linux equivalents (Task Scheduler, systemd timers) aren't
        // wired yet — fail open so the frontend doesn't bubble an error.
        let _ = (enabled, hour, minute);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    pub fn sync(enabled: bool, hour: Option<u32>, minute: Option<u32>) -> Result<(), String> {
        let path = plist_path().ok_or_else(|| "no HOME".to_string())?;

        if !enabled {
            if path.exists() {
                let _ = run_launchctl(&["unload", "-w"], &path);
                if let Err(e) = std::fs::remove_file(&path) {
                    return Err(format!("remove plist: {e}"));
                }
            }
            return Ok(());
        }

        let (h, m) = match (hour, minute) {
            (Some(h), Some(m)) if h < 24 && m < 60 => (h, m),
            _ => return Err("invalid hour/minute".to_string()),
        };

        let exec_path = std::env::current_exe()
            .map_err(|e| format!("current_exe: {e}"))?
            .to_string_lossy()
            .to_string();

        let new_contents = render_plist(&exec_path, h, m);

        // Skip the unload/write/load dance if nothing changed — reloading
        // pointlessly churns launchd's state and can race with a just-fired
        // wake.
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if existing == new_contents {
                return Ok(());
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }

        // Always unload before rewriting — launchctl refuses to overwrite a
        // loaded plist in place, and this works even if no plist is loaded
        // (exit status is just non-zero, which we ignore).
        if path.exists() {
            let _ = run_launchctl(&["unload", "-w"], &path);
        }

        std::fs::write(&path, new_contents).map_err(|e| format!("write plist: {e}"))?;

        run_launchctl(&["load", "-w"], &path)
    }

    fn plist_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        )
    }

    fn render_plist(exec_path: &str, hour: u32, minute: u32) -> String {
        // Paths on macOS are rarely malicious but can contain `&` / `<` (e.g.
        // "Projects/Foo&Bar/..."). Escape to keep the plist XML valid.
        let exec = xml_escape(exec_path);
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
        <string>--autostart</string>
    </array>
    <key>StartCalendarInterval</key>
    <dict>
        <key>Hour</key>
        <integer>{hour}</integer>
        <key>Minute</key>
        <integer>{minute}</integer>
    </dict>
    <key>RunAtLoad</key>
    <false/>
</dict>
</plist>
"#
        )
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn run_launchctl(prefix_args: &[&str], plist: &std::path::Path) -> Result<(), String> {
        let status = Command::new("launchctl")
            .args(prefix_args)
            .arg(plist)
            .status()
            .map_err(|e| format!("spawn launchctl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("launchctl exited {status}"))
        }
    }
}
