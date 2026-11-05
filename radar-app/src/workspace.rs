//! The window's root view: sidebar, header, routed content and the
//! floating layers (Tweaks FAB/panel, search).
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
use crate::components::search_palette::SearchPalette;
use crate::components::{header, sidebar, tweaks};
use crate::theme;
use crate::views::archive::ArchiveView;
use crate::views::categories::{self, CategoriesView};
use crate::views::settings::{self, SettingsView};
use crate::views::sources::{self, SourcesView};
use crate::views::{archive, briefing, saved};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, hsla, px, relative, App, Context, Div, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, KeyBindingContextPredicate, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled, Window,
};
use radar_core::types::{Theme, View};
use std::rc::Rc;

gpui_kit::actions!(radar, [OpenSearch, RunBriefing, ToggleSidebar, ToggleTheme]);

/// Key context of the workspace root; shell bindings are scoped to it.
pub const KEY_CONTEXT: &str = "Workspace";

/// Why the Sources view was opened: the sidebar "+" asks it to open its
/// add row (consumed and cleared by [`Workspace::render`] once the Sources
/// view is shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcesIntent {
    Add,
}

pub struct Workspace {
    pub search_open: bool,
    pub tweaks_open: bool,
    pub sources_intent: Option<SourcesIntent>,
    /// Drill-in state of the Archive view.
    pub archive: ArchiveView,
    /// Filter and edit-row state of the Sources view.
    pub sources: SourcesView,
    /// Edit-row state of the Categories view.
    pub categories: CategoriesView,
    /// Inputs and API-key control state of the Settings view.
    pub settings: SettingsView,
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
        // Coming back to the window drains the sync queue.
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                AppModel::global(cx).update(cx, |m, _| m.drain_now());
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            search_open: false,
            tweaks_open: false,
            sources_intent: None,
            archive: ArchiveView::default(),
            sources: SourcesView::default(),
            categories: CategoriesView::default(),
            settings: SettingsView::default(),
            focus,
        }
    }

    /// The search bar and `OpenSearch`: opens the palette dialog, unless it
    /// is already up (no second dialog stacks on top of it).
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_open && window.has_active_dialog(cx) {
            return;
        }
        self.search_open = true;
        SearchPalette::open(cx.weak_entity(), window, cx);
        cx.notify();
    }

    /// Every palette dismissal path ends here.
    pub(crate) fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = false;
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = AppModel::global(cx);
        // Per-view state lives only while its view is shown.
        let shown = entity.read(cx).model.prefs.view;
        if shown != View::Archive {
            self.archive.selected_id = None;
        }
        if shown != View::Categories {
            self.categories = CategoriesView::default();
        }
        if shown == View::Sources {
            // One-shot intent from the sidebar "+" / onboarding: open the add
            // row if a category exists, and clear it either way.
            if self.sources_intent.take() == Some(SourcesIntent::Add)
                && !entity.read(cx).model.categories.is_empty()
            {
                self.sources.start_add(window, cx);
            }
        } else {
            self.sources = SourcesView::default();
        }
        if shown == View::Settings {
            self.settings.prepare(window, cx);
        } else {
            self.settings = SettingsView::default();
        }
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
                    .test_support()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content(prefs.view, self, app, window, cx)),
            );
        let tweaks = if self.tweaks_open {
            tweaks::panel(app, cx).into_any_element()
        } else {
            tweaks::fab(prefs.theme, cx).into_any_element()
        };

        div()
            .id("workspace")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|ws, _: &OpenSearch, window, cx| ws.open_search(window, cx)))
            .on_action(|_: &RunBriefing, _, cx| run_briefing(cx))
            .on_action(|_: &ToggleSidebar, _, cx| toggle_sidebar(cx))
            .on_action(|_: &ToggleTheme, _, cx| toggle_theme(cx))
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
            // The palette dialog has no visible scrim of its own (the kit
            // keeps that flag private); dim the window beneath it.
            .when(self.search_open, |root| {
                root.child(div().absolute().inset_0().bg(hsla(0., 0., 0., 0.35)))
            })
    }
}

/// Routed content: the page of the current view.
fn content(
    view: View,
    ws: &Workspace,
    app: &AppModel,
    window: &Window,
    cx: &Context<Workspace>,
) -> Div {
    match view {
        View::Briefing => briefing::render(app, window, cx),
        View::Saved => saved::render(app, window, cx),
        View::Archive => archive::render(&ws.archive, app, window, cx),
        View::Sources => sources::render(&ws.sources, app, cx),
        View::Categories => categories::render(&ws.categories, app, cx),
        View::Settings => settings::render(&ws.settings, app, cx),
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
pub(crate) mod shell_tests;
