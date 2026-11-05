//! Pieces shared by the Sources and Categories pages: the `.pill` button,
//! the bordered table block, the edit-row frame, the error line and the
//! text input styled like `sourcesStyles.input`.

use crate::components::icon::{Icon, IconName};
use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, App, AppContext as _, ClickEvent, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled, Window,
};

pub type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// How a `.pill` is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillStyle {
    /// `.pill`.
    Plain,
    /// `.pill.active`.
    Active,
    /// Inline `background: var(--text); color: var(--bg)`: the "Add ..."
    /// buttons. The inline style also beats the `:hover` rule.
    Primary,
    /// Inline `color: var(--dot-rust)`: Delete.
    Danger,
}

/// A `.pill` button. `on_click: None` is a `disabled` button: it looks the
/// same (the CSS has no `:disabled` rule) but does nothing. `title` is the
/// HTML `title` attribute: a hover tooltip, also exposed as the accessible
/// description (`aria_description`) so it can be observed in tests.
pub struct Pill {
    pub id: ElementId,
    pub label: SharedString,
    pub aria: Option<SharedString>,
    pub style: PillStyle,
    pub icon: Option<IconName>,
    pub icon_size: f32,
    pub title: Option<SharedString>,
    pub on_click: Option<Click>,
    /// Inline `opacity: 0.5; cursor: not-allowed` (a visibly disabled pill).
    pub dimmed: bool,
}

impl Pill {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            aria: None,
            style: PillStyle::Plain,
            icon: None,
            icon_size: 13.,
            title: None,
            on_click: None,
            dimmed: false,
        }
    }

    pub fn aria(mut self, label: impl Into<SharedString>) -> Self {
        self.aria = Some(label.into());
        self
    }

    pub fn style(mut self, style: PillStyle) -> Self {
        self.style = style;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = size;
        self
    }

    /// Disabled with the inline `opacity: 0.5; cursor: not-allowed` look
    /// (any click handler is ignored).
    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }

    pub fn title(mut self, title: Option<String>) -> Self {
        self.title = title.map(SharedString::from);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Enabled when `enabled`, otherwise the handler is dropped.
    pub fn enabled(mut self, enabled: bool) -> Self {
        if !enabled {
            self.on_click = None;
        }
        self
    }

    pub fn render(
        self,
        t: &Tokens,
        children: impl IntoIterator<Item = gpui_kit::AnyElement>,
    ) -> impl IntoElement {
        let (border_strong, text) = (t.border_strong, t.text);
        let el = div()
            .id(self.id)
            .test_support()
            .aria_label(self.aria.unwrap_or_else(|| self.label.clone()))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .px(px(12.))
            .rounded_full()
            .border_1()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer();
        let el = match self.style {
            PillStyle::Plain => el
                .border_color(t.border)
                .bg(t.surface)
                .text_color(t.text_2)
                .hover(move |s| s.border_color(border_strong).text_color(text)),
            PillStyle::Active => el.border_color(t.text).bg(t.text).text_color(t.bg),
            PillStyle::Primary => el.border_color(t.text).bg(t.text).text_color(t.bg),
            PillStyle::Danger => {
                let rust = t.dot_for("rust").unwrap_or(t.text_2);
                el.border_color(t.border)
                    .bg(t.surface)
                    .text_color(rust)
                    .hover(move |s| s.border_color(border_strong))
            }
        };
        let el = match self.title {
            Some(title) => {
                let tip = title.clone();
                el.aria_description(title)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            }
            None => el,
        };
        let (el, on_click) = if self.dimmed {
            (el.opacity(0.5).cursor_not_allowed(), None)
        } else {
            (el, self.on_click)
        };
        el.children(self.icon.map(|i| Icon::new(i).size(px(self.icon_size))))
            .child(self.label)
            .children(children)
            .when_some(on_click, |el, f| el.on_click(f))
    }
}

/// `block`: the bordered, rounded table container.
pub fn block(t: &Tokens) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
        .overflow_hidden()
        .mb(px(22.))
}

/// The padded `surface-2` frame of an edit row.
pub fn edit_frame(t: &Tokens, last: bool) -> Div {
    let frame = div()
        .flex()
        .flex_col()
        .px(px(18.))
        .py(px(14.))
        .bg(t.surface_2);
    if last {
        frame
    } else {
        frame.border_b_1().border_color(t.border)
    }
}

/// `{ fontSize: 11, color: var(--dot-rust), marginTop: 8 }`.
pub fn error_line(id: &'static str, text: String, t: &Tokens) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .mt(px(8.))
        .text_size(px(11.))
        .text_color(t.dot_for("rust").unwrap_or(t.text_2))
        .child(text)
}

/// Text size of `sourcesStyles.input` / `select`.
pub const INPUT_TEXT: f32 = 13.;

/// A new single-line input with `placeholder` and `value`.
pub fn new_input(
    placeholder: &'static str,
    value: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| {
        let mut st = InputState::new(window, cx).placeholder(placeholder);
        st.set_value(value, window, cx);
        st
    })
}

/// `sourcesStyles.input`: 28 px high, 8 px padding, 1 px border,
/// `--radius`, surface fill, 13 px text (`mono`: 12 px JetBrains Mono).
pub fn text_input(
    id: &'static str,
    label: &'static str,
    state: &Entity<InputState>,
    mono: bool,
    t: &Tokens,
) -> impl IntoElement {
    // `Size::Size(px)` makes the kit draw text at 0.875 x px: 13 px / 0.875.
    let input = Input::new(state)
        .with_size(Size::Size(px(INPUT_TEXT / 0.875)))
        .id(id)
        .aria_label(label);
    // `Styled::h` (the inherent `Input::h` only sizes multi-line inputs).
    let input = Styled::h(input, px(28.))
        .w_full()
        .px(px(8.))
        .rounded(t.radius)
        .border_color(t.border)
        .bg(t.surface)
        .text_color(t.text);
    if mono {
        input.font_family(theme::FONT_MONO).text_size(px(12.))
    } else {
        input.font_family(theme::FONT_SANS).text_size(px(13.))
    }
}
