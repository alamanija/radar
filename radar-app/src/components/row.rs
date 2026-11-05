//! One article as a list row. Same event model as
//! [`Card`](super::card::Card).
//!
//! The columns (20, 160, flexible, 120, 80 and 60 px; gap 14) are a flex
//! row; [`Col`], [`cell`] and [`grid_row`] are shared with the list header
//! of the briefing view.

use super::card::{apply_event, CardEvent, CardHandler};
use super::icon::IconName;
use super::icon_button;
use crate::theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, App, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use radar_core::types::Article;
use std::rc::Rc;

/// Widths of the fixed columns; the headline column (third) takes the rest.
const COL_DOT: f32 = 20.;
const COL_SOURCE: f32 = 160.;
const COL_CATEGORY: f32 = 120.;
const COL_TIME: f32 = 80.;
const COL_ACTIONS: f32 = 60.;
const GAP: f32 = 14.;

/// Which part of the grid a [`cell`] belongs to.
#[derive(Clone, Copy)]
pub enum Col {
    Dot,
    Source,
    Headline,
    Category,
    Time,
    Actions,
}

/// A cell of the row grid, sized like its column.
pub fn cell(col: Col) -> Div {
    let d = div().flex().items_center().min_w_0();
    match col {
        Col::Dot => d.w(px(COL_DOT)).flex_none(),
        Col::Source => d.w(px(COL_SOURCE)).flex_none(),
        Col::Headline => d.flex_1(),
        Col::Category => d.w(px(COL_CATEGORY)).flex_none(),
        Col::Time => d.w(px(COL_TIME)).flex_none().justify_end(),
        Col::Actions => d.w(px(COL_ACTIONS)).flex_none().justify_end().gap(px(2.)),
    }
}

/// The row-shaped flex container (the row's padding and gap).
pub fn grid_row() -> Div {
    div().flex().items_center().px(px(16.)).gap(px(GAP))
}

#[derive(IntoElement)]
pub struct Row {
    article: Article,
    read_only: bool,
    on_event: Option<CardHandler>,
}

impl Row {
    pub fn new(article: &Article, read_only: bool) -> Self {
        Self {
            article: article.clone(),
            read_only,
            on_event: None,
        }
    }

    #[allow(dead_code)] // The archive view routes events through its own handler
    pub fn on_event(mut self, f: impl Fn(CardEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_event = Some(Rc::new(f));
        self
    }
}

pub fn part_id(id: u64, part: &str) -> SharedString {
    if part.is_empty() {
        format!("row-{id}").into()
    } else {
        format!("row-{id}-{part}").into()
    }
}

impl RenderOnce for Row {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *theme::t(cx);
        let Row {
            article: a,
            read_only,
            on_event,
        } = self;
        let emit = Rc::new(
            move |ev: CardEvent, w: &mut Window, cx: &mut App, a: &Article| match &on_event {
                Some(h) => h(ev, w, cx),
                None => apply_event(cx, a, ev, read_only),
            },
        );
        let bookmarked = a.bookmarked;
        let group: SharedString = format!("row-group-{}", a.id).into();
        let surface_2 = t.surface_2;

        let bookmark = {
            let (emit, a) = (emit.clone(), a.clone());
            let button = icon_button(
                part_id(a.id, "bookmark"),
                if a.bookmarked {
                    IconName::BookmarkFilled
                } else {
                    IconName::Bookmark
                },
                13.,
                false,
                &t,
            )
            .size(px(26.))
            .test_support()
            .aria_label(if a.bookmarked {
                "Remove bookmark"
            } else {
                "Bookmark"
            })
            .on_click(move |_, w, cx| {
                cx.stop_propagation();
                emit(CardEvent::ToggleBookmark, w, cx, &a)
            });
            // Hidden until the row is hovered, unless bookmarked.
            if bookmarked {
                button
            } else {
                button
                    .opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
            }
        };

        grid_row()
            .id(part_id(a.id, ""))
            .test_support()
            .group(group)
            .h(t.row_h)
            .border_b_1()
            .border_color(t.border)
            .text_size(px(13.5))
            .cursor_pointer()
            .opacity(if a.read { 0.6 } else { 1. })
            .hover(move |s| s.bg(surface_2))
            .on_click({
                let a = a.clone();
                move |_, w, cx| emit(CardEvent::ToggleRead, w, cx, &a)
            })
            .child(
                cell(Col::Dot).children(
                    t.dot_for(&a.accent)
                        .map(|c| div().flex_none().size(px(8.)).rounded_full().bg(c)),
                ),
            )
            .child(
                cell(Col::Source)
                    .font_family(theme::FONT_MONO)
                    .text_size(px(12.5))
                    .text_color(t.text_2)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(div().min_w_0().truncate().child(a.source.clone())),
            )
            .child(
                cell(Col::Headline)
                    .text_color(t.text)
                    .font_weight(if a.read {
                        FontWeight::NORMAL
                    } else {
                        FontWeight::MEDIUM
                    })
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(div().min_w_0().truncate().child(a.title.clone())),
            )
            .child(
                cell(Col::Category)
                    .text_size(px(11.5))
                    .text_color(t.text_3)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(a.category_label.to_uppercase()),
                    ),
            )
            .child(
                cell(Col::Time)
                    .font_family(theme::FONT_MONO)
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(a.published.clone()),
            )
            .child(cell(Col::Actions).child(bookmark))
    }
}
