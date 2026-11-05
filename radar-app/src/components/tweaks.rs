//! Port of `src/components/Tweaks.jsx` and the Tweaks FAB in `App.jsx`:
//! a floating panel of visual prefs, bottom-right.

use super::icon::{Icon, IconName};
use super::icon_button;
use super::toggle::Toggle;
use crate::app_model::AppModel;
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled,
};
use radar_core::model::RadarModel;
use radar_core::types::{Accent, Density, Theme, ViewMode};

/// Distance of the FAB and the panel from the window's bottom-right corner.
const INSET: f32 = 20.;
const ACCENTS: [(Accent, &str); 5] = [
    (Accent::Camel, "Camel"),
    (Accent::Olive, "Olive"),
    (Accent::Rust, "Rust"),
    (Accent::Slate, "Slate"),
    (Accent::Ink, "Ink"),
];

/// The 40×40 button that opens the panel (hidden while it is open).
pub fn fab(theme: Theme, cx: &Context<Workspace>) -> impl IntoElement {
    let t = *theme::t(cx);
    let hover_text = t.text;
    div()
        .id("tweaks-fab")
        .test_support()
        .absolute()
        .right(px(INSET))
        .bottom(px(INSET))
        .flex()
        .items_center()
        .justify_center()
        .size(px(40.))
        .rounded(t.radius)
        .bg(t.surface)
        .border_1()
        .border_color(t.border_strong)
        .shadow(theme::shadow_md(theme))
        // The inline background beats `.iconbtn:hover`; only the color moves.
        .text_color(t.text_2)
        .hover(move |s| s.text_color(hover_text))
        .cursor_pointer()
        .occlude()
        .on_click(cx.listener(|ws, _, _, cx| {
            ws.tweaks_open = true;
            cx.notify();
        }))
        .child(Icon::new(IconName::Sliders).size(px(16.)))
}

pub fn panel(app: &AppModel, cx: &Context<Workspace>) -> impl IntoElement {
    let t = *theme::t(cx);
    let p = &app.model.prefs;

    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(12.))
        .border_b_1()
        .border_color(t.border)
        .bg(t.surface_2)
        .child(Icon::new(IconName::Sliders).size(px(14.)))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text_2)
                .child("TWEAKS"),
        )
        .child(
            icon_button("tweaks-close", IconName::X, 13., false, &t)
                .ml_auto()
                .size(px(24.))
                .on_click(cx.listener(|ws, _, _, cx| {
                    ws.tweaks_open = false;
                    cx.notify();
                }))
                .test_support(),
        );

    let theme_seg = seg(
        &t,
        [
            (
                "tweaks-theme-light",
                "Light",
                Some(IconName::Sun),
                p.theme == Theme::Light,
                |m| m.prefs.theme = Theme::Light,
            ),
            (
                "tweaks-theme-dark",
                "Dark",
                Some(IconName::Moon),
                p.theme == Theme::Dark,
                |m| m.prefs.theme = Theme::Dark,
            ),
        ],
    );
    let view_seg = seg(
        &t,
        [
            (
                "tweaks-view-grid",
                "Grid",
                Some(IconName::Grid),
                p.view_mode == ViewMode::Grid,
                |m| m.prefs.view_mode = ViewMode::Grid,
            ),
            (
                "tweaks-view-list",
                "List",
                Some(IconName::List),
                p.view_mode == ViewMode::List,
                |m| m.prefs.view_mode = ViewMode::List,
            ),
        ],
    );
    let density_seg = seg(
        &t,
        [
            (
                "tweaks-density-cozy",
                "Cozy",
                None,
                p.density == Density::Cozy,
                |m| m.prefs.density = Density::Cozy,
            ),
            (
                "tweaks-density-compact",
                "Compact",
                None,
                p.density == Density::Compact,
                |m| m.prefs.density = Density::Compact,
            ),
        ],
    );
    let swatches = div()
        .flex()
        .gap(px(6.))
        .children(ACCENTS.map(|(accent, name)| {
            let active = p.accent == accent;
            let shadow = if active {
                // `0 0 0 2px surface, 0 0 0 3px text`, outermost first.
                vec![
                    theme::ring(3., t.text, false),
                    theme::ring(2., t.surface, false),
                ]
            } else {
                vec![theme::ring(1., t.border, false)]
            };
            div()
                .id(SharedString::from(format!("tweaks-accent-{name}")))
                .test_support()
                .aria_label(name)
                .size(px(24.))
                .rounded(px(5.))
                .bg(theme::accent_swatch(accent, p.theme))
                .shadow(shadow)
                .cursor_pointer()
                .on_click(move |_, _, cx| AppModel::mutate(cx, |m| m.prefs.accent = accent))
        }));

    let body = div()
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(group("Theme", theme_seg, &t))
        .child(group("View", view_seg, &t))
        .child(group("Density", density_seg, &t))
        .child(group("Accent", swatches, &t))
        .child(toggle_row(
            "Serif headlines",
            Toggle::new("tweaks-serif", p.use_serif)
                .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.use_serif ^= true)),
        ))
        .child(toggle_row(
            "Accent bars on cards",
            Toggle::new("tweaks-accent-bars", p.show_accent)
                .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.show_accent ^= true)),
        ))
        .child(toggle_row(
            "Sidebar collapsed",
            Toggle::new("tweaks-sidebar-collapsed", p.sidebar_collapsed)
                .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.sidebar_collapsed ^= true)),
        ));

    div()
        .id("tweaks-panel")
        .test_support()
        .absolute()
        .right(px(INSET))
        .bottom(px(INSET))
        .w(px(300.))
        .bg(t.surface)
        .border_1()
        .border_color(t.border_strong)
        .rounded(t.radius_lg)
        .shadow(theme::shadow_panel())
        .overflow_hidden()
        .occlude()
        .child(head)
        .child(body)
}

/// A prefs edit for a segment button (a plain fn, so it is `Copy`).
type Setter = fn(&mut RadarModel);

/// `style.group`: an uppercase label over its control.
fn group(label: &str, control: impl IntoElement, t: &Tokens) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text_3)
                .child(label.to_uppercase()),
        )
        .child(control)
}

/// `Seg`: a two-option segmented control.
fn seg(
    t: &Tokens,
    options: [(&'static str, &'static str, Option<IconName>, bool, Setter); 2],
) -> Div {
    let (surface, text, border) = (t.surface, t.text, t.border);
    div()
        .flex()
        .border_1()
        .border_color(t.border)
        .rounded(px(5.))
        .p(px(2.))
        .bg(t.surface_2)
        .children(options.map(|(id, label, icon, active, apply)| {
            let btn = div()
                .id(id)
                .test_support()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .py(px(5.))
                .px(px(8.))
                .rounded(px(3.))
                .text_size(px(12.))
                .text_color(t.text_2)
                .cursor_pointer()
                .on_click(move |_, _, cx| AppModel::mutate(cx, apply))
                .children(icon.map(|i| Icon::new(i).size(px(12.))))
                .child(label);
            if active {
                btn.bg(surface)
                    .text_color(text)
                    .shadow(vec![theme::ring(1., border, false)])
            } else {
                btn
            }
        }))
}

/// `style.toggleRow`.
fn toggle_row(label: &'static str, toggle: Toggle) -> Div {
    div()
        .flex()
        .justify_between()
        .items_center()
        .text_size(px(12.5))
        .child(label)
        .child(toggle)
}
