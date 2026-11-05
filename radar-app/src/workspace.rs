//! RED stub.
use crate::app_model::AppModel;
use crate::theme;
use gpui_kit::{div, App, Context, IntoElement, Render, Styled, Window};

gpui_kit::actions!(radar, [OpenSearch, RunBriefing, ToggleSidebar, ToggleTheme]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcesIntent {
    Add,
}

pub struct Workspace {
    pub search_open: bool,
    pub tweaks_open: bool,
    pub sources_intent: Option<SourcesIntent>,
}

pub fn init(_cx: &mut App) {}

impl Workspace {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&AppModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self { search_open: false, tweaks_open: false, sources_intent: None }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *theme::t(cx);
        div().size_full().bg(t.bg).text_color(t.text).font_family(theme::FONT_SANS)
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod shell_tests;
