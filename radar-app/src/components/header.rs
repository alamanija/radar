//! Port of `src/components/Header.jsx`: sidebar toggle, the view title,
//! search bar, grid/list control and briefing button (briefing view only),
//! and the theme toggle.

use super::icon::{Icon, IconName};
use super::icon_button;
use crate::app_model::AppModel;
use crate::theme::{self, Tokens};
use crate::workspace::{self, Workspace};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled,
};
use radar_core::types::{Theme, View, ViewMode};

/// The header title of each view (`labels` in `Header.jsx`, which has no
/// entry for Categories).
pub fn view_title(view: View) -> &'static str {
    match view {
        View::Briefing => "Morning Briefing",
        View::Saved => "Saved",
        View::Archive => "Archive",
        View::Sources => "Sources",
        View::Categories => "Categories",
        View::Settings => "Settings",
    }
}

fn view_icon(view: View) -> IconName {
    match view {
        View::Briefing => IconName::Sparkles,
        View::Saved => IconName::Bookmark,
        View::Archive => IconName::Archive,
        View::Sources => IconName::Feed,
        View::Categories => IconName::Tag,
        View::Settings => IconName::Settings,
    }
}

/// The search shortcut as shown in the search bar.
const SEARCH_KBD: &str = if cfg!(target_os = "macos") {
    "⌘K"
} else {
    "Ctrl+K"
};

pub fn render(app: &AppModel, cx: &Context<Workspace>) -> impl IntoElement {
    let t = *theme::t(cx);
    let prefs = &app.model.prefs;
    let view = prefs.view;
    let collapsed = prefs.sidebar_collapsed;

    let toggle_sidebar = icon_button(
        "header-sidebar-toggle",
        if collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronLeft
        },
        16.,
        false,
        &t,
    )
    .on_click(|_, _, cx| workspace::toggle_sidebar(cx))
    .test_support();

    let title = view_title(view);
    let date = (view == View::Briefing).then(|| {
        let date = chrono::Local::now().format("%A, %B %-d");
        div()
            .ml(px(4.))
            .min_w_0()
            .truncate()
            .text_size(px(12.))
            .text_color(t.text_3)
            .child(format!("· {date}"))
    });
    let breadcrumb = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(13.))
        .text_color(t.text_2)
        .child(Icon::new(view_icon(view)).size(px(14.)))
        .child(
            div()
                .id("header-title")
                .test_support()
                .aria_label(title)
                .flex_none()
                .text_color(t.text)
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .children(date);

    let search = div()
        .id("header-search")
        .test_support()
        .flex()
        .items_center()
        .gap(px(8.))
        .w(px(280.))
        .min_w(px(120.))
        .flex_shrink(1.)
        .h(px(32.))
        .px(px(10.))
        .border_1()
        .border_color(t.border)
        .rounded(px(6.))
        .bg(t.surface)
        .text_color(t.text_3)
        .text_size(px(13.))
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .on_click(cx.listener(|ws, _, window, cx| ws.open_search(window, cx)))
        .child(Icon::new(IconName::Search).size(px(14.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child("Search articles, sources…"),
        )
        .child(kbd(SEARCH_KBD, &t));

    let briefing_actions = (view == View::Briefing).then(|| {
        let mode = |id: &'static str, icon, value: ViewMode| {
            icon_button(id, icon, 14., prefs.view_mode == value, &t)
                .size(px(28.))
                .on_click(move |_, _, cx| AppModel::mutate(cx, |m| m.prefs.view_mode = value))
                .test_support()
        };
        let segmented = div()
            .flex()
            .border_1()
            .border_color(t.border)
            .rounded(px(6.))
            .p(px(2.))
            .bg(t.surface)
            .child(mode("header-view-grid", IconName::Grid, ViewMode::Grid))
            .child(mode("header-view-list", IconName::List, ViewMode::List));
        let scanning = app.model.scanning;
        let run = div()
            .id("header-run-briefing")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(32.))
            .pl(px(12.))
            .pr(px(14.))
            .rounded(px(6.))
            .bg(t.text)
            .text_color(t.bg)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .opacity(if scanning { 0.85 } else { 1. })
            .cursor_pointer()
            .on_click(|_, _, cx| workspace::run_briefing(cx))
            .child(Icon::new(IconName::Refresh).size(px(14.)).spin(scanning))
            .child(if scanning {
                "Scanning…"
            } else {
                "Morning Briefing"
            });
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(segmented)
            .child(run)
    });

    let theme_toggle = icon_button(
        "header-theme-toggle",
        match prefs.theme {
            Theme::Light => IconName::Moon,
            Theme::Dark => IconName::Sun,
        },
        15.,
        false,
        &t,
    )
    .on_click(|_, _, cx| workspace::toggle_theme(cx))
    .test_support();

    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(56.))
        .px(px(24.))
        .gap(px(12.))
        .border_b_1()
        .border_color(t.border)
        .bg(t.bg)
        .child(toggle_sidebar)
        .child(breadcrumb)
        .child(search)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(briefing_actions)
                .child(theme_toggle),
        )
}

/// `headerStyles.kbd`.
pub(crate) fn kbd(text: &'static str, t: &Tokens) -> Div {
    div()
        .ml_auto()
        .flex_none()
        .px(px(5.))
        .py(px(1.))
        .border_1()
        .border_color(t.border)
        .rounded(px(3.))
        .bg(t.surface_2)
        .font_family(theme::FONT_MONO)
        .text_size(px(10.5))
        .text_color(t.text_3)
        .child(text)
}
