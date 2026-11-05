//! The list of past briefings and the drill-in to one snapshot. The
//! drill-in selection is local view state ([`ArchiveView`]), reset when the
//! user leaves the view.
//!
//! Snapshot cards are read-only (`read_only: true`; toggles do nothing)
//! and scoped `archive-<id>`, so an article that is also in the live
//! briefing or Saved never shares element ids with it.

use super::briefing::{grid_box, page_frame, page_frame_with, view_subtitle};
use super::empty_state::empty_state;
use crate::app_model::AppModel;
use crate::components::card::{Card, CardOpts};
use crate::components::icon::{Icon, IconName};
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use chrono::Local;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use radar_core::text::format_run_at;
use radar_core::types::Archive;

/// Local state of the Archive view.
#[derive(Debug, Default)]
pub struct ArchiveView {
    /// The snapshot being viewed; `None` shows the list.
    pub selected_id: Option<i64>,
}

/// Element-id scope of a snapshot's cards (`archive-3/card-7`).
pub fn scope(archive_id: i64) -> String {
    format!("archive-{archive_id}")
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

fn unread(a: &Archive) -> usize {
    a.articles.iter().filter(|x| !x.read).count()
}

/// A muted row cell: 80 px, mono 12, right aligned.
fn muted(id: String, text: String, t: &Tokens) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .test_support()
        .aria_label(text.clone())
        .w(px(80.))
        .flex_none()
        .text_right()
        .font_family(theme::FONT_MONO)
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text)
}

fn list_row(b: &Archive, last: bool, t: &Tokens, cx: &Context<Workspace>) -> impl IntoElement {
    let id = b.id;
    let saved = b.articles.iter().filter(|a| a.bookmarked).count();
    let date = format_run_at(&b.run_at, Local::now());
    let surface_2 = t.surface_2;
    let row = div()
        .id(SharedString::from(format!("archive-row-{id}")))
        .test_support()
        .aria_label(date.clone())
        .flex()
        .items_center()
        .gap(px(14.))
        .h(px(52.))
        .px(px(18.))
        .text_size(px(13.5))
        .cursor_pointer()
        .hover(move |s| s.bg(surface_2))
        .on_click(cx.listener(move |ws, _, _, cx| {
            ws.archive.selected_id = Some(id);
            cx.notify();
        }));
    let row = if last {
        row
    } else {
        row.border_b_1().border_color(t.border)
    };
    row.child(
        div()
            .id(SharedString::from(format!("archive-row-{id}-date")))
            .test_support()
            .aria_label(date.clone())
            .flex_1()
            .min_w_0()
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text)
            .child(date),
    )
    .child(muted(
        format!("archive-row-{id}-items"),
        format!("{} items", b.articles.len()),
        t,
    ))
    .child(muted(
        format!("archive-row-{id}-unread"),
        format!("{} unread", unread(b)),
        t,
    ))
    .child(muted(
        format!("archive-row-{id}-saved"),
        format!("{saved} saved"),
        t,
    ))
    .child(
        div()
            .w(px(20.))
            .flex_none()
            .child(Icon::new(IconName::ChevronRight).size(px(14.))),
    )
}

fn back_link(t: &Tokens, cx: &Context<Workspace>) -> AnyElement {
    div()
        .id("archive-back")
        .test_support()
        .aria_label("All briefings")
        .flex()
        .items_center()
        .gap(px(6.))
        .mb(px(14.))
        .text_size(px(13.))
        .text_color(t.text_2)
        .cursor_pointer()
        .on_click(cx.listener(|ws, _, _, cx| {
            ws.archive.selected_id = None;
            cx.notify();
        }))
        .child(Icon::new(IconName::ChevronLeft).size(px(14.)))
        .child("All briefings")
        .into_any_element()
}

fn detail(
    b: &Archive,
    app: &AppModel,
    window: &Window,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> Div {
    let n = b.articles.len();
    let mut meta = format!("{n} article{} · {} unread", plural(n, "", "s"), unread(b));
    if !b.errors.is_empty() {
        let e = b.errors.len();
        meta.push_str(&format!(" · {e} source{} failed", plural(e, "", "s")));
    }
    let meta = div()
        .id("archive-detail-meta")
        .test_support()
        .aria_label(meta.clone())
        .mb(px(18.))
        .font_family(theme::FONT_MONO)
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(meta);
    let opts = CardOpts {
        show_accent: true,
        use_serif: app.model.prefs.use_serif,
        read_only: true,
    };
    let scope = scope(b.id);
    let grid = grid_box("archive-grid", window, app.model.prefs.sidebar_collapsed).children(
        b.articles
            .iter()
            .map(|a| Card::new(a, opts).scope(scope.clone())),
    );
    page_frame_with(
        Some(back_link(t, cx)),
        format_run_at(&b.run_at, Local::now()),
        [meta.into_any_element(), grid.into_any_element()],
    )
}

pub fn render(view: &ArchiveView, app: &AppModel, window: &Window, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let archives = &app.model.archives;

    if archives.is_empty() {
        return page_frame(
            "Archive",
            [
                view_subtitle(
                    "archive-subtitle",
                    "Past briefings. Goes back 90 days by default.",
                    &t,
                )
                .into_any_element(),
                empty_state(
                    "archive-empty",
                    IconName::Archive,
                    "Your archive begins with your first briefing",
                    "Every completed Morning Briefing is kept here so you can look back and see what the week was really about.",
                    &t,
                )
                .into_any_element(),
            ],
        );
    }

    if let Some(b) = view
        .selected_id
        .and_then(|id| archives.iter().find(|a| a.id == id))
    {
        return detail(b, app, window, &t, cx);
    }

    let n = archives.len();
    let list = div()
        .id("archive-list")
        .test_support()
        .overflow_hidden()
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
        .bg(t.surface)
        .children(
            archives
                .iter()
                .enumerate()
                .map(|(i, b)| list_row(b, i + 1 == n, &t, cx)),
        );
    page_frame(
        "Archive",
        [
            view_subtitle(
                "archive-subtitle",
                format!("{n} briefing{}. Newest first.", plural(n, "", "s")),
                &t,
            )
            .into_any_element(),
            list.into_any_element(),
        ],
    )
}
