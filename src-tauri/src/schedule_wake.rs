//! OS-level relauncher that wakes Radar at the scheduled briefing slot so
//! the in-process scheduler can fire even after the user fully Quit.
//!
//! The in-process `scheduler` fires briefings whenever Radar is alive; this
//! module hands the "cold start" case to the OS:
//!
//!   - macOS: a `LaunchAgent` plist at
//!     `~/Library/LaunchAgents/com.radar.scheduler.plist` with a
//!     `StartCalendarInterval` and `ProgramArguments` of
//!     `[<current_exe>, "--autostart"]`. `launchctl load/unload -w`.
//!   - Windows: a Task Scheduler entry named "Radar Scheduler" created via
//!     `schtasks.exe /Create /SC DAILY`, deleted via `/Delete /F`.
//!   - Linux: a pair of `~/.config/systemd/user/radar-scheduler.{service,timer}`
//!     units with an `OnCalendar` expression matching the slot, registered
//!     via `systemctl --user enable --now`.
//!
//! In every case the launched binary gets `--autostart`, which the main
//! setup path keys on to start with the window hidden (see `lib.rs`). Once
//! Radar is up, the in-process `scheduler` ticks, sees the slot is past,
//! and fires the briefing. If Radar is already running when the wake
//! fires, the OS scheduler skips the invocation (or spawns a second
//! process that exits fast — acceptable churn).
//!
//! `sync_schedule_wake` is idempotent: identical config → no-op.

use std::path::PathBuf;
use std::process::Command;

/// Shared label/name across all three platforms. launchd uses it as the
/// `Label`, schtasks uses it as `/TN`, systemd uses it as the unit stem.
const LABEL: &str = "com.radar.scheduler";

/// Idempotent sync. `enabled=false` removes the wake entry entirely.
#[tauri::command]
pub fn sync_schedule_wake(
    enabled: bool,
    hour: Option<u32>,
    minute: Option<u32>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return macos::sync(enabled, hour, minute);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::sync(enabled, hour, minute);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::sync(enabled, hour, minute);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (enabled, hour, minute);
        Ok(())
    }
}

fn parse_hm(hour: Option<u32>, minute: Option<u32>) -> Result<(u32, u32), String> {
    match (hour, minute) {
        (Some(h), Some(m)) if h < 24 && m < 60 => Ok((h, m)),
        _ => Err("invalid hour/minute".to_string()),
    }
}

fn current_exe_string() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|e| format!("current_exe: {e}"))
        .map(|p| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- macOS

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

        let (h, m) = parse_hm(hour, minute)?;
        let exec = current_exe_string()?;
        let new_contents = render_plist(&exec, h, m);

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
        // loaded plist in place. Safe when nothing is loaded (non-zero exit
        // we ignore).
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
        // Paths on macOS are rarely malicious but can contain `&` / `<`.
        // Escape to keep the plist XML valid.
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

// -------------------------------------------------------------- Windows

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    /// Task Scheduler takes "Radar Scheduler" (space-separated) happily;
    /// sticking to dotted LABEL for consistency with the other platforms.
    const TASK_NAME: &str = LABEL;

    pub fn sync(enabled: bool, hour: Option<u32>, minute: Option<u32>) -> Result<(), String> {
        if !enabled {
            // Idempotent: /Delete returns non-zero if the task doesn't exist.
            // Ignore — the post-condition "task is gone" is satisfied either
            // way.
            let _ = Command::new("schtasks")
                .args(["/Delete", "/TN", TASK_NAME, "/F"])
                .status();
            return Ok(());
        }

        let (h, m) = parse_hm(hour, minute)?;
        let exec = current_exe_string()?;
        let time = format!("{h:02}:{m:02}");

        // schtasks /TR is a single string; quoting the exe path handles
        // spaces (e.g. "Program Files"). --autostart is appended outside.
        let task_run = format!(r#""{exec}" --autostart"#);

        // /F on /Create overwrites an existing task with the same name,
        // making this a single-shot idempotent upsert.
        let status = Command::new("schtasks")
            .args([
                "/Create",
                "/SC", "DAILY",
                "/TN", TASK_NAME,
                "/TR", &task_run,
                "/ST", &time,
                "/F",
            ])
            .status()
            .map_err(|e| format!("spawn schtasks: {e}"))?;
        if !status.success() {
            return Err(format!("schtasks /Create exited {status}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    fn unit_dir() -> Option<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("systemd/user"));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/systemd/user"))
    }

    pub fn sync(enabled: bool, hour: Option<u32>, minute: Option<u32>) -> Result<(), String> {
        let dir = unit_dir().ok_or_else(|| "no HOME/XDG_CONFIG_HOME".to_string())?;
        let service_path = dir.join(format!("{LABEL}.service"));
        let timer_path = dir.join(format!("{LABEL}.timer"));
        let timer_unit = format!("{LABEL}.timer");

        if !enabled {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", &timer_unit])
                .status();
            let _ = std::fs::remove_file(&service_path);
            let _ = std::fs::remove_file(&timer_path);
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            return Ok(());
        }

        let (h, m) = parse_hm(hour, minute)?;
        let exec = current_exe_string()?;

        let service_contents = format!(
            r#"[Unit]
Description=Radar scheduled briefing (relauncher)

[Service]
Type=simple
ExecStart={exec} --autostart
"#
        );

        let timer_contents = format!(
            r#"[Unit]
Description=Radar daily briefing timer
Requires={LABEL}.service

[Timer]
Unit={LABEL}.service
OnCalendar=*-*-* {h:02}:{m:02}:00
Persistent=true

[Install]
WantedBy=timers.target
"#
        );

        // Idempotent early-out when both files already match on disk.
        let unchanged = std::fs::read_to_string(&service_path)
            .map(|s| s == service_contents)
            .unwrap_or(false)
            && std::fs::read_to_string(&timer_path)
                .map(|s| s == timer_contents)
                .unwrap_or(false);
        if unchanged {
            return Ok(());
        }

        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
        std::fs::write(&service_path, service_contents)
            .map_err(|e| format!("write service unit: {e}"))?;
        std::fs::write(&timer_path, timer_contents)
            .map_err(|e| format!("write timer unit: {e}"))?;

        // Reload before enable so systemd re-reads the new files.
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();

        let status = Command::new("systemctl")
            .args(["--user", "enable", "--now", &timer_unit])
            .status()
            .map_err(|e| format!("spawn systemctl: {e}"))?;
        if !status.success() {
            return Err(format!("systemctl enable exited {status}"));
        }
        Ok(())
    }
}
