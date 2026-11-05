//! Tray / menu-bar icon: icon, menu and status.
//!
//! - The menu is static: "Show Radar", "Run briefing now", a separator,
//!   "Quit Radar". Left-click toggles the main window; right-click opens
//!   the menu.
//! - Status lives in the hover tooltip (unread count, last run, top
//!   headlines). On macOS the unread count is also the menu-bar title. The
//!   model pushes both through [`set_status`] whenever they change.
//! - "Quit Radar" goes through `App::quit`, so the model's quit hook
//!   flushes the store and the sync push.
//!
//! # Threads
//! tray-icon creates the icon on a thread whose event loop is already
//! running, and calls the event handlers on that thread:
//! - macOS and Windows: GPUI's main thread. [`init`] runs from the `run`
//!   callback, after `NSApplication` finished launching or before the
//!   Win32 message loop, which dispatches for every window on the thread.
//! - Linux: the AppIndicator backend needs a GTK loop, so a thread of its
//!   own runs one (compiled, not release-tested).
//!
//! The handlers only forward a [`Command`] into a channel that a GPUI task
//! drains on the UI thread.

use anyhow::Context as _;
use async_channel::{Receiver, Sender};
use gpui_kit::App;
use radar_core::model::RadarModel;
use radar_core::text;
use std::cell::RefCell;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// 64×64, so AppKit doesn't scale the 512 px app icon on every draw.
const ICON_PNG: &[u8] = include_bytes!("../../assets/icons/64x64.png");
const TRAY_ID: &str = "radar-tray";
const SHOW_ID: &str = "tray:show";
const RUN_ID: &str = "tray:run";
const QUIT_ID: &str = "tray:quit";

/// The tray menu, top to bottom (a separator goes before the last item).
const MENU: [(&str, &str); 3] = [
    (SHOW_ID, "Show Radar"),
    (RUN_ID, "Run briefing now"),
    (QUIT_ID, "Quit Radar"),
];

thread_local! {
    /// The icon, on the thread that created it. Dropping it removes it.
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

/// What a tray interaction asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Show,
    RunBriefing,
    Quit,
}

fn command_for_menu(id: &str) -> Option<Command> {
    match id {
        SHOW_ID => Some(Command::Show),
        RUN_ID => Some(Command::RunBriefing),
        QUIT_ID => Some(Command::Quit),
        _ => None,
    }
}

/// The left button's release toggles the window.
fn command_for_click(event: &TrayIconEvent) -> Option<Command> {
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
    .then_some(Command::Toggle)
}

/// What the tray shows: the menu-bar title and the hover tooltip.
pub type Status = (Option<String>, String);

/// The menu-bar title (unread count, or none) and the hover tooltip for
/// `m`.
pub fn status(m: &RadarModel, now_ms: i64) -> Status {
    let unread = m.unread_count();
    let tooltip = text::tray_tooltip(unread, m.last_run_at_ms(), &m.articles, now_ms);
    (text::tray_title(unread), tooltip)
}

/// Creates the tray icon and routes its clicks and menu items to the app.
/// Call once, from GPUI's `run` callback. If the icon can't be created the
/// app runs without it: a second launch (or the Dock on macOS) still
/// opens the window.
pub fn init(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded();
    forward_events(tx);
    create();
    listen(rx, cx, |cx| cx.quit());
}

fn forward_events(tx: Sender<Command>) {
    let clicks = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let Some(command) = command_for_click(&event) {
            let _ = clicks.try_send(command);
        }
    }));
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_for_menu(event.id.as_ref()) {
            let _ = tx.try_send(command);
        }
    }));
}

/// Runs tray commands on the UI thread. `quit` is `App::quit` in the app.
fn listen(rx: Receiver<Command>, cx: &mut App, quit: impl Fn(&mut App) + 'static) {
    cx.spawn(async move |cx| {
        while let Ok(command) = rx.recv().await {
            log::info!("[tray] {command:?}");
            cx.update(|cx| match command {
                Command::Toggle => crate::toggle_main_window(cx),
                Command::Show => crate::open_main_window(cx),
                // The header button's path. The window is left as it is, so
                // with it closed the result arrives as a notification.
                Command::RunBriefing => crate::workspace::run_briefing(cx),
                Command::Quit => quit(cx),
            });
        }
    })
    .detach();
}

/// Shows `title` next to the icon (macOS; Windows has no title) and
/// `tooltip` on hover. Does nothing until the icon exists.
pub fn set_status(title: Option<String>, tooltip: String) {
    #[cfg(target_os = "linux")]
    gtk_loop::set_status(title, tooltip);
    #[cfg(not(target_os = "linux"))]
    apply(title, tooltip);
}

fn create() {
    #[cfg(target_os = "linux")]
    gtk_loop::spawn();
    #[cfg(not(target_os = "linux"))]
    install();
}

/// Builds the icon on this thread and keeps it alive.
fn install() -> bool {
    match build() {
        Ok(tray) => {
            TRAY.with(|t| *t.borrow_mut() = Some(tray));
            true
        }
        Err(e) => {
            log::error!("[tray] tray icon unavailable: {e:#}");
            false
        }
    }
}

fn build() -> anyhow::Result<TrayIcon> {
    let menu = Menu::new();
    let [show, run, quit] = MENU.map(|(id, label)| MenuItem::with_id(id, label, true, None));
    menu.append_items(&[&show, &run, &PredefinedMenuItem::separator(), &quit])?;
    let (rgba, width, height) = icon_rgba()?;
    let icon = Icon::from_rgba(rgba, width, height)?;
    let builder = TrayIconBuilder::new()
        .with_id(TRAY_ID)
        .with_tooltip("Radar")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    // A template image: macOS draws the alpha channel in the menu bar's
    // colour, light or dark.
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_templated(icon);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(icon);
    Ok(builder.build()?)
}

fn apply(title: Option<String>, tooltip: String) {
    TRAY.with(|t| {
        let Some(tray) = t.borrow().clone() else {
            return;
        };
        tray.set_title(title);
        #[cfg(windows)]
        let tooltip = fit_tooltip(tooltip);
        if let Err(e) = tray.set_tooltip(Some(tooltip)) {
            log::warn!("[tray] tooltip update failed: {e}");
        }
    });
}

/// Windows keeps 128 UTF-16 units of a tooltip, terminator included
/// (`szTip`), and tray-icon copies a longer one without terminating it.
#[cfg(any(windows, test))]
fn fit_tooltip(tooltip: String) -> String {
    const MAX_UNITS: usize = 127;
    if tooltip.encode_utf16().count() <= MAX_UNITS {
        return tooltip;
    }
    let mut out = String::new();
    let mut units = '…'.len_utf16();
    for c in tooltip.chars() {
        units += c.len_utf16();
        if units > MAX_UNITS {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// The tray icon as RGBA8 pixels, with its size.
fn icon_rgba() -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size().context("tray icon too large")?;
    let mut rgba = vec![0; size];
    let frame = reader.next_frame(&mut rgba)?;
    anyhow::ensure!(
        frame.color_type == png::ColorType::Rgba,
        "tray icon is {:?}, not RGBA",
        frame.color_type
    );
    rgba.truncate(frame.buffer_size());
    Ok((rgba, frame.width, frame.height))
}

/// The AppIndicator icon lives on a GTK thread; status updates hop over to
/// it through the GTK loop.
#[cfg(target_os = "linux")]
mod gtk_loop {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// The newest status, waiting for the GTK thread.
    static LATEST: Mutex<Option<super::Status>> = Mutex::new(None);
    /// The icon exists on the GTK thread.
    static READY: AtomicBool = AtomicBool::new(false);

    pub fn spawn() {
        let started = std::thread::Builder::new()
            .name("radar-tray".into())
            .spawn(|| {
                if let Err(e) = gtk::init() {
                    log::error!("[tray] GTK unavailable: {e}");
                    return;
                }
                if !super::install() {
                    return;
                }
                READY.store(true, Ordering::SeqCst);
                apply_latest();
                gtk::main();
            });
        if let Err(e) = started {
            log::error!("[tray] tray thread failed to start: {e}");
        }
    }

    pub fn set_status(title: Option<String>, tooltip: String) {
        *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some((title, tooltip));
        if READY.load(Ordering::SeqCst) {
            gtk::glib::idle_add_once(apply_latest);
        }
    }

    fn apply_latest() {
        let latest = LATEST.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((title, tooltip)) = latest {
            super::apply(title, tooltip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_model::AppModel;
    use crate::test_support::{at, wait_until, Fakes};
    use crate::workspace::shell_tests::article;
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;
    use tray_icon::dpi::PhysicalPosition;
    use tray_icon::{MouseButton, MouseButtonState, Rect, TrayIconId};

    #[test]
    fn menu_has_the_expected_labels_and_commands() {
        let labels: Vec<_> = MENU.iter().map(|(_, label)| *label).collect();
        assert_eq!(labels, ["Show Radar", "Run briefing now", "Quit Radar"]);
        let commands: Vec<_> = MENU.iter().map(|(id, _)| command_for_menu(id)).collect();
        assert_eq!(
            commands,
            [
                Some(Command::Show),
                Some(Command::RunBriefing),
                Some(Command::Quit)
            ]
        );
        assert_eq!(command_for_menu("other"), None);
    }

    #[test]
    fn left_click_release_toggles_the_window() {
        let click = |button, button_state| TrayIconEvent::Click {
            id: TrayIconId::new("radar"),
            position: PhysicalPosition::default(),
            rect: Rect::default(),
            button,
            button_state,
        };
        let up = click(MouseButton::Left, MouseButtonState::Up);
        assert_eq!(command_for_click(&up), Some(Command::Toggle));
        let down = click(MouseButton::Left, MouseButtonState::Down);
        assert_eq!(command_for_click(&down), None);
        let right = click(MouseButton::Right, MouseButtonState::Up);
        assert_eq!(
            command_for_click(&right),
            None,
            "right-click opens the menu"
        );
    }

    #[test]
    fn status_is_unread_title_and_tooltip() {
        let mut m = RadarModel::default();
        assert_eq!(
            status(&m, 0),
            (None, "Radar — all caught up · last run never".into())
        );
        m.articles = vec![article(1, "tools", false), article(2, "tools", true)];
        let (title, tooltip) = status(&m, 0);
        assert_eq!(title.as_deref(), Some("1"));
        assert_eq!(
            tooltip,
            "Radar — 1 unread · last run never\n\nFeed — Article 1\nFeed — Article 2"
        );
    }

    #[test]
    fn windows_tooltip_fits_the_tip_buffer() {
        let short = "Radar — all caught up".to_string();
        assert_eq!(fit_tooltip(short.clone()), short);
        for long in ["x".repeat(300), "🛰️".repeat(100)] {
            let fit = fit_tooltip(long);
            assert!(fit.encode_utf16().count() <= 127, "{}", fit.len());
            assert!(fit.ends_with('…'));
        }
    }

    #[test]
    fn icon_decodes_to_rgba() {
        let (rgba, w, h) = icon_rgba().unwrap();
        assert_eq!((w, h), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);
    }

    #[gpui_kit::test]
    fn tray_commands_reach_the_app(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = cx.update(|cx| {
            gpui_kit::init(cx);
            let model = AppModel::init(dir.path().join("radar.json"), None, cx);
            crate::workspace::init(cx);
            model
        });
        model.update(cx, |m, cx| {
            m.set_hooks(fakes.hooks(vec![article(1, "tools", false)], at(9, 0)), cx)
        });
        let (tx, rx) = async_channel::unbounded();
        let quits = Rc::new(Cell::new(0));
        let counted = quits.clone();
        cx.update(|cx| listen(rx, cx, move |_| counted.set(counted.get() + 1)));
        let send = |cx: &mut TestAppContext, c: Command| {
            tx.try_send(c).unwrap();
            cx.run_until_parked();
        };

        send(cx, Command::Show);
        assert_eq!(cx.windows().len(), 1);
        let window = cx.windows()[0];
        cx.update_window(window, |_, w, _| w.activate_window())
            .unwrap();
        cx.run_until_parked();
        send(cx, Command::Toggle);
        assert!(cx.windows().is_empty(), "the focused window closes");
        send(cx, Command::Toggle);
        assert_eq!(cx.windows().len(), 1, "no focused window: it opens");

        send(cx, Command::RunBriefing);
        assert_eq!(fakes.runs(), 1);
        wait_until(cx, |cx| !model.read_with(cx, |m, _| m.model.scanning));

        send(cx, Command::Quit);
        assert_eq!(quits.get(), 1);
    }
}
