//! Shell components (ports of `src/components/*.jsx`).
//!
//! Each shell part is a function the [`Workspace`](crate::workspace::Workspace)
//! calls from its render with the current [`AppModel`](crate::app_model::AppModel).
//! Model changes go through `AppModel::mutate`; workspace-local state
//! (search, Tweaks, sources intent) through `cx.listener`.

pub mod header;
pub mod icon;
pub mod sidebar;
pub mod toggle;
pub mod tweaks;

use crate::theme::Tokens;
use gpui_kit::{
    div, px, Div, ElementId, InteractiveElement as _, ParentElement as _, Stateful, Styled,
};
use icon::{Icon, IconName};

/// The `.iconbtn` class of `styles.css`: 32×32, `--radius`, text-2;
/// hover: surface-2 and text; `.active`: accent-text on accent-soft (and
/// no hover change, as `.active` wins the CSS tie).
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    icon_size: f32,
    active: bool,
    t: &Tokens,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(t.radius)
        .cursor_pointer()
        .child(Icon::new(icon).size(px(icon_size)));
    if active {
        el.text_color(t.accent_text).bg(t.accent_soft)
    } else {
        let (hover_bg, hover_text) = (t.surface_2, t.text);
        el.text_color(t.text_2)
            .hover(move |s| s.bg(hover_bg).text_color(hover_text))
    }
}
