//! Headless UI tests for the Saved and Archive views. Run with
//! `cargo test -p radar-app saved_archive`.

use crate::workspace::shell_tests::{article, boot, category, Shell};
use chrono::{Duration as Dur, Local, Utc};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use radar_core::types::{Archive, SourceError, View};

fn show(s: &Shell, cx: &mut TestAppContext, view: View) {
    s.edit(cx, |m| m.prefs.view = view);
}

fn snapshot(id: i64, hours_ago: i64, articles: Vec<radar_core::types::Article>) -> Archive {
    Archive {
        id,
        run_at: (Utc::now() - Dur::hours(hours_ago)).to_rfc3339(),
        articles,
        errors: vec![],
    }
}

/// Top edge of an element in the current frame.
fn top(s: &Shell, cx: &mut TestAppContext, id: &'static str) -> f32 {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        f32::from(window.find(id).bounds().origin.y)
    })
    .unwrap()
}

fn live_with_one_bookmark(s: &Shell, cx: &mut TestAppContext) {
    s.edit(cx, |m| {
        m.categories = vec![category("tools")];
        m.articles = vec![
            article(1, "tools", false),
            article(2, "tools", true),
            article(3, "tools", false),
        ];
        m.articles[1].bookmarked = true;
    });
}

#[gpui_kit::test]
fn saved_lists_only_bookmarked(cx: &mut TestAppContext) {
    let s = boot(cx);
    live_with_one_bookmark(&s, cx);
    show(&s, cx, View::Saved);
    assert_eq!(s.label(cx, "content-title").as_deref(), Some("Saved"));
    assert!(s.exists(cx, "saved/card-2"));
    assert!(!s.exists(cx, "saved/card-1"));
    assert!(!s.exists(cx, "saved/card-3"));
    assert!(!s.exists(cx, "saved-empty"));
    assert_eq!(
        s.label(cx, "saved-subtitle").as_deref(),
        Some("1 articles kept for later. Pinned to the top of your briefing on request.")
    );
}

#[gpui_kit::test]
fn saved_empty_state_copy_and_toggles_are_live(cx: &mut TestAppContext) {
    let s = boot(cx);
    live_with_one_bookmark(&s, cx);
    s.edit(cx, |m| m.articles[1].bookmarked = false);
    show(&s, cx, View::Saved);
    assert_eq!(
        s.label(cx, "saved-empty").as_deref(),
        Some("Nothing saved yet")
    );

    s.edit(cx, |m| m.articles[1].bookmarked = true);
    // Saved cards act on the live model; ids are scoped, so the briefing's
    // `card-2` is not what is clicked.
    s.click(cx, "saved/card-2-bookmark");
    assert!(!s.read(cx, |m| m.model.articles[1].bookmarked));
    assert!(s.exists(cx, "saved-empty"), "unsaved card leaves the page");
}

#[gpui_kit::test]
fn archive_list_newest_first_with_counts(cx: &mut TestAppContext) {
    let s = boot(cx);
    let mut a = vec![article(1, "tools", false), article(2, "tools", true)];
    a[1].bookmarked = true;
    s.edit(cx, |m| {
        m.archives = vec![
            snapshot(9, 1, a.clone()),
            snapshot(8, 30, vec![article(3, "tools", true)]),
        ];
    });
    show(&s, cx, View::Archive);
    assert_eq!(s.label(cx, "content-title").as_deref(), Some("Archive"));
    assert_eq!(
        s.label(cx, "archive-subtitle").as_deref(),
        Some("2 briefings. Newest first.")
    );
    assert!(top(&s, cx, "archive-row-9") < top(&s, cx, "archive-row-8"));
    let l = |s: &Shell, cx: &mut TestAppContext, id: &'static str| s.label(cx, id);
    assert_eq!(l(&s, cx, "archive-row-9-items").as_deref(), Some("2 items"));
    assert_eq!(
        l(&s, cx, "archive-row-9-unread").as_deref(),
        Some("1 unread")
    );
    assert_eq!(l(&s, cx, "archive-row-9-saved").as_deref(), Some("1 saved"));
    assert_eq!(l(&s, cx, "archive-row-8-items").as_deref(), Some("1 items"));
    assert_eq!(
        l(&s, cx, "archive-row-8-unread").as_deref(),
        Some("0 unread")
    );
    assert_eq!(l(&s, cx, "archive-row-8-saved").as_deref(), Some("0 saved"));
    let date = s.label(cx, "archive-row-9-date").unwrap();
    assert!(
        date.starts_with("Today · ") || date.starts_with("Yesterday · "),
        "{date}"
    );

    s.edit(cx, |m| m.archives.truncate(1));
    assert_eq!(
        s.label(cx, "archive-subtitle").as_deref(),
        Some("1 briefing. Newest first.")
    );
}

#[gpui_kit::test]
fn archive_drill_in_and_back(cx: &mut TestAppContext) {
    let s = boot(cx);
    let mut b = snapshot(
        7,
        2,
        vec![article(1, "tools", false), article(2, "tools", true)],
    );
    b.errors = vec![SourceError {
        source_id: 1,
        source_name: "Dezeen".into(),
        message: "boom".into(),
    }];
    s.edit(cx, |m| {
        m.archives = vec![b.clone(), snapshot(6, 50, vec![])]
    });
    show(&s, cx, View::Archive);
    assert!(!s.exists(cx, "archive-back"));

    s.click(cx, "archive-row-7");
    assert!(s.exists(cx, "archive-back"));
    assert_eq!(
        s.label(cx, "archive-back").as_deref(),
        Some("All briefings")
    );
    assert_eq!(
        s.label(cx, "content-title").unwrap(),
        radar_core::text::format_run_at(&b.run_at, Local::now())
    );
    assert_eq!(
        s.label(cx, "archive-detail-meta").as_deref(),
        Some("2 articles · 1 unread · 1 source failed")
    );
    assert!(s.exists(cx, "archive-7/card-1"));
    assert!(s.exists(cx, "archive-7/card-2"));
    assert!(!s.exists(cx, "archive-row-7"));

    s.click(cx, "archive-back");
    assert!(!s.exists(cx, "archive-back"));
    assert!(s.exists(cx, "archive-row-7"));
    assert_eq!(s.label(cx, "content-title").as_deref(), Some("Archive"));

    // Leaving the view forgets the selection.
    s.click(cx, "archive-row-6");
    assert_eq!(
        s.label(cx, "archive-detail-meta").as_deref(),
        Some("0 articles · 0 unread")
    );
    show(&s, cx, View::Briefing);
    show(&s, cx, View::Archive);
    assert!(s.exists(cx, "archive-row-7"));
}

#[gpui_kit::test]
fn archive_cards_toggles_are_noops(cx: &mut TestAppContext) {
    let s = boot(cx);
    live_with_one_bookmark(&s, cx);
    // The same article ids live in the briefing and in the snapshot; the
    // scoped ids keep them apart.
    s.edit(cx, |m| {
        m.archives = vec![snapshot(4, 3, m.articles.clone())];
    });
    show(&s, cx, View::Archive);
    s.click(cx, "archive-row-4");
    assert!(s.exists(cx, "archive-4/card-1"));
    assert!(!s.exists(cx, "card-1"), "briefing ids must not leak in");

    s.click(cx, "archive-4/card-1");
    s.click(cx, "archive-4/card-1-bookmark");
    s.click(cx, "archive-4/card-2-bookmark");
    s.read(cx, |m| {
        assert!(!m.model.articles[0].read, "live read state untouched");
        assert!(!m.model.articles[0].bookmarked, "live bookmark untouched");
        assert!(m.model.articles[1].bookmarked, "live bookmark untouched");
        let snap = &m.model.archives[0].articles;
        assert!(!snap[0].read && !snap[0].bookmarked && snap[1].bookmarked);
    });
}

#[gpui_kit::test]
fn archive_empty_state_copy(cx: &mut TestAppContext) {
    let s = boot(cx);
    show(&s, cx, View::Archive);
    assert_eq!(
        s.label(cx, "archive-empty").as_deref(),
        Some("Your archive begins with your first briefing")
    );
    assert_eq!(
        s.label(cx, "archive-subtitle").as_deref(),
        Some("Past briefings. Goes back 90 days by default.")
    );
    assert!(!s.exists(cx, "archive-list"));
}
