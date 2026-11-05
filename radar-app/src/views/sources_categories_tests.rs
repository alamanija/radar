//! Headless UI tests for the Sources and Categories views. Run with
//! `cargo test -p radar-app sources_categories`.

use crate::theme;
use crate::views::sources::health_color;
use crate::workspace::shell_tests::{boot, category, Shell};
use crate::workspace::{SourcesIntent, Workspace};
use chrono::Utc;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Hsla, TestAppContext};
use radar_core::types::{Accent, Density, Source, SourceHealth, Theme, View};

const URL_ERROR: &str = "Feed URL must start with http:// or https://";

fn source(id: u32, name: &str, category: &str, is_default: bool) -> Source {
    Source {
        id,
        name: name.into(),
        category: category.into(),
        feed_url: Some(format!("https://{name}.test/feed")),
        enabled: true,
        is_default,
        last_fetch_at: None,
        health: SourceHealth::Ok,
    }
}

fn show(s: &Shell, cx: &mut TestAppContext, view: View) {
    s.edit(cx, |m| m.prefs.view = view);
}

/// Replaces an input's text (no `Change` event, as `set_value` documents).
fn set_text(
    s: &Shell,
    cx: &mut TestAppContext,
    pick: impl FnOnce(&Workspace) -> Entity<InputState>,
    text: &str,
) {
    let input = s.ws.read_with(cx, |w, _| pick(w));
    cx.update_window(s.window, |_, window, cx| {
        input.update(cx, |i, cx| i.set_value(text, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn src_name(w: &Workspace) -> Entity<InputState> {
    w.sources.editing.as_ref().unwrap().name.clone()
}

fn src_url(w: &Workspace) -> Entity<InputState> {
    w.sources.editing.as_ref().unwrap().url.clone()
}

fn cat_label(w: &Workspace) -> Entity<InputState> {
    w.categories.editing.as_ref().unwrap().label.clone()
}

fn cat_desc(w: &Workspace) -> Entity<InputState> {
    w.categories.editing.as_ref().unwrap().description.clone()
}

fn description(s: &Shell, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).description().map(str::to_owned)
    })
    .unwrap()
}

fn selected(s: &Shell, cx: &mut TestAppContext, id: &'static str) -> Option<bool> {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).selected()
    })
    .unwrap()
}

fn select_value(s: &Shell, cx: &mut TestAppContext) -> Option<String> {
    s.ws.read_with(cx, |w, cx| {
        w.sources
            .editing
            .as_ref()
            .unwrap()
            .category
            .read(cx)
            .selected_value()
            .cloned()
    })
}

fn pick_category(s: &Shell, cx: &mut TestAppContext, id: &str) {
    let select = s.ws.read_with(cx, |w, _| {
        w.sources.editing.as_ref().unwrap().category.clone()
    });
    cx.update_window(s.window, |_, window, cx| {
        select.update(cx, |st, cx| {
            st.set_selected_value(&id.to_string(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn sources_view(s: &Shell, cx: &mut TestAppContext) {
    s.edit(cx, |m| {
        m.categories = vec![category("tools"), category("design")];
        m.sources = vec![
            source(1, "alpha", "tools", true),
            source(2, "beta", "tools", false),
        ];
    });
    show(s, cx, View::Sources);
}

#[gpui_kit::test]
fn add_source_row_validates_url(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.click(cx, "sources-add");
    assert!(s.exists(cx, "source-edit-name"));
    assert_eq!(
        select_value(&s, cx).as_deref(),
        Some("tools"),
        "first category"
    );

    // Real keystrokes into the name input.
    s.click(cx, "source-edit-name");
    cx.update_window(s.window, |_, window, cx| window.input("Feed", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        s.ws.read_with(cx, |w, cx| src_name(w).read(cx).value()),
        "Feed"
    );

    for bad in ["ftp://x", "nope"] {
        set_text(&s, cx, src_url, bad);
        s.click(cx, "source-edit-save");
        assert_eq!(
            s.label(cx, "source-edit-error").as_deref(),
            Some(URL_ERROR),
            "{bad}"
        );
        assert_eq!(s.read(cx, |m| m.model.sources.len()), 2, "{bad} not saved");
    }

    set_text(&s, cx, src_url, "https://a.b/feed");
    s.click(cx, "source-edit-save");
    assert!(!s.exists(cx, "source-edit-name"), "row closes");
    assert!(!s.exists(cx, "source-edit-error"));
    let added = s.read(cx, |m| m.model.sources.last().cloned().unwrap());
    assert_eq!(added.name, "Feed");
    assert_eq!(added.feed_url.as_deref(), Some("https://a.b/feed"));
    assert_eq!(added.category, "tools");
    assert!(added.enabled && !added.is_default);
}

#[gpui_kit::test]
fn add_source_requires_name_and_allows_empty_url(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.click(cx, "sources-add");
    s.click(cx, "source-edit-save");
    assert_eq!(
        s.label(cx, "source-edit-error").as_deref(),
        Some("Name is required")
    );
    set_text(&s, cx, src_name, "  Spaced  ");
    s.click(cx, "source-edit-save");
    let added = s.read(cx, |m| m.model.sources.last().cloned().unwrap());
    assert_eq!(added.name, "Spaced", "trimmed");
    assert_eq!(added.feed_url, None, "empty URL means none yet");
}

#[gpui_kit::test]
fn add_disabled_without_categories(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories.clear());
    show(&s, cx, View::Sources);
    assert_eq!(
        description(&s, cx, "sources-add").as_deref(),
        Some("Add a category first")
    );
    s.click(cx, "sources-add");
    assert!(
        !s.exists(cx, "source-edit-name"),
        "disabled button does nothing"
    );

    s.edit(cx, |m| m.categories = vec![category("tools")]);
    assert_eq!(description(&s, cx, "sources-add"), None);
    s.click(cx, "sources-add");
    assert!(s.exists(cx, "source-edit-name"));
}

#[gpui_kit::test]
fn edit_row_saves_patch(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.edit(cx, |m| {
        m.sources[1].last_fetch_at = Some(42);
        m.sources[1].enabled = false;
    });
    s.click(cx, "source-2-edit");
    assert!(
        !s.exists(cx, "source-2-toggle"),
        "row swapped for the edit row"
    );
    assert_eq!(
        s.ws.read_with(cx, |w, cx| src_name(w).read(cx).value()),
        "beta"
    );
    assert_eq!(
        s.ws.read_with(cx, |w, cx| src_url(w).read(cx).value()),
        "https://beta.test/feed"
    );
    assert_eq!(select_value(&s, cx).as_deref(), Some("tools"));

    set_text(&s, cx, src_name, " Gamma ");
    set_text(&s, cx, src_url, "");
    pick_category(&s, cx, "design");
    s.click(cx, "source-edit-save");

    let got = s.read(cx, |m| m.model.sources[1].clone());
    assert_eq!(got.name, "Gamma");
    assert_eq!(got.feed_url, None);
    assert_eq!(got.category, "design");
    assert!(!got.enabled, "untouched fields stay");
    assert_eq!(got.last_fetch_at, Some(42));
    assert!(s.exists(cx, "source-2-toggle"), "back to the plain row");
    assert_eq!(
        s.label(cx, "source-2-feed").as_deref(),
        Some("— no feed URL set")
    );
}

#[gpui_kit::test]
fn edit_row_cancel_and_delete(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.click(cx, "source-2-edit");
    set_text(&s, cx, src_name, "Changed");
    s.click(cx, "source-edit-cancel");
    assert_eq!(s.read(cx, |m| m.model.sources[1].name.clone()), "beta");
    assert!(s.exists(cx, "source-2-toggle"));

    s.click(cx, "source-2-edit");
    s.click(cx, "source-edit-delete");
    assert_eq!(s.read(cx, |m| m.model.sources.len()), 1);
    assert!(!s.exists(cx, "source-2-edit"));
    assert!(!s.exists(cx, "source-edit-name"));
}

#[gpui_kit::test]
fn default_source_has_no_delete_button(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.click(cx, "source-1-edit");
    assert!(s.exists(cx, "source-edit-save"));
    assert!(!s.exists(cx, "source-edit-delete"));
    s.click(cx, "source-edit-cancel");
    s.click(cx, "source-2-edit");
    assert!(s.exists(cx, "source-edit-delete"));
}

#[gpui_kit::test]
fn last_fetch_shows_relative_or_never(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    let five_min_ago = Utc::now().timestamp_millis() - 5 * 60_000;
    s.edit(cx, |m| {
        m.sources[0].last_fetch_at = Some(five_min_ago);
        m.sources[1].health = SourceHealth::Warn;
        m.sources.push(source(3, "gamma", "gone", false));
        m.sources[2].health = SourceHealth::Stale;
        m.sources[2].feed_url = None;
    });
    let l = |s: &Shell, cx: &mut TestAppContext, id: &'static str| s.label(cx, id);
    assert_eq!(
        l(&s, cx, "source-1-health").as_deref(),
        Some("● Live · 5m ago")
    );
    assert_eq!(
        l(&s, cx, "source-2-health").as_deref(),
        Some("● Delayed · never")
    );
    assert_eq!(
        l(&s, cx, "source-3-health").as_deref(),
        Some("● Stale · never")
    );
    assert_eq!(l(&s, cx, "source-1-category").as_deref(), Some("TOOLS"));
    assert_eq!(
        l(&s, cx, "source-3-category").as_deref(),
        Some("— uncategorized")
    );
    assert_eq!(
        l(&s, cx, "source-3-feed").as_deref(),
        Some("— no feed URL set")
    );
    assert_eq!(
        l(&s, cx, "source-1-feed").as_deref(),
        Some("https://alpha.test/feed")
    );
}

#[gpui_kit::test]
fn sources_subtitle_filters_and_toggle(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.edit(cx, |m| m.sources[0].enabled = false);
    assert_eq!(
        s.label(cx, "sources-subtitle").as_deref(),
        Some("1 of 2 enabled. Add any RSS/Atom feed — Radar will fetch and summarize it.")
    );
    assert_eq!(s.label(cx, "sources-filter-all").as_deref(), Some("All 2"));
    assert_eq!(
        s.label(cx, "sources-filter-on").as_deref(),
        Some("Enabled 1")
    );
    assert_eq!(
        s.label(cx, "sources-filter-custom").as_deref(),
        Some("Custom 1")
    );

    s.click(cx, "sources-filter-on");
    assert!(!s.exists(cx, "source-1-toggle") && s.exists(cx, "source-2-toggle"));
    s.click(cx, "sources-filter-custom");
    assert!(!s.exists(cx, "source-1-toggle") && s.exists(cx, "source-2-toggle"));
    s.click(cx, "sources-filter-all");
    assert!(s.exists(cx, "source-1-toggle") && s.exists(cx, "source-2-toggle"));

    // The custom Toggle (not the kit Switch) flips `enabled`.
    s.click(cx, "source-1-toggle");
    assert!(s.read(cx, |m| m.model.sources[0].enabled));
}

#[gpui_kit::test]
fn health_dot_colors(cx: &mut TestAppContext) {
    let _ = cx;
    let rgb = |hex: u32| Hsla::from(gpui_kit::rgb(hex));
    for theme_mode in [Theme::Light, Theme::Dark] {
        let t = theme::tokens(theme_mode, Accent::Camel, Density::Cozy);
        assert_eq!(health_color(SourceHealth::Ok, &t), rgb(0x6B8E4E));
        assert_eq!(health_color(SourceHealth::Warn, &t), rgb(0xB07E3C));
        assert_eq!(health_color(SourceHealth::Stale, &t), t.text_3);
    }
}

#[gpui_kit::test]
fn category_add_slugifies_and_appears_in_source_select(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.categories = vec![category("tools")];
        m.sources.clear();
    });
    show(&s, cx, View::Categories);
    assert_eq!(
        s.label(cx, "categories-subtitle").as_deref(),
        Some("1 category. Descriptions flow into the Claude prompt so the model knows how to bucket articles.")
    );
    s.click(cx, "categories-add");
    s.click(cx, "category-edit-save");
    assert_eq!(
        s.label(cx, "category-edit-error").as_deref(),
        Some("Label is required")
    );
    set_text(&s, cx, cat_label, "  Brand Voice! ");
    set_text(&s, cx, cat_desc, " tone and copy ");
    s.click(cx, "category-edit-save");

    let cats = s.read(cx, |m| m.model.categories.clone());
    assert_eq!(cats.len(), 2);
    assert_eq!(cats[1].id, "brand-voice");
    assert_eq!(cats[1].label, "Brand Voice!");
    assert_eq!(cats[1].description, "tone and copy");
    assert_eq!(cats[1].accent, "olive", "default accent");
    assert_eq!(
        s.label(cx, "category-brand-voice-count").as_deref(),
        Some("0")
    );
    assert_eq!(
        s.label(cx, "categories-subtitle").as_deref(),
        Some("2 categories. Descriptions flow into the Claude prompt so the model knows how to bucket articles.")
    );

    // Same label again dedupes the slug.
    s.click(cx, "categories-add");
    set_text(&s, cx, cat_label, "Brand Voice!");
    s.click(cx, "category-edit-save");
    assert_eq!(
        s.read(cx, |m| m.model.categories[2].id.clone()),
        "brand-voice-2"
    );

    show(&s, cx, View::Sources);
    s.click(cx, "sources-add");
    pick_category(&s, cx, "brand-voice");
    assert_eq!(select_value(&s, cx).as_deref(), Some("brand-voice"));
}

#[gpui_kit::test]
fn categories_empty_subtitle(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories.clear());
    show(&s, cx, View::Categories);
    assert_eq!(
        s.label(cx, "categories-subtitle").as_deref(),
        Some("No categories yet. Add at least one before you can create sources or run a Claude-enhanced briefing.")
    );
}

#[gpui_kit::test]
fn category_edit_patches_and_shows_counts(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.edit(cx, |m| m.categories[0].description = "desc".into());
    show(&s, cx, View::Categories);
    assert_eq!(s.label(cx, "category-tools-count").as_deref(), Some("2"));
    assert_eq!(s.label(cx, "category-design-count").as_deref(), Some("0"));
    assert_eq!(
        s.label(cx, "category-tools-description").as_deref(),
        Some("desc")
    );
    assert_eq!(
        s.label(cx, "category-design-description").as_deref(),
        Some("— no description")
    );

    s.click(cx, "category-tools-edit");
    assert_eq!(
        s.ws.read_with(cx, |w, cx| cat_label(w).read(cx).value()),
        "TOOLS"
    );
    set_text(&s, cx, cat_label, "Gadgets");
    s.click(cx, "category-edit-save");
    let c = s.read(cx, |m| m.model.categories[0].clone());
    assert_eq!((c.id.as_str(), c.label.as_str()), ("tools", "Gadgets"));
    assert_eq!(c.description, "desc");
}

#[gpui_kit::test]
fn category_delete_blocked_shows_count_tooltip(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    show(&s, cx, View::Categories);

    s.click(cx, "category-tools-edit");
    assert_eq!(
        description(&s, cx, "category-edit-delete").as_deref(),
        Some("2 source(s) use this category")
    );
    s.click(cx, "category-edit-delete");
    assert_eq!(s.read(cx, |m| m.model.categories.len()), 2, "blocked");
    assert!(s.exists(cx, "category-edit-save"), "row stays open");

    s.click(cx, "category-edit-cancel");
    s.click(cx, "category-design-edit");
    assert_eq!(description(&s, cx, "category-edit-delete"), None);
    s.click(cx, "category-edit-delete");
    let ids = s.read(cx, |m| {
        m.model
            .categories
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(ids, ["tools"]);
    assert!(!s.exists(cx, "category-edit-save"));
}

#[gpui_kit::test]
fn new_category_row_has_no_delete(cx: &mut TestAppContext) {
    let s = boot(cx);
    show(&s, cx, View::Categories);
    s.click(cx, "categories-add");
    assert!(s.exists(cx, "category-edit-save"));
    assert!(!s.exists(cx, "category-edit-delete"));
}

#[gpui_kit::test]
fn category_accent_swatches(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories = vec![category("tools")]);
    show(&s, cx, View::Categories);
    s.click(cx, "categories-add");
    let accents = ["olive", "rust", "slate", "plum", "sand", "ink"];
    for a in accents {
        let id: &'static str = Box::leak(format!("category-swatch-{a}").into_boxed_str());
        assert_eq!(s.label(cx, id).as_deref(), Some(a), "swatch title");
        assert_eq!(selected(&s, cx, id), Some(a == "olive"), "{a}");
    }
    s.click(cx, "category-swatch-rust");
    assert_eq!(selected(&s, cx, "category-swatch-rust"), Some(true));
    assert_eq!(selected(&s, cx, "category-swatch-olive"), Some(false));

    set_text(&s, cx, cat_label, "Warm");
    s.click(cx, "category-edit-save");
    assert_eq!(s.read(cx, |m| m.model.categories[1].accent.clone()), "rust");

    // Editing starts from the category's own accent.
    s.click(cx, "category-warm-edit");
    assert_eq!(selected(&s, cx, "category-swatch-rust"), Some(true));
}

#[gpui_kit::test]
fn sources_intent_add_opens_row_once(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories = vec![category("tools")]);
    s.ws.update(cx, |w, _| w.sources_intent = Some(SourcesIntent::Add));
    show(&s, cx, View::Sources);
    assert!(s.exists(cx, "source-edit-name"), "intent opens the add row");
    assert_eq!(s.ws.read_with(cx, |w, _| w.sources_intent), None, "cleared");

    s.click(cx, "source-edit-cancel");
    assert!(
        !s.exists(cx, "source-edit-name"),
        "not reopened by later renders"
    );

    // Leaving and returning also keeps it closed.
    show(&s, cx, View::Briefing);
    show(&s, cx, View::Sources);
    assert!(!s.exists(cx, "source-edit-name"));
}

#[gpui_kit::test]
fn sources_intent_without_categories_is_cleared(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories.clear());
    s.ws.update(cx, |w, _| w.sources_intent = Some(SourcesIntent::Add));
    show(&s, cx, View::Sources);
    assert!(!s.exists(cx, "source-edit-name"));
    assert_eq!(s.ws.read_with(cx, |w, _| w.sources_intent), None);
}

#[gpui_kit::test]
fn sidebar_add_button_opens_the_add_row(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories = vec![category("tools")]);
    s.click(cx, "sidebar-add-source");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Sources);
    assert!(s.exists(cx, "source-edit-name"));
}

#[gpui_kit::test]
fn leaving_the_view_discards_the_open_row(cx: &mut TestAppContext) {
    let s = boot(cx);
    sources_view(&s, cx);
    s.click(cx, "source-2-edit");
    assert!(s.exists(cx, "source-edit-name"));
    show(&s, cx, View::Briefing);
    assert!(!s.exists(cx, "source-edit-name"));
    show(&s, cx, View::Sources);
    assert!(
        !s.exists(cx, "source-edit-name"),
        "JSX state dies on unmount"
    );
}
