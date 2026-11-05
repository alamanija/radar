//! Port of `src/views/SourcesView.jsx`: the filterable source list with
//! inline add and edit rows.
//!
//! # State
//! [`SourcesView`] lives on the `Workspace` and is dropped whenever another
//! view is shown (the JSX state dies with the unmounted component). The
//! edit row's draft is held by its widgets: two gpui-kit `Input`s and a
//! `Select` ([`Editing`]), read when Save is pressed.
//!
//! # Differences from the JSX
//! - `<input>`/`<select>` are gpui-kit `Input`/`Select`; they keep the
//!   kit's focus ring and menu chrome (see the task report).
//! - Hover titles use GPUI tooltips and also set the accessible
//!   description, which is what tests read.

use super::briefing::{page_frame, view_subtitle};
use super::form::{
    block, edit_frame, error_line, new_input, text_input, Pill, PillStyle, INPUT_TEXT,
};
use crate::app_model::AppModel;
use crate::components::icon::{Icon, IconName};
use crate::components::toggle::Toggle;
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use chrono::Utc;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IndexPath, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, hsla, px, AnyElement, AppContext as _, Context, Div, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled,
};
use radar_core::model::SourcePatch;
use radar_core::text::relative_time_ms;
use radar_core::types::{Category, Source, SourceHealth};

/// Message for an unusable feed URL (`attemptSave` in the JSX).
pub const URL_ERROR: &str = "Feed URL must start with http:// or https://";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    On,
    Custom,
}

/// What the open edit row is for (`editing.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    New,
    Source(u32),
}

/// A category as a `Select` option: the id is the value, the label is shown.
#[derive(Clone)]
pub struct CategoryOption {
    pub id: String,
    pub label: String,
}

impl SearchableListItem for CategoryOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

/// The open edit row and its draft.
pub struct Editing {
    pub target: Target,
    pub name: Entity<InputState>,
    pub url: Entity<InputState>,
    pub category: Entity<SelectState<Vec<CategoryOption>>>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct SourcesView {
    pub filter: Filter,
    pub editing: Option<Editing>,
}

/// `isValidFeedUrl`: empty is fine ("no URL yet"); otherwise the text must
/// parse as an absolute URL with an `http:` or `https:` scheme.
pub fn is_valid_feed_url(u: &str) -> bool {
    if u.is_empty() {
        return true;
    }
    url::Url::parse(u).is_ok_and(|p| matches!(p.scheme(), "http" | "https"))
}

/// `healthStyle`: Live green, Delayed amber (both fixed hex in the JSX),
/// Stale muted.
pub fn health_color(h: SourceHealth, t: &Tokens) -> Hsla {
    match h {
        SourceHealth::Ok => gpui_kit::rgb(0x6B8E4E).into(),
        SourceHealth::Warn => gpui_kit::rgb(0xB07E3C).into(),
        SourceHealth::Stale => t.text_3,
    }
}

fn health_label(h: SourceHealth) -> &'static str {
    match h {
        SourceHealth::Ok => "Live",
        SourceHealth::Warn => "Delayed",
        SourceHealth::Stale => "Stale",
    }
}

fn options(categories: &[Category]) -> Vec<CategoryOption> {
    categories
        .iter()
        .map(|c| CategoryOption {
            id: c.id.clone(),
            label: c.label.clone(),
        })
        .collect()
}

impl SourcesView {
    /// Opens the edit row for `target` (replacing any open one), with the
    /// draft of `emptyDraft()` or `startEdit()`.
    pub fn open(
        &mut self,
        target: Target,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Workspace>,
    ) {
        let app = AppModel::global(cx).read(cx);
        let categories = app.model.categories.clone();
        let (name, url, category) = match target {
            Target::New => (
                String::new(),
                String::new(),
                categories.first().map(|c| c.id.clone()),
            ),
            Target::Source(id) => match app.model.sources.iter().find(|s| s.id == id) {
                Some(s) => (
                    s.name.clone(),
                    s.feed_url.clone().unwrap_or_default(),
                    Some(s.category.clone()),
                ),
                None => return,
            },
        };
        let index = category
            .and_then(|id| categories.iter().position(|c| c.id == id))
            .map(|row| IndexPath::default().row(row));
        let opts = options(&categories);
        self.editing = Some(Editing {
            target,
            name: new_input("Source name (e.g. The Dieline)", &name, window, cx),
            url: new_input("https://example.com/feed/", &url, window, cx),
            category: cx.new(|cx| SelectState::new(opts, index, window, cx)),
            error: None,
        });
        cx.notify();
    }

    pub fn start_add(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Workspace>) {
        self.open(Target::New, window, cx);
    }

    /// `attemptSave`: validate, then add or patch through the model.
    fn save(&mut self, cx: &mut Context<Workspace>) {
        let Some(ed) = self.editing.as_mut() else {
            return;
        };
        let name = ed.name.read(cx).value().trim().to_string();
        let url = ed.url.read(cx).value().trim().to_string();
        let category = ed.category.read(cx).selected_value().cloned();
        let error = if name.is_empty() {
            Some("Name is required")
        } else if category.is_none() && ed.target == Target::New {
            Some("Pick a category")
        } else if !is_valid_feed_url(&url) {
            Some(URL_ERROR)
        } else {
            None
        };
        if let Some(e) = error {
            ed.error = Some(e.to_string());
            cx.notify();
            return;
        }
        let feed_url = (!url.is_empty()).then_some(url);
        let target = ed.target;
        self.editing = None;
        AppModel::mutate(cx, |m| match target {
            Target::New => {
                m.add_source(name, category.unwrap_or_default(), feed_url);
            }
            Target::Source(id) => m.update_source(
                id,
                SourcePatch {
                    name: Some(name),
                    category,
                    feed_url: Some(feed_url),
                    enabled: None,
                },
            ),
        });
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Workspace>) {
        if let Some(Target::Source(id)) = self.editing.as_ref().map(|e| e.target) {
            self.editing = None;
            // Defaults have no Delete button; the model also refuses them.
            let _ = AppModel::mutate(cx, |m| m.delete_source(id));
            cx.notify();
        }
    }
}

fn count_span(n: usize, active: bool, t: &Tokens) -> AnyElement {
    let color = if active {
        let bg = t.bg;
        hsla(bg.h, bg.s, bg.l, bg.a * 0.55)
    } else {
        t.text_3
    };
    div()
        .text_size(px(12.))
        .text_color(color)
        .font_features(theme::font_features(true))
        .child(n.to_string())
        .into_any_element()
}

fn filter_pill(
    id: &'static str,
    label: &'static str,
    n: usize,
    filter: Filter,
    current: Filter,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> impl IntoElement {
    let active = filter == current;
    Pill::new(id, label)
        .aria(format!("{label} {n}"))
        .style(if active {
            PillStyle::Active
        } else {
            PillStyle::Plain
        })
        .on_click(cx.listener(move |ws, _, _, cx| {
            ws.sources.filter = filter;
            cx.notify();
        }))
        .render(t, [count_span(n, active, t)])
}

/// `srcRow` column widths: `20px 1fr 140px 110px 44px`, gap 14.
fn grid_row() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(18.))
        .py(px(12.))
        .text_size(px(13.5))
}

fn head_row(t: &Tokens) -> impl IntoElement {
    let cell = |w: Option<f32>, text: &'static str| {
        let d = div().child(text);
        match w {
            Some(w) => d.w(px(w)).flex_none(),
            None => d.flex_1().min_w_0(),
        }
    };
    grid_row()
        .bg(t.surface_2)
        .border_b_1()
        .border_color(t.border)
        .text_size(px(11.))
        .text_color(t.text_3)
        .font_weight(FontWeight::MEDIUM)
        .child(div().w(px(20.)).flex_none())
        .child(cell(None, "SOURCE"))
        .child(cell(Some(140.), "CATEGORY"))
        .child(cell(Some(110.), "LAST FETCH"))
        .child(div().w(px(44.)).flex_none())
}

fn source_row(
    s: &Source,
    categories: &[Category],
    last: bool,
    now_ms: i64,
    t: &Tokens,
    cx: &Context<Workspace>,
) -> impl IntoElement {
    let id = s.id;
    let category = categories
        .iter()
        .find(|c| c.id == s.category)
        .map_or("— uncategorized", |c| c.label.as_str())
        .to_string();
    let feed = s
        .feed_url
        .clone()
        .unwrap_or_else(|| "— no feed URL set".into());
    let health = format!(
        "● {} · {}",
        health_label(s.health),
        relative_time_ms(s.last_fetch_at, now_ms)
    );
    let (hover_bg, hover_text) = (t.surface_2, t.text);
    let feed_el = div()
        .id(SharedString::from(format!("source-{id}-feed")))
        .test_support()
        .aria_label(feed.clone())
        .truncate()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.5))
        .text_color(t.text_3)
        .when_some(s.feed_url.clone(), |el, url| {
            let tip = SharedString::from(url);
            el.aria_description(tip.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .child(feed);
    let edit = div()
        .id(SharedString::from(format!("source-{id}-edit")))
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
            ws.sources.open(Target::Source(id), window, cx);
        }));
    let row = grid_row().id(SharedString::from(format!("source-row-{id}")));
    let row = if last {
        row
    } else {
        row.border_b_1().border_color(t.border)
    };
    row.child(
        div().w(px(20.)).flex_none().child(
            Toggle::new(SharedString::from(format!("source-{id}-toggle")), s.enabled)
                .on_click(move |_, _, cx| AppModel::mutate(cx, |m| m.toggle_source(id))),
        ),
    )
    .child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id(SharedString::from(format!("source-{id}-name")))
                    .test_support()
                    .aria_label(s.name.clone())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(s.name.clone()),
            )
            .child(feed_el),
    )
    .child(
        div()
            .id(SharedString::from(format!("source-{id}-category")))
            .test_support()
            .aria_label(category.clone())
            .w(px(140.))
            .flex_none()
            .text_size(px(12.5))
            .text_color(t.text_2)
            .child(category),
    )
    .child(
        div()
            .id(SharedString::from(format!("source-{id}-health")))
            .test_support()
            .aria_label(health.clone())
            .w(px(110.))
            .flex_none()
            .font_family(theme::FONT_MONO)
            .text_size(px(11.5))
            .text_color(health_color(s.health, t))
            .child(health),
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

fn edit_row(ed: &Editing, last: bool, t: &Tokens, cx: &Context<Workspace>) -> impl IntoElement {
    let is_new = ed.target == Target::New;
    let is_default = match ed.target {
        Target::Source(id) => AppModel::global(cx)
            .read(cx)
            .model
            .sources
            .iter()
            .any(|s| s.id == id && s.is_default),
        Target::New => false,
    };
    let save = Pill::new("source-edit-save", if is_new { "Add" } else { "Save" })
        .on_click(cx.listener(|ws, _, _, cx| ws.sources.save(cx)))
        .render(t, []);
    let cancel = Pill::new("source-edit-cancel", "Cancel")
        .on_click(cx.listener(|ws, _, _, cx| {
            ws.sources.editing = None;
            cx.notify();
        }))
        .render(t, []);
    let delete = (!is_new && !is_default).then(|| {
        Pill::new("source-edit-delete", "Delete")
            .style(PillStyle::Danger)
            .on_click(cx.listener(|ws, _, _, cx| ws.sources.delete(cx)))
            .render(t, [])
    });
    edit_frame(t, last)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.))
                        .child(text_input(
                            "source-edit-name",
                            "Source name",
                            &ed.name,
                            false,
                            t,
                        ))
                        .child(text_input("source-edit-url", "Feed URL", &ed.url, true, t)),
                )
                .child(
                    div().w(px(180.)).flex_none().child(
                        Select::new(&ed.category)
                            .with_size(Size::Size(px(INPUT_TEXT / 0.875)))
                            .h(px(28.))
                            .id("source-edit-category")
                            .accessibility_label("Category")
                            .placeholder("— no categories —"),
                    ),
                )
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
        .children(
            ed.error
                .clone()
                .map(|e| error_line("source-edit-error", e, t)),
        )
}

pub fn render(view: &SourcesView, app: &AppModel, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let m = &app.model;
    let enabled = m.sources.iter().filter(|s| s.enabled).count();
    let custom = m.sources.iter().filter(|s| !s.is_default).count();
    let adding = view
        .editing
        .as_ref()
        .is_some_and(|e| e.target == Target::New);
    let no_categories = m.categories.is_empty();

    let subtitle = view_subtitle(
        "sources-subtitle",
        format!(
            "{enabled} of {} enabled. Add any RSS/Atom feed — Radar will fetch and summarize it.",
            m.sources.len()
        ),
        &t,
    );

    let add = Pill::new("sources-add", "Add source")
        .style(PillStyle::Primary)
        .icon(IconName::Plus)
        .title(no_categories.then(|| "Add a category first".to_string()))
        .on_click(cx.listener(|ws, _, window, cx| ws.sources.start_add(window, cx)))
        .enabled(!adding && !no_categories)
        .render(&t, []);
    let bar = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .mb(px(16.))
        .child(filter_pill(
            "sources-filter-all",
            "All",
            m.sources.len(),
            Filter::All,
            view.filter,
            &t,
            cx,
        ))
        .child(filter_pill(
            "sources-filter-on",
            "Enabled",
            enabled,
            Filter::On,
            view.filter,
            &t,
            cx,
        ))
        .child(filter_pill(
            "sources-filter-custom",
            "Custom",
            custom,
            Filter::Custom,
            view.filter,
            &t,
            cx,
        ))
        .child(div().ml_auto().child(add));

    let filtered: Vec<&Source> = m
        .sources
        .iter()
        .filter(|s| match view.filter {
            Filter::All => true,
            Filter::On => s.enabled,
            Filter::Custom => !s.is_default,
        })
        .collect();
    let now_ms = Utc::now().timestamp_millis();
    let mut table = block(&t).child(head_row(&t));
    if let Some(ed) = view.editing.as_ref().filter(|e| e.target == Target::New) {
        table = table.child(edit_row(ed, false, &t, cx));
    }
    for (i, s) in filtered.iter().enumerate() {
        let last = i + 1 == filtered.len() && !adding;
        match view
            .editing
            .as_ref()
            .filter(|e| e.target == Target::Source(s.id))
        {
            Some(ed) => table = table.child(edit_row(ed, last, &t, cx)),
            None => table = table.child(source_row(s, &m.categories, last, now_ms, &t, cx)),
        }
    }
    page_frame(
        "Sources",
        [
            subtitle.into_any_element(),
            bar.into_any_element(),
            table.into_any_element(),
        ],
    )
}
