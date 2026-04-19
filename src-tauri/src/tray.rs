//! System-tray / menu-bar presence.
//!
//! Deliberately minimal. The tray is an icon + hover tooltip + tiny
//! right-click menu. Menu contents:
//!
//!   Show Radar          ← focuses the main window
//!   Run briefing now    ← emits RUN_EVENT; frontend fires a briefing
//!   ---
//!   Quit Radar
//!
//! Everything status-like (unread count, last-run time, top article
//! headlines) lives in the tooltip string that the frontend builds and
//! pushes via `set_tray_status`. That keeps the menu bar visually quiet
//! and avoids the churn of rebuilding the native menu on every state
//! change.
//!
//! Best-practice notes (macOS):
//! - `icon_as_template(true)` tells AppKit to ignore RGB and use the alpha
//!   channel as a monochrome mask, so the icon auto-inverts on light/dark
//!   menu bars. The current icon is an RGBA app icon — the silhouette
//!   works, but a dedicated monochrome design would look crisper.
//! - We embed a 64×64 asset (`icons/64x64.png`) via `include_bytes!` rather
//!   than passing the 512×512 app icon; avoids having AppKit rescale a
//!   full-size bitmap every render.
//! - `show_menu_on_left_click(false)` — left-click toggles the window,
//!   right-click opens the menu. Standard macOS hybrid-menu-bar-app idiom.

use tauri::{
    AppHandle, Emitter, Manager, Runtime,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub const TRAY_ID: &str = "radar-tray";
pub const RUN_EVENT: &str = "tray://run-briefing";

/// Pre-sized menu-bar asset. 64×64 comfortably covers 22pt @2x retina and is
/// what the bundled icon set ships anyway; no extra asset lives in the repo.
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/64x64.png");

const SHOW_ID: &str = "tray:show";
const RUN_ID: &str = "tray:run";
const QUIT_ID: &str = "tray:quit";

pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let icon = tauri::image::Image::from_bytes(TRAY_ICON_BYTES)
        .expect("icons/64x64.png must be a valid PNG — check bundle.icon list");

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
                toggle_main_window(tray.app_handle());
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
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false);
    let focused = w.is_focused().unwrap_or(false);
    if visible && focused {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Called on `RunEvent::Reopen` — user clicked the dock icon, activated the
/// app via a notification click, or brought the app to the foreground.
pub fn surface_main_window<R: Runtime>(app: &AppHandle<R>) {
    show_main_window(app);
}

/// Push a formatted tooltip string (and optional short menu-bar title) from
/// the frontend. The frontend builds the whole tooltip — unread count +
/// last-run + top headlines — because it owns the state; Rust just sets
/// the strings on the tray.
#[tauri::command]
pub fn set_tray_status<R: Runtime>(
    app: AppHandle<R>,
    title: Option<String>,
    tooltip: String,
) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    tray.set_title(title).map_err(|e| e.to_string())?;
    tray.set_tooltip(Some(tooltip)).map_err(|e| e.to_string())?;
    Ok(())
}
