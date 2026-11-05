//! The window's root view: sidebar, header, routed content and the
//! floating layers (Tweaks FAB/panel, search). Port of the layout in
//! `App.jsx`.
//!
//! # Pattern for later tasks
//! - Views read the global [`AppModel`] and change it with
//!   `AppModel::mutate(cx, |m| ..)`; the workspace re-renders on every
//!   model notification.
//! - State that is per-window and not persisted (search open, Tweaks open,
//!   the Sources "add" intent) lives on [`Workspace`].
//! - A view replaces its placeholder in [`content`] (by `prefs.view`).
//! - Shortcuts: an action in [`actions!`](gpui_kit::actions) plus a
//!   binding in [`init`] with context [`KEY_CONTEXT`]. The workspace
//!   focuses itself when the window opens, so its bindings work at once.

use crate::app_model::AppModel;
use crate::components::icon::{Icon, IconName};
use crate::components::{header, sidebar, tweaks};
use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, hsla, px, relative, App, BoxShadow, Context, Div, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, KeyBindingContextPredicate, KeyDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled, Window,
};
use radar_core::types::{Theme, View};
use std::rc::Rc;

gpui_kit::actions!(radar, [OpenSearch, RunBriefing, ToggleSidebar, ToggleTheme]);

/// Key context of the workspace root; shell bindings are scoped to it.
pub const KEY_CONTEXT: &str = "Workspace";

/// Why the Sources view was opened: the sidebar "+" asks it to open its
/// add row (consumed and cleared by the Sources view, Task 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcesIntent {
    Add,
}

pub struct Workspace {
    pub search_open: bool,
    pub tweaks_open: bool,
    pub sources_intent: Option<SourcesIntent>,
    focus: FocusHandle,
}

/// Binds the shell shortcuts. Call once at startup, before windows open.
pub fn init(cx: &mut App) {
    let context = match KeyBindingContextPredicate::parse(KEY_CONTEXT) {
        Ok(p) => Rc::new(p),
        Err(e) => {
            log::error!("[keys] invalid context {KEY_CONTEXT:?}: {e}");
            return;
        }
    };
    // `secondary-` (cmd on macOS, ctrl elsewhere) resolves only through
    // `KeyBinding::load`; `KeyBinding::new` panics on it.
    match KeyBinding::load(
        "secondary-k",
        Box::new(OpenSearch),
        Some(context),
        false,
        None,
        cx,
    ) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(e) => log::error!("[keys] invalid OpenSearch binding: {e}"),
    }
}

/// The `RunBriefing` action and the header button.
pub fn run_briefing(cx: &mut App) {
    AppModel::global(cx).update(cx, |m, cx| m.run_briefing(cx));
}

pub fn toggle_sidebar(cx: &mut App) {
    AppModel::mutate(cx, |m| m.prefs.sidebar_collapsed ^= true);
}

pub fn toggle_theme(cx: &mut App) {
    AppModel::mutate(cx, |m| {
        m.prefs.theme = match m.prefs.theme {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    });
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&AppModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            search_open: false,
            tweaks_open: false,
            sources_intent: None,
            focus,
        }
    }

    /// The search bar and `OpenSearch`. The palette itself is Task 13.
    pub fn open_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = true;
        cx.notify();
    }

    fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = false;
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = AppModel::global(cx);
        let app = entity.read(cx);
        let t = *theme::t(cx);
        let prefs = &app.model.prefs;

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(header::render(app, cx))
            .child(
                div()
                    .id("content")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content(prefs.view, &t)),
            );
        let tweaks = if self.tweaks_open {
            tweaks::panel(app, cx).into_any_element()
        } else {
            tweaks::fab(prefs.theme, cx).into_any_element()
        };
        let search = self.search_open.then(|| search_placeholder(window, &t, cx));

        div()
            .id("workspace")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|ws, _: &OpenSearch, _, cx| ws.open_search(cx)))
            .on_action(|_: &RunBriefing, _, cx| run_briefing(cx))
            .on_action(|_: &ToggleSidebar, _, cx| toggle_sidebar(cx))
            .on_action(|_: &ToggleTheme, _, cx| toggle_theme(cx))
            .when(self.search_open, |root| {
                root.on_key_down(cx.listener(|ws, e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" {
                        ws.close_search(cx);
                        cx.stop_propagation();
                    }
                }))
            })
            .relative()
            .size_full()
            .flex()
            .overflow_hidden()
            .bg(t.bg)
            .text_color(t.text)
            .font_family(theme::FONT_SANS)
            .font_features(theme::font_features(false))
            .text_size(px(14.))
            .line_height(relative(1.5))
            .children((!prefs.sidebar_collapsed).then(|| sidebar::render(app, cx)))
            .child(main)
            .child(tweaks)
            .children(search)
    }
}

/// Routed content. Each view's real page replaces its placeholder here in
/// a later task (briefing 9, saved/archive 10, sources/categories 11,
/// settings 12).
fn content(view: View, t: &Tokens) -> Div {
    let title = header::view_title(view);
    // `viewStyles.wrap` + `viewStyles.title`.
    div()
        .w_full()
        .max_w(px(1240.))
        .mx_auto()
        .pt(px(24.))
        .px(px(32.))
        .pb(px(80.))
        .child(
            div()
                .id("content-title")
                .test_support()
                .aria_label(title)
                .mb(px(6.))
                .font_family(theme::FONT_SERIF)
                .text_size(px(28.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
                .child(title),
        )
}

/// Stand-in for the search palette (Task 13): the palette's scrim and
/// input row. Clicking outside closes it; so does Escape (handled on the
/// root, which holds focus).
fn search_placeholder(window: &Window, t: &Tokens, cx: &Context<Workspace>) -> impl IntoElement {
    let panel = div()
        .w(px(600.))
        .max_w(relative(0.9))
        .bg(t.surface)
        .border_1()
        .border_color(t.border_strong)
        .rounded(t.radius_lg)
        .shadow(vec![BoxShadow::new(
            px(0.),
            px(20.),
            hsla(0., 0., 0., 0.25),
        )
        .blur_radius(px(30.))])
        .overflow_hidden()
        // Clicks inside the panel must not reach the scrim.
        .occlude()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(14.))
                .text_color(t.text_2)
                .child(Icon::new(IconName::Search).size(px(16.)))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(15.))
                        .text_color(t.text_3)
                        .child("Search articles, sources…"),
                ),
        );
    div()
        .id("search-scrim")
        .test_support()
        .absolute()
        .inset_0()
        .flex()
        .items_start()
        .justify_center()
        .pt(window.viewport_size().height * 0.1)
        .bg(hsla(0., 0., 0., 0.35))
        .occlude()
        .on_click(cx.listener(|ws, _, _, cx| ws.close_search(cx)))
        .child(panel)
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod shell_tests;
