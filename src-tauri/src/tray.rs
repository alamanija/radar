//! System-tray / menu-bar presence.
//!
//! Builds a persistent tray icon with a menu, keeps the app alive when the
//! user closes the window (hide-to-tray), and exposes a command so the
//! frontend can push "unread · 3h ago" status into the tray title.
//!
//! The daily-briefing scheduler in `App.jsx` is frontend-only, so we have to
//! keep the webview process alive for it to keep firing. Intercepting window
//! close → hide (rather than quitting) does exactly that. Real background
//! scheduling (in Rust) is a separate, larger change.
//!
//! Best-practice notes (macOS):
//! - `icon_as_template(true)` tells AppKit the icon is a monochrome template
//!   so it adapts to light/dark menu bars. The current icon is the bundled
//!   app icon, which is colored — it still renders, just not "properly" as a
//!   template. Ship a dedicated monochrome PNG under `icons/tray.png` before
//!   GA and load it here.
//! - `show_menu_on_left_click(false)` makes left-click focus the window
//!   (standard macOS idiom for hybrid menu-bar apps) and right-click open
//!   the menu.

use tauri::{
    AppHandle, Emitter, Manager, Runtime,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub const TRAY_ID: &str = "radar-tray";
pub const RUN_EVENT: &str = "tray://run-briefing";

const SHOW_ID: &str = "tray:show";
const RUN_ID: &str = "tray:run";
const QUIT_ID: &str = "tray:quit";

pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build_menu(app)?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle.icon must be configured — tray cannot render without a default icon");

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Radar")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => show_main_window(app),
            RUN_ID => {
                show_main_window(app);
                let _ = app.emit(RUN_EVENT, ());
            }
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let show = MenuItem::with_id(app, SHOW_ID, "Show Radar", true, None::<&str>)?;
    let run = MenuItem::with_id(app, RUN_ID, "Run briefing now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit Radar", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&show, &run, &sep, &quit])
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Pushed from the frontend whenever the briefing changes. `unread` feeds the
/// tray title on macOS (and tooltip everywhere); `last_run_label` is the
/// already-formatted relative time, e.g. "3h ago".
#[tauri::command]
pub fn set_tray_status<R: Runtime>(
    app: AppHandle<R>,
    unread: u32,
    last_run_label: Option<String>,
) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };

    let title = match (unread, last_run_label.as_deref()) {
        (0, None) | (0, Some("never")) => None,
        (0, Some(l)) => Some(l.to_string()),
        (n, Some(l)) => Some(format!("{n} · {l}")),
        (n, None) => Some(n.to_string()),
    };
    tray.set_title(title).map_err(|e| e.to_string())?;

    let tooltip = if unread == 0 {
        "Radar — all caught up".to_string()
    } else {
        format!("Radar — {unread} unread")
    };
    tray.set_tooltip(Some(tooltip)).map_err(|e| e.to_string())?;

    Ok(())
}
