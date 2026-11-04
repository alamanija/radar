//! Root view of the main window. Task 8 fills in the shell (sidebar,
//! header, views); for now it paints the themed frame.

use crate::app_model::AppModel;
use crate::theme;
use gpui_kit::{div, Context, IntoElement, Render, Styled, Window};

pub struct Workspace;

impl Workspace {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Re-render on any model change (theme tokens are re-applied by
        // `update_model` before it notifies).
        cx.observe(&AppModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme::t(cx);
        div()
            .size_full()
            .bg(t.bg)
            .text_color(t.text)
            .font_family(theme::FONT_SANS)
    }
}
