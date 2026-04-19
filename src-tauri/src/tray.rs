//! System-tray / menu-bar presence.
//!
//! Builds a persistent tray icon with a dynamic menu that rebuilds whenever
//! the briefing changes or the window is shown/hidden. Menu contents:
//!
//!   Recent              ← disabled header (only when we have articles)
//!   <article 1>         ← click → opens article URL in browser
//!   <article 2>
//!   …up to MAX_ARTICLES
//!   ---
//!   Show/Hide Radar     ← label tracks actual window visibility
//!   Run briefing now    ← emits RUN_EVENT, frontend fires a briefing
//!   ---
//!   Quit Radar
//!
//! Article state lives in `TrayState` (an `AppHandle`-managed singleton) so
//! the menu-event callback — which only receives the clicked item's id —
//! can look a URL up by index. Menu-item ids are namespaced strings:
//!
//!   tray:toggle-window            single id, dispatches on live visibility
//!   tray:run                      fire briefing
//!   tray:quit                     exit the process
//!   tray:article:<index>          open the article at that index
//!
//! Best-practice notes (macOS):
//! - `icon_as_template(true)` — when the bundled icon is replaced with a
//!   monochrome PNG it'll auto-adapt to light/dark menu bars.
//! - `show_menu_on_left_click(false)` — left-click toggles the window,
//!   right-click opens the menu. Standard macOS hybrid-menu-bar-app idiom.

use std::sync::Mutex;

use serde::Deserialize;
use tauri::{
    AppHandle, Emitter, Manager, Runtime,
    menu::{Menu, MenuBuilder, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_opener::OpenerExt;

pub const TRAY_ID: &str = "radar-tray";
pub const RUN_EVENT: &str = "tray://run-briefing";

const TOGGLE_ID: &str = "tray:toggle-window";
const RUN_ID: &str = "tray:run";
const QUIT_ID: &str = "tray:quit";
const ARTICLE_ID_PREFIX: &str = "tray:article:";

/// Cap on article menu items. More than ~5 turns the menu into a wall.
const MAX_ARTICLES: usize = 5;

/// Truncation width for article titles in menu items — long headlines
/// eat the whole menu otherwise.
const TITLE_MAX_CHARS: usize = 48;

#[derive(Default)]
pub struct TrayState {
    articles: Mutex<Vec<ArticleLink>>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleLink {
    pub title: String,
    pub url: String,
}

pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    app.manage(TrayState::default());

    let menu = build_menu(app, &[], false)?;
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
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
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

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        TOGGLE_ID => toggle_main_window(app),
        RUN_ID => {
            show_main_window(app);
            let _ = app.emit(RUN_EVENT, ());
        }
        QUIT_ID => app.exit(0),
        id if id.starts_with(ARTICLE_ID_PREFIX) => {
            let Ok(idx) = id[ARTICLE_ID_PREFIX.len()..].parse::<usize>() else {
                return;
            };
            let url = {
                let state = app.state::<TrayState>();
                state
                    .articles
                    .lock()
                    .ok()
                    .and_then(|a| a.get(idx).map(|link| link.url.clone()))
            };
            if let Some(url) = url {
                let _ = app.opener().open_url(url, None::<&str>);
            }
        }
        _ => {}
    }
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    articles: &[ArticleLink],
    window_visible: bool,
) -> tauri::Result<Menu<R>> {
    let mut b = MenuBuilder::new(app);

    if !articles.is_empty() {
        // Disabled header item acts as a section label.
        let header = MenuItem::new(app, "Recent", false, None::<&str>)?;
        b = b.item(&header);
        for (i, link) in articles.iter().take(MAX_ARTICLES).enumerate() {
            let id = format!("{ARTICLE_ID_PREFIX}{i}");
            let label = truncate(&link.title, TITLE_MAX_CHARS);
            let item = MenuItem::with_id(app, &id, &label, true, None::<&str>)?;
            b = b.item(&item);
        }
        b = b.separator();
    }

    let toggle_label = if window_visible { "Hide Radar" } else { "Show Radar" };
    let toggle = MenuItem::with_id(app, TOGGLE_ID, toggle_label, true, None::<&str>)?;
    let run = MenuItem::with_id(app, RUN_ID, "Run briefing now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit Radar", true, None::<&str>)?;

    b = b.item(&toggle).item(&run).separator().item(&quit);
    b.build()
}

fn rebuild_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let articles = {
        let state = app.state::<TrayState>();
        state.articles.lock().map(|a| a.clone()).unwrap_or_default()
    };
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    let menu = build_menu(app, &articles, visible)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    let _ = rebuild_menu(app);
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
    let _ = rebuild_menu(app);
}

/// Called from `lib.rs` after the close-to-tray handler hides the window —
/// keeps the menu's Show/Hide label in sync.
pub fn on_window_hidden<R: Runtime>(app: &AppHandle<R>) {
    let _ = rebuild_menu(app);
}

/// Called on `RunEvent::Reopen` — user clicked the dock icon, activated the
/// app via a notification click, or brought the app to the foreground.
pub fn surface_main_window<R: Runtime>(app: &AppHandle<R>) {
    show_main_window(app);
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

/// Replace the list of articles shown at the top of the tray menu. Called
/// from the frontend on every briefing change; the Rust side stores the
/// snapshot so the menu-click handler can look up a URL by index.
#[tauri::command]
pub fn set_tray_articles<R: Runtime>(
    app: AppHandle<R>,
    articles: Vec<ArticleLink>,
) -> Result<(), String> {
    {
        let state = app.state::<TrayState>();
        let mut lock = state.articles.lock().map_err(|e| e.to_string())?;
        *lock = articles.into_iter().take(MAX_ARTICLES).collect();
    }
    rebuild_menu(&app).map_err(|e| e.to_string())
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let mut out: String = chars.iter().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}
