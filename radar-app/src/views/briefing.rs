//! The briefing view: greeting, summary line, stats
//! strip, error banner, category chips and the article grid or list (or the
//! first-run tutorial / loading skeletons).
//!
//! # Layout notes
//! - The grid fits as many equal columns of at least 320 px as it can. That
//!   needs the container width, so the column count is derived from the
//!   window size each frame ([`grid_columns`]).
//! - Lists of more than [`VIRTUAL_MIN`] rows render through a virtual list
//!   with its own scroll area, because GPUI cannot virtualize inside the
//!   page's scroll container.

use super::onboarding;
use crate::app_model::AppModel;
use crate::components::card::{Card, CardOpts};
use crate::components::row::{cell, grid_row, Col, Row};
use crate::components::sidebar;
use crate::components::skeleton::{SkeletonCard, SkeletonRow};
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use chrono::{Local, Utc};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::v_virtual_list;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, hsla, px, size, AnyElement, Context, Div, FontWeight, HighlightStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Window,
};
use radar_core::text;
use radar_core::types::{Article, Category, SourceError, ViewMode};
use std::rc::Rc;

/// List mode switches to a virtual list above this many articles.
pub const VIRTUAL_MIN: usize = 50;
/// Page content width limit and horizontal padding.
const MAX_WIDTH: f32 = 1240.;
const GUTTER: f32 = 32.;
const GRID_GAP: f32 = 14.;
const CARD_MIN_WIDTH: f32 = 320.;
/// Skeleton counts while scanning.
const SKELETON_CARDS: usize = 8;
const SKELETON_ROWS: usize = 10;

/// The frame every view shares
/// (32 px gutter, 1240 max width centered, serif 28/500 title). The
/// title's id is `content-title`.
pub fn page_frame(
    title: impl Into<SharedString>,
    children: impl IntoIterator<Item = AnyElement>,
) -> Div {
    page_frame_with(None, title, children)
}

/// [`page_frame`] with an element above the title (the Archive "All
/// briefings" link).
pub fn page_frame_with(
    lead: Option<AnyElement>,
    title: impl Into<SharedString>,
    children: impl IntoIterator<Item = AnyElement>,
) -> Div {
    let title: SharedString = title.into();
    div()
        .w_full()
        .max_w(px(MAX_WIDTH))
        .mx_auto()
        .pt(px(24.))
        .px(px(GUTTER))
        .pb(px(80.))
        .children(lead)
        .child(
            div()
                .id("content-title")
                .test_support()
                .aria_label(title.clone())
                .mb(px(6.))
                .font_family(theme::FONT_SERIF)
                .text_size(px(28.))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .children(children)
}

/// The muted line under the title. `id` is also
/// its accessible label's owner.
pub fn view_subtitle(
    id: &'static str,
    text: impl Into<SharedString>,
    t: &Tokens,
) -> impl IntoElement {
    let text: SharedString = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .mb(px(22.))
        .text_color(t.text_2)
        .child(text)
}

/// Columns of `repeat(auto-fill, minmax(320px, 1fr))` with a 14 px gap in
/// a container `width` px wide.
fn grid_columns(width: f32) -> u16 {
    (((width + GRID_GAP) / (CARD_MIN_WIDTH + GRID_GAP)).floor() as u16).max(1)
}

/// `strong` text inside a muted sentence.
fn rich(text: String, bold: Option<std::ops::Range<usize>>, t: &Tokens) -> StyledText {
    let style = HighlightStyle {
        color: Some(t.text),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    StyledText::new(text).with_highlights(bold.map(|r| (r, style)))
}

/// The summary line below the greeting; the second value is the same
/// sentence without markup.
fn subtitle(
    scanning: bool,
    articles: &[Article],
    show_tutorial: bool,
    t: &Tokens,
) -> (AnyElement, String) {
    let unread = articles.iter().filter(|a| !a.read).count();
    let sources = articles
        .iter()
        .map(|a| a.source.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let (text, bold) = if scanning {
        ("Scanning sources…".to_string(), None)
    } else if articles.is_empty() {
        if show_tutorial {
            ("Let's get you set up.".to_string(), None)
        } else {
            let text = "No briefing yet. Add sources, set feed URLs, then press Morning Briefing."
                .to_string();
            let start = text.find("Morning Briefing").unwrap_or(0);
            let end = start + "Morning Briefing".len();
            (text, Some(start..end))
        }
    } else {
        let lead = "Your briefing surfaced ";
        let n = format!("{} articles", articles.len());
        let plural = if sources == 1 { "" } else { "s" };
        let text = format!("{lead}{n} across {sources} source{plural} · {unread} unread.");
        (text, Some(lead.len()..lead.len() + n.len()))
    };
    let el = rich(text.clone(), bold, t).into_any_element();
    (el, text)
}

fn stat(id: &'static str, label: &'static str, value: String, last: bool, t: &Tokens) -> Div {
    let mut value_el = div()
        .id(id)
        .test_support()
        .aria_label(value.clone())
        .text_size(px(22.))
        .font_weight(FontWeight::MEDIUM)
        .child(value);
    if last {
        // `Last run`: 15 px mono in text-2.
        value_el = value_el
            .text_size(px(15.))
            .text_color(t.text_2)
            .font_family(theme::FONT_MONO);
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(px(4.))
        .px(px(18.))
        .py(px(14.))
        .when(!last, |d| d.border_r_1().border_color(t.border))
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.text_3)
                .child(label.to_uppercase()),
        )
        .child(value_el)
}

fn stat_strip(articles: &[Article], last_run: String, t: &Tokens) -> Div {
    div()
        .flex()
        .mb(px(22.))
        .overflow_hidden()
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
        .bg(t.surface)
        .child(stat(
            "stat-fetched",
            "Fetched",
            articles.len().to_string(),
            false,
            t,
        ))
        .child(stat(
            "stat-unread",
            "Unread",
            articles.iter().filter(|a| !a.read).count().to_string(),
            false,
            t,
        ))
        .child(stat(
            "stat-saved",
            "Saved",
            articles.iter().filter(|a| a.bookmarked).count().to_string(),
            false,
            t,
        ))
        .child(stat("stat-last-run", "Last run", last_run, true, t))
}

fn errors_banner(errors: &[SourceError], t: &Tokens) -> impl IntoElement {
    let head = format!(
        "{} source{} failed:",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" }
    );
    let names = errors
        .iter()
        .map(|e| e.source_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!("{head} {names}");
    let strong = HighlightStyle {
        color: Some(t.text),
        font_weight: Some(FontWeight::MEDIUM),
        ..Default::default()
    };
    div()
        .id("errors-banner")
        .test_support()
        .aria_label(text.clone())
        .mb(px(18.))
        .px(px(14.))
        .py(px(10.))
        .border_1()
        .border_color(t.border_strong)
        .rounded(t.radius)
        .bg(t.surface_2)
        .font_family(theme::FONT_MONO)
        .text_size(px(12.5))
        .text_color(t.text_2)
        .child(StyledText::new(text).with_highlights([(0..head.len(), strong)]))
}

/// One category chip (a pill).
fn chip(
    id: SharedString,
    label: &str,
    count: usize,
    dot: Option<gpui_kit::Hsla>,
    active: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    t: &Tokens,
) -> impl IntoElement {
    let (border_strong, text) = (t.border_strong, t.text);
    let pill = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(30.))
        .px(px(12.))
        .rounded_full()
        .border_1()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer();
    let pill = if active {
        pill.border_color(t.text).bg(t.text).text_color(t.bg)
    } else {
        pill.border_color(t.border)
            .bg(t.surface)
            .text_color(t.text_2)
            .hover(move |s| s.border_color(border_strong).text_color(text))
    };
    let count_color = if active {
        let bg = t.bg;
        hsla(bg.h, bg.s, bg.l, bg.a * 0.55)
    } else {
        t.text_3
    };
    pill.on_click(on_click)
        .test_support()
        .aria_label(format!("{label} {count}"))
        .children(
            dot.filter(|_| !active)
                .map(|c| div().flex_none().size(px(8.)).rounded_full().bg(c)),
        )
        .child(label.to_string())
        .child(
            div()
                .text_size(px(12.))
                .text_color(count_color)
                .font_features(theme::font_features(true))
                .child(count.to_string()),
        )
}

fn chips(
    categories: &[Category],
    articles: &[Article],
    active: &str,
    t: &Tokens,
) -> impl IntoElement {
    let pick = |id: String| {
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            let id = id.clone();
            AppModel::mutate(cx, |m| m.active_category = id)
        }
    };
    let mut row = div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .mb(px(22.))
        .pb(px(18.))
        .border_b_1()
        .border_color(t.border)
        .child(chip(
            "chip-all".into(),
            "All",
            articles.len(),
            None,
            active == "all",
            pick("all".into()),
            t,
        ));
    for c in categories {
        let count = articles.iter().filter(|a| a.category == c.id).count();
        row = row.child(chip(
            format!("chip-{}", c.id).into(),
            &c.label,
            count,
            t.dot_for(&c.accent),
            active == c.id,
            pick(c.id.clone()),
            t,
        ));
    }
    row
}

/// The bordered box around list rows.
fn list_box(t: &Tokens) -> Div {
    div()
        .overflow_hidden()
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
        .bg(t.surface)
}

/// The list's column header row.
fn list_head(t: &Tokens) -> Div {
    let label = |col: Col, text: &'static str| {
        cell(col)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text_3)
            .child(text.to_uppercase())
    };
    grid_row()
        .h(px(36.))
        .border_b_1()
        .border_color(t.border)
        .bg(t.surface_2)
        .child(cell(Col::Dot))
        .child(label(Col::Source, "Source"))
        .child(label(Col::Headline, "Headline"))
        .child(label(Col::Category, "Category"))
        .child(label(Col::Time, "Time"))
        .child(cell(Col::Actions))
}

/// The article list; virtualized above [`VIRTUAL_MIN`] rows.
fn list(
    articles: Vec<Article>,
    window: &Window,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> AnyElement {
    let head = list_head(t);
    if articles.len() <= VIRTUAL_MIN {
        return list_box(t)
            .id("briefing-list")
            .test_support()
            .child(head)
            .children(articles.iter().map(|a| Row::new(a, false)))
            .into_any_element();
    }
    let sizes = Rc::new(vec![size(px(1.), t.row_h); articles.len()]);
    // Leave room for the header (56), page padding, greeting, stats and
    // chips; never smaller than a few rows.
    let height = (window.viewport_size().height - px(300.)).max(px(300.));
    let articles = Rc::new(articles);
    let rows = v_virtual_list(
        cx.entity(),
        "briefing-vlist",
        sizes,
        move |_, range, _, _| {
            articles[range]
                .iter()
                .map(|a| Row::new(a, false))
                .collect::<Vec<_>>()
        },
    );
    list_box(t)
        .id("briefing-list")
        .test_support()
        .child(head)
        .child(
            div()
                .id("briefing-vlist")
                .test_support()
                .h(height)
                .child(rows),
        )
        .into_any_element()
}

fn skeleton_list(t: &Tokens) -> AnyElement {
    list_box(t)
        .id("briefing-list")
        .test_support()
        .children((0..SKELETON_ROWS).map(SkeletonRow::new))
        .into_any_element()
}

/// Renders the briefing page.
pub fn render(app: &AppModel, window: &Window, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let m = &app.model;
    let prefs = &m.prefs;
    let articles = &m.articles;
    let greeting = text::greeting(&m.display_name(app.account.as_ref()), Local::now());

    // First run: no briefing yet, no sources, nothing archived; held back
    // until hydration is complete so it can't flash before data arrives.
    let show_tutorial = app.hydrated()
        && !m.scanning
        && articles.is_empty()
        && m.sources.is_empty()
        && m.archives.is_empty();

    let (subtitle_el, subtitle_text) = subtitle(m.scanning, articles, show_tutorial, &t);
    let subtitle = div()
        .id("briefing-subtitle")
        .test_support()
        .aria_label(subtitle_text)
        .mb(px(22.))
        .text_color(t.text_2)
        .child(subtitle_el);

    let mut children: Vec<AnyElement> = vec![subtitle.into_any_element()];
    if show_tutorial {
        children.push(onboarding::render(app, cx).into_any_element());
    } else {
        let last_run = text::relative_time_ms(m.last_run_at_ms(), Utc::now().timestamp_millis());
        children.push(stat_strip(articles, last_run, &t).into_any_element());
    }
    if !m.briefing_errors.is_empty() && !m.scanning {
        children.push(errors_banner(&m.briefing_errors, &t).into_any_element());
    }
    if !m.categories.is_empty() {
        children.push(chips(&m.categories, articles, &m.active_category, &t).into_any_element());
    }

    let opts = CardOpts {
        show_accent: prefs.show_accent,
        use_serif: prefs.use_serif,
        read_only: false,
    };
    let grid = prefs.view_mode == ViewMode::Grid;
    let body = if m.scanning {
        if grid {
            grid_box("briefing-grid", window, prefs.sidebar_collapsed)
                .children((0..SKELETON_CARDS).map(SkeletonCard::new))
                .into_any_element()
        } else {
            skeleton_list(&t)
        }
    } else {
        let filtered: Vec<Article> = articles
            .iter()
            .filter(|a| m.active_category == "all" || a.category == m.active_category)
            .cloned()
            .collect();
        if grid {
            grid_box("briefing-grid", window, prefs.sidebar_collapsed)
                .children(filtered.iter().map(|a| Card::new(a, opts)))
                .into_any_element()
        } else {
            list(filtered, window, &t, cx)
        }
    };
    children.push(body);

    page_frame(greeting, children)
}

/// The card grid container, sized for the current window.
pub fn grid_box(
    id: impl Into<gpui_kit::ElementId>,
    window: &Window,
    sidebar_collapsed: bool,
) -> impl gpui_kit::ParentElement + IntoElement {
    let sidebar = if sidebar_collapsed {
        0.
    } else {
        sidebar::WIDTH
    };
    let avail = f32::from(window.viewport_size().width) - sidebar;
    let width = avail.min(MAX_WIDTH) - 2. * GUTTER;
    div()
        .id(id)
        .test_support()
        .grid()
        .grid_cols(grid_columns(width))
        .gap(px(GRID_GAP))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn grid_columns_follow_auto_fill() {
        assert_eq!(grid_columns(100.), 1);
        assert_eq!(grid_columns(320.), 1);
        assert_eq!(grid_columns(653.), 1);
        assert_eq!(grid_columns(654.), 2);
        // 1240 - 64: three 320 px columns and two gaps need 988.
        assert_eq!(grid_columns(1176.), 3);
        assert_eq!(grid_columns(988.), 3);
        assert_eq!(grid_columns(987.), 2);
    }
}

#[cfg(test)]
#[path = "briefing_tests.rs"]
mod tests;
