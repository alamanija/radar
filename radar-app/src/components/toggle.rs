//! A 30×18 switch.

use crate::theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, StatefulInteractiveElement as _, Styled, Window,
};

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(gpui_kit::IntoElement)]
pub struct Toggle {
    id: ElementId,
    on: bool,
    on_click: Option<Handler>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            on_click: None,
        }
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::t(cx);
        let track = if self.on { t.accent } else { t.border_strong };
        let mut el = div()
            .id(self.id)
            .test_support()
            .role(Role::Switch)
            .aria_toggled(self.on.into())
            .relative()
            .flex_none()
            .w(px(30.))
            .h(px(18.))
            .rounded(px(10.))
            .bg(track)
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if self.on { 14. } else { 2. }))
                    .size(px(14.))
                    .rounded_full()
                    .bg(t.surface),
            );
        if let Some(f) = self.on_click {
            el = el.on_click(f);
        }
        el
    }
}
