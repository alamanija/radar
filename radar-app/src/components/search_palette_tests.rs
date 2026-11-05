//! Headless tests of the search palette. Run with
//! `cargo test -p radar-app search_palette`.

use crate::workspace::shell_tests::{article, boot, Shell};
use crate::workspace::OpenSearch;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::{IndexPath, WindowExt as _};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{point, px, AppContext as _, MouseButton, Role, TestAppContext};
use radar_core::types::{Archive, Source, SourceHealth, View};

fn fixture(s: &Shell, cx: &mut TestAppContext) {
    s.edit(cx, |m| {
        let mut a1 = article(1, "tools", false);
        a1.title = "Bauhaus Rebrand".into();
        a1.source = "Dezeen".into();
        a1.summary = "Geometric forms".into();
        let mut a2 = article(2, "tools", false);
        a2.title = "Packaging trends".into();
        a2.source = "It's Nice That".into();
        a2.summary = "Kraft BOARD revival".into();
        // Archive-only article, plus an older copy of article 1 that the
        // current one must win over.
        let mut a3 = article(3, "tools", false);
        a3.title = "Archived Poster".into();
        let mut old1 = article(1, "tools", false);
        old1.title = "Old copy of bauhaus".into();
        m.articles = vec![a1, a2];
        m.archives = vec![Archive {
            id: 1,
            run_at: "2026-01-01T00:00:00Z".into(),
            articles: vec![a3, old1],
            errors: vec![],
        }];
        m.sources = vec![Source {
            id: 7,
            name: "Brand New".into(),
            category: "tools".into(),
            feed_url: Some("https://FEEDS.example/rss".into()),
            enabled: true,
            is_default: false,
            last_fetch_at: None,
            health: SourceHealth::Ok,
        }];
    });
}

fn open(s: &Shell, cx: &mut TestAppContext) {
    cx.dispatch_action(s.window, OpenSearch);
    cx.run_until_parked();
}

fn type_query(s: &Shell, cx: &mut TestAppContext, text: &str) {
    cx.update_window(s.window, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn press(s: &Shell, cx: &mut TestAppContext, key: &'static str) {
    cx.update_window(s.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Labels of the rendered result rows, in order.
fn rows(s: &Shell, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::ListBoxOption)
            .iter()
            .filter_map(|r| r.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

fn selected_row(s: &Shell, cx: &mut TestAppContext, section: usize, row: usize) -> bool {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        window.find(IndexPath::new(row).section(section)).selected() == Some(true)
    })
    .unwrap()
}

fn dialog_open(s: &Shell, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.window, |_, window, cx| window.has_active_dialog(cx))
        .unwrap()
}

fn search_open(s: &Shell, cx: &mut TestAppContext) -> bool {
    s.ws.read_with(cx, |w, _| w.search_open)
}

#[gpui_kit::test]
fn palette_filters_case_insensitive(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    // (typed query, expected rows): title, source, summary, name, category,
    // feed URL, and an archive-only article.
    let cases: [(&str, &[&str]); 7] = [
        ("BAUHAUS", &["Bauhaus Rebrand"]),
        ("dEzEeN", &["Bauhaus Rebrand"]),
        ("kraft board", &["Packaging trends"]),
        ("brand NEW", &["Brand New"]),
        ("TOOLS", &["Brand New"]),
        ("feeds.EXAMPLE", &["Brand New"]),
        ("poster", &["Archived Poster"]),
    ];
    for (query, want) in cases {
        open(&s, cx);
        type_query(&s, cx, query);
        assert_eq!(rows(&s, cx), want, "query {query:?}");
        // Esc clears the query, then closes.
        press(&s, cx, "escape");
        press(&s, cx, "escape");
        assert!(!search_open(&s, cx));
    }
}

#[gpui_kit::test]
fn palette_empty_query_hint_and_no_matches(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    assert!(rows(&s, cx).is_empty());
    assert_eq!(
        s.label(cx, "palette-empty").as_deref(),
        Some("Type to search articles and sources.")
    );
    type_query(&s, cx, "zzzz");
    assert!(rows(&s, cx).is_empty());
    assert_eq!(s.label(cx, "palette-empty").as_deref(), Some("No matches."));
}

#[gpui_kit::test]
fn palette_enter_opens_article_url(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    type_query(&s, cx, "bauhaus");
    press(&s, cx, "enter");
    assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/1"));
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx));
}

#[gpui_kit::test]
fn palette_source_navigates(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    type_query(&s, cx, "brand new");
    press(&s, cx, "enter");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Sources);
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx));
}

#[gpui_kit::test]
fn palette_click_on_row_chooses_it(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    type_query(&s, cx, "packaging");
    cx.update_window(s.window, |_, window, cx| {
        window.click(IndexPath::new(0).section(0), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/2"));
    assert!(!search_open(&s, cx));
}

#[gpui_kit::test]
fn palette_esc_closes(cx: &mut TestAppContext) {
    let s = boot(cx);
    open(&s, cx);
    assert!(search_open(&s, cx));
    assert!(dialog_open(&s, cx));
    // Keystroke goes to the focused element: the palette's own input.
    press(&s, cx, "escape");
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx));
}

#[gpui_kit::test]
fn palette_closes_on_scrim_click_and_can_reopen(cx: &mut TestAppContext) {
    let s = boot(cx);
    open(&s, cx);
    cx.update_window(s.window, |_, window, cx| {
        window.pointer_down(point(px(4.), px(500.)), MouseButton::Left, cx);
        window.pointer_up(point(px(4.), px(500.)), MouseButton::Left, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx));
    open(&s, cx);
    assert!(search_open(&s, cx));
    assert!(dialog_open(&s, cx));
}

#[gpui_kit::test]
fn palette_open_twice_does_not_stack(cx: &mut TestAppContext) {
    let s = boot(cx);
    open(&s, cx);
    open(&s, cx);
    press(&s, cx, "escape");
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx), "a second dialog was stacked");
}

#[gpui_kit::test]
fn palette_arrow_keys_move_selection(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    // "e" matches both live articles, the archive-only one and the source.
    type_query(&s, cx, "e");
    assert_eq!(
        rows(&s, cx),
        [
            "Bauhaus Rebrand",
            "Packaging trends",
            "Archived Poster",
            "Brand New"
        ]
    );
    assert!(selected_row(&s, cx, 0, 0));
    press(&s, cx, "down");
    assert!(selected_row(&s, cx, 0, 1));
    press(&s, cx, "down");
    press(&s, cx, "down");
    assert!(selected_row(&s, cx, 1, 0), "selection crosses into Sources");
    press(&s, cx, "up");
    assert!(selected_row(&s, cx, 0, 2));
    press(&s, cx, "up");
    // Enter confirms the highlighted row.
    press(&s, cx, "enter");
    assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/2"));
}

#[gpui_kit::test]
fn palette_esc_closes_at_once_with_a_query(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    type_query(&s, cx, "bauhaus");
    press(&s, cx, "escape");
    assert!(!search_open(&s, cx));
    assert!(!dialog_open(&s, cx));
}

#[gpui_kit::test]
fn palette_choose_uses_the_rows_the_user_saw(cx: &mut TestAppContext) {
    let s = boot(cx);
    fixture(&s, cx);
    open(&s, cx);
    type_query(&s, cx, "e");
    assert_eq!(rows(&s, cx)[0], "Bauhaus Rebrand");
    // A briefing lands and Enter follows before the palette redraws: the
    // chosen row is the one the user was shown, not whatever now sits at
    // that index.
    cx.update_window(s.window, |_, window, cx| {
        s.model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                let mut n = article(9, "tools", false);
                n.title = "Fresh arrival".into();
                r.articles.insert(0, n);
            })
        });
        // Dispatch straight away: `press` would draw a frame first.
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/1"));
}
