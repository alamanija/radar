//! Port of `src/views/CategoriesView.jsx`: the category list with inline
//! add and edit rows, accent swatches, and delete blocked while sources
//! still use the category.
//!
//! State lives on the `Workspace` ([`CategoriesView`]) and is dropped when
//! another view is shown. The edit row's draft is held by two gpui-kit
//! `Input`s plus the chosen accent name.

use super::briefing::{page_frame, view_subtitle};
use super::form::{block, edit_frame, error_line, new_input, text_input, Pill, PillStyle};
use crate::app_model::AppModel;
use crate::components::icon::{Icon, IconName};
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, relative, Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use radar_core::model::CategoryPatch;
use radar_core::types::Category;

/// `ACCENTS`: the swatches, in order.
pub const ACCENTS: [&str; 6] = ["olive", "rust", "slate", "plum", "sand", "ink"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    New,
    Category(String),
}

/// The open edit row and its draft.
pub struct Editing {
    pub target: Target,
    pub label: Entity<InputState>,
    pub description: Entity<InputState>,
    pub accent: String,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct CategoriesView {
    pub editing: Option<Editing>,
}

fn source_count(app: &AppModel, category: &str) -> usize {
    app.model
        .sources
        .iter()
        .filter(|s| s.category == category)
        .count()
}

impl CategoriesView {
    /// `startAdd` / `startEdit`: opens the edit row, replacing any open one.
    pub fn open(&mut self, target: Target, window: &mut Window, cx: &mut Context<Workspace>) {
        let (label, description, accent) = match &target {
            Target::New => (String::new(), String::new(), "olive".to_string()),
            Target::Category(id) => {
                let app = AppModel::global(cx).read(cx);
                match app.model.categories.iter().find(|c| &c.id == id) {
                    Some(c) => (c.label.clone(), c.description.clone(), c.accent.clone()),
                    None => return,
                }
            }
        };
        self.editing = Some(Editing {
            target,
            label: new_input("Category name", &label, window, cx),
            description: new_input(
                "Description — helps Claude pick this category",
                &description,
                window,
                cx,
            ),
            accent,
            error: None,
        });
        cx.notify();
    }

    /// `attemptSave`. ("Pick an accent" cannot happen here: the draft
    /// always holds one of the six swatches.)
    fn save(&mut self, cx: &mut Context<Workspace>) {
        let Some(ed) = self.editing.as_mut() else {
            return;
        };
        let label = ed.label.read(cx).value().trim().to_string();
        let description = ed.description.read(cx).value().trim().to_string();
        if label.is_empty() {
            ed.error = Some("Label is required".into());
            cx.notify();
            return;
        }
        let accent = ed.accent.clone();
        let target = ed.target.clone();
        self.editing = None;
        AppModel::mutate(cx, |m| match target {
            Target::New => {
                m.add_category(label, description, accent);
            }
            Target::Category(id) => m.update_category(
                &id,
                CategoryPatch {
                    label: Some(label),
                    description: Some(description),
                    accent: Some(accent),
                },
            ),
        });
        cx.notify();
    }

    /// `handleDelete`: refused while sources use the category.
    fn delete(&mut self, cx: &mut Context<Workspace>) {
        let Some(Target::Category(id)) = self.editing.as_ref().map(|e| e.target.clone()) else {
            return;
        };
        if source_count(AppModel::global(cx).read(cx), &id) > 0 {
            return;
        }
        self.editing = None;
        let _ = AppModel::mutate(cx, |m| m.delete_category(&id));
        cx.notify();
    }
}

/// Head and body rows share `20px 1fr 2fr 80px 44px` with a 14 px gap.
fn grid_row() -> Div {
    div().flex().items_center().gap(px(14.)).px(px(18.))
}

/// `1fr` / `2fr` column.
fn fr(n: f32) -> Div {
    let mut d = div().flex_basis(relative(0.)).min_w_0();
    d.style().flex_grow = Some(n);
    d
}

fn head_row(t: &Tokens) -> impl IntoElement {
    grid_row()
        .py(px(10.))
        .border_b_1()
        .border_color(t.border)
        .bg(t.surface_2)
        .text_size(px(11.))
        .text_color(t.text_3)
        .font_weight(FontWeight::MEDIUM)
        .child(div().w(px(20.)).flex_none())
        .child(fr(1.).child("NAME"))
        .child(fr(2.).child("DESCRIPTION"))
        .child(div().w(px(80.)).flex_none().child("SOURCES"))
        .child(div().w(px(44.)).flex_none())
}

fn category_row(
    c: &Category,
    used_by: usize,
    last: bool,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> impl IntoElement {
    let id = c.id.clone();
    let key = c.id.clone();
    let (hover_bg, hover_text) = (t.surface_2, t.text);
    let description = if c.description.is_empty() {
        "— no description".to_string()
    } else {
        c.description.clone()
    };
    let desc_el = div()
        .id(SharedString::from(format!("category-{}-description", c.id)))
        .test_support()
        .aria_label(description.clone())
        .truncate()
        .text_size(px(12.5))
        .text_color(if c.description.is_empty() {
            t.text_3
        } else {
            t.text_2
        })
        .when(!c.description.is_empty(), |el| {
            let tip = SharedString::from(c.description.clone());
            el.aria_description(tip.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .child(description);
    let edit = div()
        .id(SharedString::from(format!("category-{}-edit", c.id)))
        .test_support()
        .aria_label("Edit")
        .aria_description("Edit")
        .tooltip(|window, cx| Tooltip::new("Edit").build(window, cx))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(t.radius)
        .text_color(t.text_2)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg).text_color(hover_text))
        .child(Icon::new(IconName::DotV).size(px(14.)))
        .on_click(cx.listener(move |ws, _, window, cx| {
            ws.categories.open(Target::Category(id.clone()), window, cx);
        }));
    let row = grid_row()
        .id(SharedString::from(format!("category-row-{key}")))
        .py(px(12.))
        .text_size(px(13.5));
    let row = if last {
        row
    } else {
        row.border_b_1().border_color(t.border)
    };
    row.child(
        div().flex_none().w(px(20.)).child(
            div()
                .size(px(8.))
                .rounded_full()
                .bg(t.dot_for(&c.accent).unwrap_or_default()),
        ),
    )
    .child(
        fr(1.).child(
            div()
                .id(SharedString::from(format!("category-{key}-label")))
                .test_support()
                .aria_label(c.label.clone())
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
                .child(c.label.clone()),
        ),
    )
    .child(fr(2.).child(desc_el))
    .child(
        div()
            .id(SharedString::from(format!("category-{key}-count")))
            .test_support()
            .aria_label(used_by.to_string())
            .w(px(80.))
            .flex_none()
            .font_family(theme::FONT_MONO)
            .text_size(px(12.))
            .text_color(t.text_3)
            .child(used_by.to_string()),
    )
    .child(
        div()
            .w(px(44.))
            .flex_none()
            .flex()
            .justify_end()
            .child(edit),
    )
}

/// `Swatches`: 24 px circles; the selected one gets a 2 px surface gap and
/// a 3 px text ring, the others a 1 px border ring.
fn swatches(selected: &str, t: &Tokens, cx: &Context<Workspace>) -> impl IntoElement {
    div().flex().gap(px(6.)).children(ACCENTS.map(|a| {
        let is_sel = a == selected;
        let shadow = if is_sel {
            vec![
                theme::ring(3., t.text, false),
                theme::ring(2., t.surface, false),
            ]
        } else {
            vec![theme::ring(1., t.border, false)]
        };
        div()
            .id(SharedString::from(format!("category-swatch-{a}")))
            .test_support()
            .aria_label(a)
            .aria_selected(is_sel)
            .tooltip(move |window, cx| Tooltip::new(a).build(window, cx))
            .size(px(24.))
            .flex_none()
            .rounded_full()
            .bg(t.dot_for(a).unwrap_or_default())
            .shadow(shadow)
            .cursor_pointer()
            .on_click(cx.listener(move |ws, _, _, cx| {
                if let Some(ed) = ws.categories.editing.as_mut() {
                    ed.accent = a.to_string();
                    cx.notify();
                }
            }))
    }))
}

fn edit_row(
    ed: &Editing,
    used_by: usize,
    last: bool,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> impl IntoElement {
    let is_new = ed.target == Target::New;
    let save = Pill::new("category-edit-save", if is_new { "Add" } else { "Save" })
        .on_click(cx.listener(|ws, _, _, cx| ws.categories.save(cx)))
        .render(t, []);
    let cancel = Pill::new("category-edit-cancel", "Cancel")
        .on_click(cx.listener(|ws, _, _, cx| {
            ws.categories.editing = None;
            cx.notify();
        }))
        .render(t, []);
    let blocked = used_by > 0;
    let delete = (!is_new).then(|| {
        Pill::new("category-edit-delete", "Delete")
            .style(if blocked {
                PillStyle::Plain
            } else {
                PillStyle::Danger
            })
            .title(blocked.then(|| format!("{used_by} source(s) use this category")))
            .on_click(cx.listener(|ws, _, _, cx| ws.categories.delete(cx)))
            .enabled(!blocked)
            .render(t, [])
    });
    edit_frame(t, last)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(fr(1.).child(text_input(
                    "category-edit-label",
                    "Category name",
                    &ed.label,
                    false,
                    t,
                )))
                .child(fr(2.).child(text_input(
                    "category-edit-description",
                    "Description",
                    &ed.description,
                    false,
                    t,
                )))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(6.))
                        .child(save)
                        .child(cancel)
                        .children(delete),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .mt(px(10.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text_3)
                        .child("ACCENT"),
                )
                .child(swatches(&ed.accent, t, cx)),
        )
        .children(
            ed.error
                .clone()
                .map(|e| error_line("category-edit-error", e, t)),
        )
}

pub fn render(view: &CategoriesView, app: &AppModel, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let categories = &app.model.categories;
    let n = categories.len();
    let subtitle = view_subtitle(
        "categories-subtitle",
        if n == 0 {
            "No categories yet. Add at least one before you can create sources or run a Claude-enhanced briefing.".to_string()
        } else {
            format!(
                "{n} categor{}. Descriptions flow into the Claude prompt so the model knows how to bucket articles.",
                if n == 1 { "y" } else { "ies" }
            )
        },
        &t,
    );
    let adding = view
        .editing
        .as_ref()
        .is_some_and(|e| e.target == Target::New);
    let add = Pill::new("categories-add", "Add category")
        .style(PillStyle::Primary)
        .icon(IconName::Plus)
        .on_click(cx.listener(|ws, _, window, cx| ws.categories.open(Target::New, window, cx)))
        .enabled(!adding)
        .render(&t, []);
    let bar = div().flex().mb(px(16.)).child(div().ml_auto().child(add));

    let mut table = block(&t).child(head_row(&t));
    if let Some(ed) = view.editing.as_ref().filter(|e| e.target == Target::New) {
        table = table.child(edit_row(ed, 0, false, &t, cx));
    }
    for (i, c) in categories.iter().enumerate() {
        let last = i + 1 == n && !adding;
        let used_by = source_count(app, &c.id);
        match view
            .editing
            .as_ref()
            .filter(|e| e.target == Target::Category(c.id.clone()))
        {
            Some(ed) => table = table.child(edit_row(ed, used_by, last, &t, cx)),
            None => table = table.child(category_row(c, used_by, last, &t, cx)),
        }
    }
    page_frame(
        "Categories",
        [
            subtitle.into_any_element(),
            bar.into_any_element(),
            table.into_any_element(),
        ],
    )
}
