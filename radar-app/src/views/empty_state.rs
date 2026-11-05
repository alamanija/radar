//! Port of `src/views/EmptyState.jsx`: a dashed placeholder box with an
//! icon, a serif title and a message. Saved and Archive use it.

use crate::components::icon::{Icon, IconName};
use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled,
};

/// `EmptyState({ icon, title, message })`. `id` names the element (its
/// accessible label is the title).
pub fn empty_state(
    id: impl Into<SharedString>,
    icon: IconName,
    title: &'static str,
    message: &'static str,
    t: &Tokens,
) -> impl IntoElement {
    div()
        .id(id.into())
        .test_support()
        .aria_label(title)
        .flex()
        .flex_col()
        .items_center()
        .py(px(60.))
        .px(px(24.))
        .border_1()
        .border_dashed()
        .border_color(t.border_strong)
        .rounded(t.radius)
        .bg(t.surface)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(44.))
                .mb(px(14.))
                .rounded(px(10.))
                .bg(t.surface_2)
                .text_color(t.text_3)
                .child(Icon::new(icon).size(px(20.))),
        )
        .child(
            div()
                .mb(px(6.))
                .font_family(theme::FONT_SERIF)
                .text_size(px(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
                .child(title),
        )
        .child(
            div()
                .text_size(px(13.5))
                .text_color(t.text_2)
                .child(message),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::testing::host;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{px, TestAppContext};

    #[gpui_kit::test]
    fn empty_state_shows_title_and_fits_its_parent(cx: &mut TestAppContext) {
        let h = host(cx, 400., |_, cx| {
            let t = *theme::t(cx);
            empty_state(
                "empty",
                IconName::Inbox,
                "Nothing here",
                "Add something.",
                &t,
            )
            .into_any_element()
        });
        h.with_window(cx, |window| {
            let e = window.find("empty");
            assert_eq!(e.label(), Some("Nothing here"));
            assert_eq!(e.bounds().size.width, px(400.));
        });
    }
}
