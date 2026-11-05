//! Headless UI tests for the briefing view and its cards. Run with
//! `cargo test -p radar-app briefing`.

use crate::workspace::shell_tests::{article, boot, category, Shell};
use chrono::{Duration as Dur, Utc};
use gpui_kit::TestAppContext;
use httpmock::prelude::*;
use radar_core::auth::{AuthConfig, Session, Tokens};
use radar_core::keychain::{KeyStore, MemKeyStore, CLERK_OAUTH_USER};
use radar_core::sync::SyncClient;
use radar_core::types::{Account, Archive, SourceError, View, ViewMode};
use std::sync::Arc;
use std::time::Duration;

fn account() -> Account {
    Account {
        id: "user_1".into(),
        email: "a@b.co".into(),
        name: String::new(),
        image_url: None,
    }
}

fn signed_in_client(base: &str) -> Arc<SyncClient> {
    let store = Arc::new(MemKeyStore::new());
    let tokens = Tokens {
        access_token: "at".into(),
        refresh_token: Some("rt".into()),
        expires_at: Utc::now().timestamp() + 3600,
        id_token: None,
    };
    store
        .set(CLERK_OAUTH_USER, &serde_json::to_string(&tokens).unwrap())
        .unwrap();
    let cfg = AuthConfig {
        frontend_api: base.to_string(),
        client_id: "test".into(),
        redirect_port: 0,
    };
    Arc::new(SyncClient::new(base.to_string(), Session::load(cfg, store)))
}

/// Waits (in real time) for the tokio runtime to finish background work.
fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut() -> bool) {
    cx.executor().allow_parking();
    for _ in 0..500 {
        cx.run_until_parked();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn three_articles(s: &Shell, cx: &mut TestAppContext) {
    s.edit(cx, |m| {
        m.categories = vec![category("packaging"), category("tools")];
        m.articles = vec![
            article(1, "packaging", false),
            article(2, "packaging", true),
            article(3, "tools", false),
        ];
        m.articles[2].bookmarked = true;
    });
}

#[gpui_kit::test]
fn briefing_shows_greeting_with_display_name(cx: &mut TestAppContext) {
    let s = boot(cx);
    let anon = s.label(cx, "content-title").unwrap();
    assert!(anon.ends_with('.') && !anon.contains(','), "{anon}");

    s.edit(cx, |m| m.profile.name = "Ada".into());
    let named = s.label(cx, "content-title").unwrap();
    assert!(named.ends_with(", Ada."), "{named}");
    assert_ne!(named, "Morning Briefing");
}

#[gpui_kit::test]
fn briefing_greeting_falls_back_to_account_email(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.model.update(cx, |m, cx| {
        m.account = Some(account());
        cx.notify();
    });
    cx.run_until_parked();
    let label = s.label(cx, "content-title").unwrap();
    assert!(label.ends_with(", a."), "{label}");
}

#[gpui_kit::test]
fn stats_row_values(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    s.edit(cx, |m| {
        m.archives = vec![Archive {
            id: 1,
            run_at: (Utc::now() - Dur::hours(2)).to_rfc3339(),
            articles: vec![],
            errors: vec![],
        }];
    });
    assert_eq!(s.label(cx, "stat-fetched").as_deref(), Some("3"));
    assert_eq!(s.label(cx, "stat-unread").as_deref(), Some("2"));
    assert_eq!(s.label(cx, "stat-saved").as_deref(), Some("1"));
    assert_eq!(s.label(cx, "stat-last-run").as_deref(), Some("2h ago"));
}

#[gpui_kit::test]
fn stats_last_run_is_never_without_archives(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    assert_eq!(s.label(cx, "stat-last-run").as_deref(), Some("never"));
}

#[gpui_kit::test]
fn category_chips_filter_articles(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    for id in ["card-1", "card-2", "card-3"] {
        assert!(s.exists(cx, id), "{id} under All");
    }
    assert_eq!(s.label(cx, "chip-all").as_deref(), Some("All 3"));
    assert_eq!(s.label(cx, "chip-tools").as_deref(), Some("TOOLS 1"));

    s.click(cx, "chip-tools");
    assert_eq!(s.read(cx, |m| m.model.active_category.clone()), "tools");
    assert!(s.exists(cx, "card-3"));
    assert!(!s.exists(cx, "card-1"));
    assert!(!s.exists(cx, "card-2"));

    s.click(cx, "chip-all");
    assert_eq!(s.read(cx, |m| m.model.active_category.clone()), "all");
    assert!(s.exists(cx, "card-1"));
}

#[gpui_kit::test]
fn no_chips_without_categories(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories.clear());
    assert!(!s.exists(cx, "chip-all"));
}

#[gpui_kit::test]
fn grid_vs_list_follows_prefs(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    assert!(s.exists(cx, "briefing-grid"));
    assert!(s.exists(cx, "card-1"));
    assert!(!s.exists(cx, "briefing-list"));

    s.edit(cx, |m| m.prefs.view_mode = ViewMode::List);
    assert!(s.exists(cx, "briefing-list"));
    assert!(s.exists(cx, "row-1"));
    assert!(!s.exists(cx, "briefing-grid"));
    assert!(!s.exists(cx, "card-1"));
}

#[gpui_kit::test]
fn long_list_is_virtualized(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.prefs.view_mode = ViewMode::List;
        m.articles = (1..=200).map(|i| article(i, "tools", false)).collect();
    });
    assert!(s.exists(cx, "briefing-vlist"));
    assert!(s.exists(cx, "row-1"));
    assert!(!s.exists(cx, "row-200"), "off-screen rows must not render");

    // Up to 50 rows render directly.
    s.edit(cx, |m| m.articles.truncate(50));
    assert!(!s.exists(cx, "briefing-vlist"));
    assert!(s.exists(cx, "row-50"));
}

#[gpui_kit::test]
fn scanning_shows_skeletons(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    s.edit(cx, |m| m.scanning = true);
    assert_eq!(
        s.label(cx, "briefing-subtitle").as_deref(),
        Some("Scanning sources…")
    );
    assert!(s.exists(cx, "skeleton-card-0"));
    assert!(s.exists(cx, "skeleton-card-7"));
    assert!(!s.exists(cx, "skeleton-card-8"));
    assert!(!s.exists(cx, "card-1"));

    s.edit(cx, |m| m.prefs.view_mode = ViewMode::List);
    assert!(s.exists(cx, "skeleton-row-0"));
    assert!(s.exists(cx, "skeleton-row-9"));
    assert!(!s.exists(cx, "skeleton-row-10"));
    assert!(!s.exists(cx, "skeleton-card-0"));
}

#[gpui_kit::test]
fn errors_banner_lists_source_errors(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    assert!(!s.exists(cx, "errors-banner"));
    let err = |id, name: &str| SourceError {
        source_id: id,
        source_name: name.into(),
        message: "boom".into(),
    };
    s.edit(cx, |m| {
        m.briefing_errors = vec![err(1, "Dezeen"), err(2, "It's Nice That")]
    });
    assert_eq!(
        s.label(cx, "errors-banner").as_deref(),
        Some("2 sources failed: Dezeen, It's Nice That")
    );
    s.edit(cx, |m| m.briefing_errors = vec![err(1, "Dezeen")]);
    assert_eq!(
        s.label(cx, "errors-banner").as_deref(),
        Some("1 source failed: Dezeen")
    );
    s.edit(cx, |m| m.scanning = true);
    assert!(!s.exists(cx, "errors-banner"), "hidden while scanning");
}

#[gpui_kit::test]
fn onboarding_shown_when_no_sources_and_hydrated(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.sources.clear();
        m.articles.clear();
        m.archives.clear();
    });
    assert!(s.exists(cx, "onboarding"));
    assert_eq!(
        s.label(cx, "briefing-subtitle").as_deref(),
        Some("Let's get you set up.")
    );
    assert!(
        !s.exists(cx, "stat-fetched"),
        "no stats strip during the tutorial"
    );

    // A source (or any briefing) ends the first-run state.
    s.edit(cx, |m| {
        m.add_source("A".into(), "tools".into(), Some("https://a.test".into()));
    });
    assert!(!s.exists(cx, "onboarding"));
    assert!(s.exists(cx, "stat-fetched"));
}

#[gpui_kit::test]
fn onboarding_hidden_before_hydrated(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.sources.clear();
        m.articles.clear();
        m.archives.clear();
    });
    // Not ready: hidden.
    s.model.update(cx, |m, cx| {
        m.ready = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!s.exists(cx, "onboarding"));

    // Ready and signed in, but the synced resources haven't been pulled.
    s.model.update(cx, |m, cx| {
        m.ready = true;
        m.account = Some(account());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!s.read(cx, |m| m.hydrated()));
    assert!(!s.exists(cx, "onboarding"));

    // All five pulled.
    s.model.update(cx, |m, cx| {
        m.resources_pulled = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(s.exists(cx, "onboarding"));
}

#[gpui_kit::test]
fn onboarding_hidden_while_scanning(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.sources.clear();
        m.articles.clear();
        m.archives.clear();
        m.scanning = true;
    });
    assert!(!s.exists(cx, "onboarding"));
}

#[gpui_kit::test]
fn onboarding_steps_navigate_and_show_progress(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.sources.clear();
        m.articles.clear();
        m.archives.clear();
        m.categories.clear();
        m.profile.name = "Ada".into();
    });
    // Step 1 is done (a name is set): it has no button, the others do.
    assert!(!s.exists(cx, "onboarding-cta-1"));
    assert!(s.exists(cx, "onboarding-cta-2"));
    assert_eq!(s.label(cx, "onboarding-step-1").as_deref(), Some("done"));
    assert_eq!(s.label(cx, "onboarding-step-2").as_deref(), Some("todo"));

    s.click(cx, "onboarding-cta-2");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Categories);

    s.edit(cx, |m| m.prefs.view = View::Briefing);
    s.click(cx, "onboarding-cta-3");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Sources);
    // The Sources view consumed the intent (no categories: no add row).
    assert_eq!(s.ws.read_with(cx, |w, _| w.sources_intent), None);

    s.edit(cx, |m| m.prefs.view = View::Briefing);
    s.click(cx, "onboarding-cta-4");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Settings);
}

#[gpui_kit::test]
fn subtitle_follows_the_briefing_state(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.add_source("A".into(), "tools".into(), Some("https://a.test".into()));
        m.articles.clear();
    });
    assert!(!s.exists(cx, "onboarding"));
    assert_eq!(
        s.label(cx, "briefing-subtitle").as_deref(),
        Some("No briefing yet. Add sources, set feed URLs, then press Morning Briefing.")
    );

    three_articles(&s, cx);
    assert_eq!(
        s.label(cx, "briefing-subtitle").as_deref(),
        Some("Your briefing surfaced 3 articles across 1 source · 2 unread.")
    );
    s.edit(cx, |m| m.articles[2].source = "Other".into());
    assert_eq!(
        s.label(cx, "briefing-subtitle").as_deref(),
        Some("Your briefing surfaced 3 articles across 2 sources · 2 unread.")
    );
}

#[gpui_kit::test]
fn card_click_toggles_read_without_sync(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    s.click(cx, "card-1");
    assert!(s.read(cx, |m| m.model.articles[0].read));
    s.click(cx, "card-1");
    assert!(!s.read(cx, |m| m.model.articles[0].read));
}

#[gpui_kit::test]
fn card_buttons_do_not_toggle_read(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    s.click(cx, "card-1-bookmark");
    s.read(cx, |m| {
        assert!(m.model.articles[0].bookmarked);
        assert!(
            !m.model.articles[0].read,
            "the click must not reach the card"
        );
    });
    assert_eq!(
        s.label(cx, "card-1-bookmark").as_deref(),
        Some("Remove bookmark")
    );
}

#[gpui_kit::test]
fn card_open_opens_url_without_changing_read_state(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let any = server.mock(|when, then| {
        when.any_request();
        then.status(200);
    });
    let s = boot(cx);
    three_articles(&s, cx);
    let client = signed_in_client(&server.base_url());
    s.model.update(cx, |m, _| m.sync = Some(client.clone()));

    s.click(cx, "card-1-open");
    assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/1"));
    assert!(
        !s.read(cx, |m| m.model.articles[0].read),
        "Open must not mark read"
    );
    cx.executor().allow_parking();
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(any.calls(), 0, "no PATCH for Open");
    assert!(client.queued().is_empty());
}

#[gpui_kit::test]
fn row_actions_work(cx: &mut TestAppContext) {
    let s = boot(cx);
    three_articles(&s, cx);
    s.edit(cx, |m| m.prefs.view_mode = ViewMode::List);
    s.click(cx, "row-1");
    assert!(s.read(cx, |m| m.model.articles[0].read));
    s.click(cx, "row-1-bookmark");
    s.read(cx, |m| {
        assert!(m.model.articles[0].bookmarked);
        assert!(
            m.model.articles[0].read,
            "bookmark click must not toggle read"
        );
    });
}

#[gpui_kit::test]
fn card_toggle_read_calls_model_and_queues_patch_when_signed_in(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let patch = server.mock(|when, then| {
        when.method(PATCH)
            .path("/sync/article-states")
            .header("authorization", "Bearer at")
            .json_body(serde_json::json!({"articleId":"1","read":true,"bookmarked":false}));
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });
    let s = boot(cx);
    three_articles(&s, cx);
    let client = signed_in_client(&server.base_url());
    s.model.update(cx, |m, _| m.sync = Some(client.clone()));

    s.click(cx, "card-1");
    assert!(
        s.read(cx, |m| m.model.articles[0].read),
        "model updates at once"
    );
    wait_until(cx, || patch.calls() > 0);
    patch.assert();
    assert!(client.queued().is_empty());

    // Server down: the PATCH lands in the offline queue instead.
    // (Port 9, discard: nothing listens, so the connection is refused.)
    let offline = signed_in_client("http://127.0.0.1:9");
    s.model.update(cx, |m, _| m.sync = Some(offline.clone()));
    s.click(cx, "card-3-bookmark");
    wait_until(cx, || !offline.queued().is_empty());
    let queued = offline.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].method, "PATCH");
    assert_eq!(queued[0].path, "/sync/article-states");
    assert_eq!(queued[0].body["articleId"], "3");
}

#[gpui_kit::test]
fn card_toggle_without_sync_sends_nothing(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let any = server.mock(|when, then| {
        when.any_request();
        then.status(200);
    });
    let s = boot(cx);
    three_articles(&s, cx);
    assert!(s.read(cx, |m| m.sync.is_none()));
    s.click(cx, "card-1");
    cx.executor().allow_parking();
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(any.calls(), 0);
}
