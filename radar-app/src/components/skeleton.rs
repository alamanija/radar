//! Loading placeholders: skeleton cards and the briefing list's skeleton
//! rows.
//!
//! GPUI gradients have two stops and no background scrolling, so rather
//! than a moving shimmer the bars pulse between `surface_2` and `border`
//! over 1.4 s.

use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, ease_in_out, px, relative, Animation, AnimationExt as _, AnyElement, App, Div, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled,
    Window,
};
use std::time::Duration;

const PERIOD: Duration = Duration::from_millis(1400);

fn mix(a: Hsla, b: Hsla, d: f32) -> Hsla {
    let l = |x: f32, y: f32| x + (y - x) * d;
    gpui_kit::hsla(l(a.h, b.h), l(a.s, b.s), l(a.l, b.l), l(a.a, b.a))
}

/// One skeleton bar. `w`/`h` in px; `w = None` fills the width.
fn skel(id: SharedString, t: &Tokens, w: Option<f32>, h: f32, radius: f32) -> AnyElement {
    let (from, to) = (t.surface_2, t.border);
    let bar = div()
        .id(id.clone())
        .flex_none()
        .h(px(h))
        .rounded(px(radius));
    let bar = match w {
        Some(w) => bar.w(px(w)),
        None => bar.w_full(),
    };
    bar.with_animation(
        id,
        Animation::new(PERIOD)
            .repeat()
            .with_easing(|d| ease_in_out(if d < 0.5 { d * 2. } else { 2. - d * 2. })),
        move |el, d| el.bg(mix(from, to, d)),
    )
    .into_any_element()
}

/// A bar `pct` of the parent's width.
fn skel_pct(id: SharedString, t: &Tokens, pct: f32, h: f32) -> AnyElement {
    let (from, to) = (t.surface_2, t.border);
    div()
        .id(id.clone())
        .h(px(h))
        .w(relative(pct))
        .rounded(px(4.))
        .with_animation(
            id,
            Animation::new(PERIOD)
                .repeat()
                .with_easing(|d| ease_in_out(if d < 0.5 { d * 2. } else { 2. - d * 2. })),
            move |el, d| el.bg(mix(from, to, d)),
        )
        .into_any_element()
}

/// A placeholder grid card.
#[derive(IntoElement)]
pub struct SkeletonCard {
    index: usize,
}

impl SkeletonCard {
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

impl RenderOnce for SkeletonCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *theme::t(cx);
        let i = self.index;
        let s = |part: &str| SharedString::from(format!("skeleton-card-{i}-{part}"));
        div()
            .id(SharedString::from(format!("skeleton-card-{i}")))
            .test_support()
            .flex()
            .flex_col()
            .min_h(px(180.))
            .overflow_hidden()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(t.radius)
            .child(skel(s("thumb"), &t, None, 96., 0.))
            .child(
                div()
                    .flex_1()
                    .p(t.density_pad)
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .mb(px(12.))
                            .child(skel(s("meta-a"), &t, Some(80.), 10., 4.))
                            .child(skel(s("meta-b"), &t, Some(50.), 10., 4.)),
                    )
                    .child(div().mb(px(8.)).child(skel_pct(s("t1"), &t, 0.92, 14.)))
                    .child(div().mb(px(14.)).child(skel_pct(s("t2"), &t, 0.70, 14.)))
                    .child(div().mb(px(6.)).child(skel(s("s1"), &t, None, 10., 4.)))
                    .child(skel_pct(s("s2"), &t, 0.60, 10.)),
            )
    }
}

/// One loading row of list mode: dot, source, headline, time.
#[derive(IntoElement)]
pub struct SkeletonRow {
    index: usize,
}

impl SkeletonRow {
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

impl RenderOnce for SkeletonRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *theme::t(cx);
        let i = self.index;
        let s = |part: &str| SharedString::from(format!("skeleton-row-{i}-{part}"));
        let row: Div = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(16.))
            .h(px(44.))
            .border_b_1()
            .border_color(t.border);
        row.id(SharedString::from(format!("skeleton-row-{i}")))
            .test_support()
            .child(skel(s("dot"), &t, Some(10.), 10., 5.))
            .child(skel(s("source"), &t, Some(110.), 10., 4.))
            .child(div().flex_1().child(skel(s("title"), &t, None, 10., 4.)))
            .child(skel(s("time"), &t, Some(60.), 10., 4.))
    }
}
