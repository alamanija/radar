//! Port of `src/views/SavedView.jsx`: the bookmarked articles of the
//! current briefing as a card grid. Cards are live (toggles change the
//! model) and scoped `saved` so their element ids never collide with the
//! briefing's.

use super::briefing::{grid_box, page_frame, view_subtitle};
use super::empty_state::empty_state;
use crate::app_model::AppModel;
use crate::components::card::{Card, CardOpts};
use crate::components::icon::IconName;
use crate::theme;
use crate::workspace::Workspace;
use gpui_kit::{AnyElement, Context, Div, IntoElement as _, Window};

/// Element-id scope of the Saved cards (`saved/card-<id>`).
pub const SCOPE: &str = "saved";

pub fn render(app: &AppModel, window: &Window, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let prefs = &app.model.prefs;
    // `articles.filter(a => a.bookmarked)`: only the bookmark flag counts.
    let saved: Vec<_> = app.model.articles.iter().filter(|a| a.bookmarked).collect();
    let opts = CardOpts {
        show_accent: prefs.show_accent,
        use_serif: prefs.use_serif,
        read_only: false,
    };
    let subtitle = view_subtitle(
        "saved-subtitle",
        format!(
            "{} articles kept for later. Pinned to the top of your briefing on request.",
            saved.len()
        ),
        &t,
    );
    let body: AnyElement = if saved.is_empty() {
        empty_state(
            "saved-empty",
            IconName::Bookmark,
            "Nothing saved yet",
            "Tap the bookmark icon on any card to keep it here.",
            &t,
        )
        .into_any_element()
    } else {
        use gpui_kit::ParentElement as _;
        grid_box("saved-grid", window, prefs.sidebar_collapsed)
            .children(saved.iter().map(|a| Card::new(a, opts).scope(SCOPE)))
            .into_any_element()
    };
    page_frame("Saved", [subtitle.into_any_element(), body])
}
