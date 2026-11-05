//! The ⌘K search palette: a gpui-kit
//! [`Command`] hosted in a dialog, over [`radar_core::search::search`].
//!
//! [`Workspace::search_open`] stays the source of truth for "is it open":
//! [`Workspace::open_search`] sets it, and every way out of the palette
//! (Esc, the scrim, choosing a row) ends in [`Workspace::close_search`].

use super::header::kbd;
use super::icon::{Icon, IconName};
use crate::app_model::AppModel;
use crate::theme;
use crate::workspace::Workspace;
use gpui_kit::base::actions::Cancel;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{IndexPath, WindowExt as _};
use gpui_kit::{
    div, px, AnyElement, App, AppContext as _, Context, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled, WeakEntity, Window,
};
use radar_core::search::{search, Hit};
use radar_core::types::{Article, Source, View};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const EMPTY_QUERY_HINT: &str = "Type to search articles and sources.";
const NO_MATCHES: &str = "No matches.";
/// Panel width.
const WIDTH: f32 = 600.;
/// Kit groups are addressed by position; both are always present so the
/// section of a hit never depends on the other group being empty.
const ARTICLES: usize = 0;
const SOURCES: usize = 1;

/// The dialog content: owns the [`CommandState`] and rebuilds the result
/// groups from the model whenever the query changes.
pub struct SearchPalette {
    state: Entity<CommandState>,
    workspace: WeakEntity<Workspace>,
    /// The hits the last render put on screen; a choice is made from these,
    /// never from a fresh search of a model that may have moved on.
    shown: Rc<RefCell<Shown>>,
}

type Shown = (Vec<Article>, Vec<Source>);

/// Search hits split by kind (articles first, as `search` orders them).
fn split(query: &str, app: &AppModel) -> (Vec<Article>, Vec<Source>) {
    let m = &app.model;
    let mut articles = Vec::new();
    let mut sources = Vec::new();
    for hit in search(query, &m.articles, &m.archives, &m.sources) {
        match hit {
            Hit::Article(a) => articles.push(a),
            Hit::Source(s) => sources.push(s),
        }
    }
    (articles, sources)
}

impl SearchPalette {
    /// Opens the palette as a dialog on `window`. `workspace` is told when
    /// it closes, by whatever route.
    pub fn open(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let palette = cx.new(|_| SearchPalette {
            state: state.clone(),
            workspace: workspace.clone(),
            shown: Rc::default(),
        });
        // The input takes focus once, not on every rebuild of the dialog.
        let focus_input = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_input.replace(false) {
                state.update(cx, |s, cx| s.focus(window, cx));
            }
            let workspace = workspace.clone();
            let palette = palette.clone();
            dialog
                .w(px(WIDTH))
                .p_0()
                .overflow_hidden()
                .close_button(false)
                .on_close(move |_, _, cx| mark_closed(&workspace, cx))
                .content(move |content, _, _| content.child(palette.clone()))
        });
    }
}

fn mark_closed(workspace: &WeakEntity<Workspace>, cx: &mut App) {
    workspace.update(cx, |ws, cx| ws.close_search(cx)).ok();
}

impl Render for SearchPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme::t(cx);
        let query = self.state.read(cx).query(cx);
        let (articles, sources) = split(&query, AppModel::global(cx).read(cx));
        *self.shown.borrow_mut() = (articles.clone(), sources.clone());

        let article_group = CommandGroup::new()
            .label("ARTICLES")
            .items(articles.iter().map(|a| {
                let dot = t.dot_for(&a.accent);
                let subtitle = format!("{} · {} · {}", a.source, a.category_label, a.published);
                result_row(&a.title, subtitle, move |_| match dot {
                    Some(color) => div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(color)
                        .into_any_element(),
                    None => div().into_any_element(),
                })
            }));
        let source_group = CommandGroup::new()
            .label("SOURCES")
            .items(sources.iter().map(|s| {
                let subtitle = format!(
                    "{} · {}",
                    s.category,
                    s.feed_url.as_deref().unwrap_or("no feed URL")
                );
                result_row(&s.name, subtitle, |text_3| {
                    div()
                        .text_color(text_3)
                        .child(Icon::new(IconName::Feed).size(px(14.)))
                        .into_any_element()
                })
            }));

        let me = cx.entity();
        let shown = self.shown.clone();
        let workspace = self.workspace.clone();
        let esc_workspace = self.workspace.clone();
        let command = Command::new(&self.state)
            .group(article_group)
            .group(source_group)
            .filterable(false)
            .bordered(false)
            .placeholder("Search articles, sources…")
            .max_h(px(420.))
            .on_query(move |_, _, cx| me.update(cx, |_, cx| cx.notify()))
            .empty(|state, _, cx| {
                let text = if state.query(cx).trim().is_empty() {
                    EMPTY_QUERY_HINT
                } else {
                    NO_MATCHES
                };
                let t = *theme::t(cx);
                div()
                    .id("palette-empty")
                    .test_support()
                    .aria_label(text)
                    .py(px(40.))
                    .w_full()
                    .text_center()
                    .text_size(px(13.))
                    .text_color(t.text_3)
                    .child(text)
            })
            .footer(|_, _, cx| {
                let t = *theme::t(cx);
                div()
                    .flex()
                    .px(px(16.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(kbd("esc", &t))
            })
            .on_confirm(move |ix, window, cx| {
                choose(&shown.borrow(), ix, &workspace, window, cx);
            });
        // Esc closes at once: take it before the kit's
        // clear-the-query-first handler sees it.
        div()
            .child(command)
            .capture_action(move |_: &Cancel, window, cx| {
                window.close_dialog(cx);
                mark_closed(&esc_workspace, cx);
                cx.stop_propagation();
            })
    }
}

/// One result row: leading marker, title over a mono subtitle.
fn result_row(
    title: &str,
    subtitle: String,
    marker: impl Fn(Hsla) -> AnyElement + 'static,
) -> CommandItem {
    let label = title.to_owned();
    CommandItem::new()
        .label(title.to_owned())
        .child(move |_, cx| {
            let t = *theme::t(cx);
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .w(px(16.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(marker(t.text_3)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(theme::FONT_MONO)
                                .text_size(px(11.5))
                                .text_color(t.text_3)
                                .child(subtitle.clone()),
                        ),
                )
        })
}

/// Enter / click on a row: open the article's URL, or go to Sources; then
/// close either way.
fn choose(
    shown: &Shown,
    ix: IndexPath,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let (articles, sources) = shown;
    match ix.section {
        ARTICLES => {
            if let Some(a) = articles.get(ix.row).filter(|a| !a.url.is_empty()) {
                cx.open_url(&a.url);
            }
        }
        SOURCES if ix.row < sources.len() => {
            AppModel::mutate(cx, |m| m.prefs.view = View::Sources);
        }
        _ => {}
    }
    window.close_dialog(cx);
    mark_closed(workspace, cx);
}

#[cfg(test)]
#[path = "search_palette_tests.rs"]
mod tests;
