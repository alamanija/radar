//! Port of `src/components/Sidebar.jsx`: brand, view nav with counts,
//! category filters, the first sources, the sync indicator (signed in only)
//! and the profile footer.

use super::icon::{Icon, IconName};
use super::sync_indicator;
use crate::app_model::AppModel;
use crate::theme::{self, Tokens};
use crate::workspace::{SourcesIntent, Workspace};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, img, px, AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    StyledImage as _,
};
use radar_core::types::View;

/// `.app`'s `--sidebar-w` when expanded (collapsed is 0: not rendered).
pub const WIDTH: f32 = 260.;
/// The sidebar lists this many sources, then "View all".
const SOURCES_SHOWN: usize = 8;

pub fn render(app: &AppModel, cx: &Context<Workspace>) -> impl IntoElement {
    let t = *theme::t(cx);
    let m = &app.model;
    let view = m.prefs.view;
    let unread = m.unread_count();
    let saved = m.articles.iter().filter(|a| a.bookmarked).count();

    let nav = |id: &'static str, icon, label: &'static str, count: Option<usize>, to: View| {
        item(
            id.into(),
            Lead::Icon(icon),
            label.into(),
            count,
            view == to,
            &t,
        )
        .on_click(move |_, _, cx| AppModel::mutate(cx, |m| m.prefs.view = to))
        .test_support()
    };
    let views = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(nav(
            "nav-briefing",
            IconName::Sparkles,
            "Briefing",
            Some(unread),
            View::Briefing,
        ))
        .child(nav(
            "nav-saved",
            IconName::Bookmark,
            "Saved",
            Some(saved),
            View::Saved,
        ))
        .child(nav(
            "nav-archive",
            IconName::Archive,
            "Archive",
            None,
            View::Archive,
        ))
        .child(nav(
            "nav-sources",
            IconName::Feed,
            "Sources",
            Some(m.sources.len()),
            View::Sources,
        ))
        .child(nav(
            "nav-categories",
            IconName::Tag,
            "Categories",
            Some(m.categories.len()),
            View::Categories,
        ))
        .child(nav(
            "nav-settings",
            IconName::Settings,
            "Settings",
            None,
            View::Settings,
        ));

    let categories = (!m.categories.is_empty()).then(|| {
        let rows = m.categories.iter().map(|c| {
            let count = m.articles.iter().filter(|a| a.category == c.id).count();
            let active = view == View::Briefing && m.active_category == c.id;
            let cat_id = c.id.clone();
            item(
                format!("sidebar-category-{}", c.id).into(),
                Lead::Dot(t.dot_for(&c.accent)),
                c.label.clone().into(),
                Some(count),
                active,
                &t,
            )
            .on_click(move |_, _, cx| {
                let cat_id = cat_id.clone();
                AppModel::mutate(cx, |m| {
                    m.prefs.view = View::Briefing;
                    m.active_category = cat_id;
                })
            })
            .test_support()
        });
        div()
            .child(section("Categories", &t))
            .child(div().flex().flex_col().gap(px(1.)).children(rows))
    });

    let add_source = div()
        .id("sidebar-add-source")
        .test_support()
        .flex()
        .cursor_pointer()
        .text_color(t.text_3)
        .child(Icon::new(IconName::Plus).size(px(13.)))
        .on_click(cx.listener(|ws, _, _, cx| {
            ws.sources_intent = Some(SourcesIntent::Add);
            cx.notify();
            AppModel::mutate(cx, |m| m.prefs.view = View::Sources);
        }));
    let sources = m.sources.iter().take(SOURCES_SHOWN).map(|s| {
        let id = s.id;
        let icon = if s.enabled {
            IconName::CircleFilled
        } else {
            IconName::Circle
        };
        source_row(format!("sidebar-source-{id}").into(), &t)
            .opacity(if s.enabled { 1. } else { 0.45 })
            .child(Icon::new(icon).size(px(10.)))
            .child(div().flex_1().min_w_0().truncate().child(s.name.clone()))
            .on_click(move |_, _, cx| AppModel::mutate(cx, |m| m.toggle_source(id)))
            .test_support()
    });
    let view_all = source_row("sidebar-view-all-sources".into(), &t)
        .text_color(t.text_3)
        .child(Icon::new(IconName::ChevronRight).size(px(12.)))
        .child(format!("View all ({})", m.sources.len()))
        .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.view = View::Sources))
        .test_support();

    let scroll = div()
        .id("sidebar-scroll")
        .flex_1()
        .overflow_y_scroll()
        .pt(px(4.))
        .px(px(10.))
        .pb(px(16.))
        .child(views)
        .children(categories)
        .child(section("Sources", &t).child(add_source))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .children(sources)
                .child(view_all),
        );

    div()
        .id("sidebar")
        .test_support()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(WIDTH))
        .h_full()
        .min_w_0()
        .overflow_hidden()
        .bg(t.surface_2)
        .border_r_1()
        .border_color(t.border)
        .child(brand(&t))
        .child(scroll)
        .children(
            app.account
                .is_some()
                .then(|| sync_indicator::render(&app.sync_status, &t)),
        )
        .child(footer(app, &t))
}

fn brand(t: &Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .pt(px(18.))
        .px(px(18.))
        .pb(px(16.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(6.))
                .bg(t.text)
                .text_color(t.bg)
                .child(Icon::new(IconName::Radar).size(px(16.))),
        )
        .child(
            div()
                .font_family(theme::FONT_SERIF)
                .text_size(px(18.))
                .font_weight(FontWeight::MEDIUM)
                .child("Radar"),
        )
}

/// `sidebarStyles.section` (with the `marginTop: 18` every use adds).
fn section(label: &str, t: &Tokens) -> Div {
    div()
        .mt(px(18.))
        .pt(px(10.))
        .px(px(10.))
        .pb(px(6.))
        .flex()
        .justify_between()
        .items_center()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text_3)
        .child(label.to_uppercase())
}

enum Lead {
    Icon(IconName),
    /// A category dot; `None` paints nothing (unknown accent).
    Dot(Option<Hsla>),
}

/// `SidebarItem`. The count, when present, gets the id `<id>-count` and
/// its value as accessible label.
fn item(
    id: SharedString,
    lead: Lead,
    label: SharedString,
    count: Option<usize>,
    active: bool,
    t: &Tokens,
) -> Stateful<Div> {
    let lead: AnyElement = match lead {
        Lead::Icon(icon) => Icon::new(icon).size(px(15.)).into_any_element(),
        Lead::Dot(color) => div()
            .flex_none()
            .size(px(8.))
            .rounded_full()
            .when_some(color, |d, c| d.bg(c))
            .into_any_element(),
    };
    let count = count.map(|n| {
        div()
            .id(SharedString::from(format!("{id}-count")))
            .test_support()
            .aria_label(n.to_string())
            .ml_auto()
            .text_size(px(11.))
            .text_color(t.text_3)
            .font_features(theme::font_features(true))
            .child(n.to_string())
    });
    let el = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(32.))
        .px(px(10.))
        .rounded(px(5.))
        .text_size(px(13.5))
        .cursor_pointer()
        .child(lead)
        .child(div().flex_1().min_w_0().truncate().child(label))
        .children(count);
    if active {
        el.bg(t.surface)
            .text_color(t.text)
            .shadow(vec![theme::ring(1., t.border, true)])
    } else {
        let hover = t.surface;
        el.text_color(t.text_2).hover(move |s| s.bg(hover))
    }
}

/// `sidebarStyles.sourceRow`.
fn source_row(id: SharedString, t: &Tokens) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(30.))
        .px(px(10.))
        .rounded(px(5.))
        .text_size(px(13.))
        .text_color(t.text_2)
        .cursor_pointer()
}

fn footer(app: &AppModel, t: &Tokens) -> Div {
    let m = &app.model;
    let account = app.account.as_ref();
    let name = m.display_name(account);
    let role = Some(m.profile.role.trim())
        .filter(|r| !r.is_empty())
        .or(account.map(|a| a.email.as_str()).filter(|e| !e.is_empty()))
        .unwrap_or("Settings → Profile")
        .to_string();
    let initial: SharedString = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".into())
        .into();
    let (soft, text) = (t.accent_soft, t.accent_text);
    let initial_avatar = move || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded_full()
            .bg(soft)
            .text_color(text)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial.clone())
            .into_any_element()
    };
    let avatar = match account.and_then(|a| a.image_url.clone()) {
        Some(url) => img(url)
            .flex_none()
            .size(px(28.))
            .rounded_full()
            .bg(t.surface_2)
            .object_fit(ObjectFit::Cover)
            .with_fallback(initial_avatar)
            .into_any_element(),
        None => initial_avatar(),
    };
    let (name_text, name_color) = if name.is_empty() {
        ("Set your name".to_string(), t.text_3)
    } else {
        (name, t.text)
    };
    let profile = div()
        .id("sidebar-profile")
        .test_support()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .cursor_pointer()
        .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.view = View::Settings))
        .child(
            div()
                .id("sidebar-profile-name")
                .test_support()
                .aria_label(name_text.clone())
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(name_color)
                .truncate()
                .child(name_text),
        )
        .child(
            div()
                .id("sidebar-profile-role")
                .test_support()
                .aria_label(role.clone())
                .text_size(px(11.))
                .text_color(t.text_3)
                .truncate()
                .child(role),
        );
    div()
        .p(px(12.))
        .border_t_1()
        .border_color(t.border)
        .flex()
        .items_center()
        .gap(px(10.))
        .child(avatar)
        .child(profile)
}
