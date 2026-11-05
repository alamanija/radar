//! Headless tests for the Settings view, account sign-in/out from the UI
//! and the sync indicator. Run with `cargo test -p radar-app settings`.

use crate::app_model::{PUSH_DEBOUNCE, SAVE_DEBOUNCE};
use crate::components::sync_indicator::{indicator, Tone};
use crate::platform::autostart;
use crate::test_support::{
    account, serve_empty, serve_states, session, settle, sign_in_as, stamped, wait_until,
};
use crate::workspace::shell_tests::{boot, Shell};
use chrono::NaiveTime;
use gpui_kit::component::time_field::TimeFieldEvent;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{point, px, AppContext as _, ScrollDelta, TestAppContext};
use httpmock::prelude::*;
use radar_core::auth::AuthError;
use radar_core::keychain::{KeyStore, MemKeyStore, ANTHROPIC_USER, CLERK_OAUTH_USER};
use radar_core::store;
use radar_core::sync::SyncStatus;
use radar_core::types::View;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Arc;

fn settings(cx: &mut TestAppContext) -> Shell {
    let s = boot(cx);
    s.edit(cx, |m| m.prefs.view = View::Settings);
    s
}

fn description(s: &Shell, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    cx.update_window(s.window, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).description().map(str::to_owned)
    })
    .unwrap()
}

/// Scrolls the page until `id` is on screen (the page is taller than the
/// test window).
fn reveal(s: &Shell, cx: &mut TestAppContext, id: &'static str) {
    for _ in 0..40 {
        let visible = cx
            .update_window(s.window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some_and(|e| e.visible())
            })
            .unwrap();
        if visible {
            return;
        }
        cx.update_window(s.window, |_, window, cx| {
            let delta = ScrollDelta::Pixels(point(px(0.), px(-120.)));
            window.scroll("content", delta, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    panic!("{id} never scrolled into view");
}

fn click(s: &Shell, cx: &mut TestAppContext, id: &'static str) {
    reveal(s, cx, id);
    s.click(cx, id);
}

fn type_text(s: &Shell, cx: &mut TestAppContext, id: &'static str, text: &str) {
    click(s, cx, id);
    cx.update_window(s.window, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn notifications(s: &Shell, cx: &mut TestAppContext) -> usize {
    cx.update_window(s.window, |_, window, cx| window.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn settings_sections_follow_the_jsx(cx: &mut TestAppContext) {
    let s = settings(cx);
    assert_eq!(s.label(cx, "content-title").as_deref(), Some("Settings"));
    assert_eq!(
        s.label(cx, "settings-subtitle").as_deref(),
        Some(
            "Radar runs locally by default. Sign in to sync your sources, categories, \
             profile, and read state across devices."
        )
    );
    let sections = [
        ("Account", "Enables cross-device sync"),
        ("Profile", "Shown in the greeting and sidebar"),
        ("Briefing", "Controls when and how Radar scans"),
        ("Curation", "Anthropic Claude filters and summarizes"),
        ("Appearance", "Local preferences"),
        ("Updates", "Signed releases from GitHub"),
        ("Integrations", "Not yet shipped"),
    ];
    for (i, (title, sub)) in sections.iter().enumerate() {
        let id = format!("settings-section-{i}");
        assert_eq!(s.label(cx, id.clone()).as_deref(), Some(*title), "{id}");
        assert_eq!(
            s.label(cx, format!("{id}-sub")).as_deref(),
            Some(*sub),
            "{id}"
        );
    }
    assert_eq!(s.label(cx, "settings-staleness").as_deref(), Some("—"));
    assert_eq!(
        s.label(cx, "settings-row-staleness-sub").as_deref(),
        Some("Not wired — articles aren't cached across briefings yet.")
    );
    assert_eq!(s.label(cx, "settings-model").as_deref(), Some("Opus 4.7"));
    assert_eq!(
        s.label(cx, "settings-row-gmail-label").as_deref(),
        Some("Gmail — newsletter inbox")
    );
    // The Gmail button is disabled.
    click(&s, cx, "settings-gmail-connect");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Settings);

    // Lens: "Add" while empty, then a 4-row textarea with the JSX placeholder.
    assert_eq!(s.label(cx, "settings-lens-edit").as_deref(), Some("Add"));
    click(&s, cx, "settings-lens-edit");
    let lens =
        s.ws.read_with(cx, |w, _| w.settings.form.as_ref().unwrap().lens.clone());
    let placeholder = lens
        .unwrap()
        .read_with(cx, |l, _| l.presentation().placeholder().to_string());
    assert_eq!(
        placeholder,
        "e.g. Brand studio founder focused on packaging and editorial illustration. \
         Prefer craft over trend."
    );
    type_text(&s, cx, "settings-lens-input", "Packaging first");
    click(&s, cx, "settings-lens-save");
    assert_eq!(
        s.read(cx, |m| m.model.profile.lens.clone()),
        "Packaging first"
    );
    assert_eq!(s.label(cx, "settings-lens-edit").as_deref(), Some("Edit"));
    click(&s, cx, "settings-lens-edit");
    click(&s, cx, "settings-lens-clear");
    assert_eq!(s.read(cx, |m| m.model.profile.lens.clone()), "");
}

#[gpui_kit::test]
fn api_key_save_shows_preview_never_raw(cx: &mut TestAppContext) {
    const RAW: &str = "  sk-ant-api03-abcdefXYZ  ";
    let s = boot(cx);
    let store = Arc::new(MemKeyStore::new());
    s.model.update(cx, |m, _| m.key_store = store.clone());
    s.edit(cx, |m| m.prefs.view = View::Settings);
    wait_until(cx, |cx| s.exists(cx, "settings-api-key-add"));
    assert_eq!(
        s.label(cx, "settings-api-key-add").as_deref(),
        Some("Add key")
    );

    click(&s, cx, "settings-api-key-add");
    let draft = s.ws.read_with(cx, |w, _| {
        w.settings.form.as_ref().unwrap().api.draft.clone().unwrap()
    });
    assert_eq!(
        draft.read_with(cx, |d, _| d.presentation().placeholder().to_string()),
        "sk-ant-…"
    );
    // Save does nothing while the draft is empty.
    click(&s, cx, "settings-api-key-save");
    assert!(s.exists(cx, "settings-api-key-input"));

    type_text(&s, cx, "settings-api-key-input", RAW);
    click(&s, cx, "settings-api-key-save");
    wait_until(cx, |cx| s.exists(cx, "settings-api-key-preview"));

    assert_eq!(
        s.label(cx, "settings-api-key-preview").as_deref(),
        Some("sk-ant-••••••XYZ")
    );
    assert!(
        !s.exists(cx, "settings-api-key-input"),
        "draft input closed"
    );
    assert_eq!(
        store.get(ANTHROPIC_USER).unwrap().as_deref(),
        Some("sk-ant-api03-abcdefXYZ")
    );
    s.ws.read_with(cx, |w, _| {
        let api = &w.settings.form.as_ref().unwrap().api;
        assert!(api.draft.is_none(), "the raw draft is dropped after saving");
        assert!(api.present);
        assert_eq!(api.preview.as_deref(), Some("sk-ant-••••••XYZ"));
        assert!(api.error.is_none());
    });
    assert!(s.exists(cx, "settings-api-key-replace"));

    click(&s, cx, "settings-api-key-clear");
    wait_until(cx, |cx| s.exists(cx, "settings-api-key-add"));
    assert_eq!(store.get(ANTHROPIC_USER).unwrap(), None);
}

#[gpui_kit::test]
fn profile_edit_persists_and_marks_profile_dirty(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty(&server);
    let body = json!({"name": "Maya", "role": null, "lens": null});
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/profile")
            .header_missing("if-match")
            .json_body(body.clone());
        then.status(200).json_body(stamped(body.clone(), "t2"));
    });
    let s = settings(cx);
    sign_in_as(&s.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |cx| s.read(cx, |m| m.resources_pulled));

    type_text(&s, cx, "settings-profile-name", "Maya");
    assert_eq!(s.read(cx, |m| m.model.profile.name.clone()), "Maya");
    assert!(
        s.read(cx, |m| m.resources.profile.needs_push(&m.model.profile)),
        "the profile resource is dirty"
    );

    cx.executor().advance_clock(SAVE_DEBOUNCE);
    cx.run_until_parked();
    assert_eq!(store::load(&s.path).profile.name, "Maya");
    assert_eq!(put.calls(), 0, "the push waits for its 800 ms debounce");

    cx.executor().advance_clock(PUSH_DEBOUNCE);
    wait_until(cx, |_| put.calls() == 1);
    wait_until(cx, |cx| {
        s.read(cx, |m| {
            !m.resources.profile.needs_push(&m.model.profile)
                && m.resources.profile.etag.as_deref() == Some("t2")
        })
    });

    type_text(&s, cx, "settings-profile-role", "AD");
    assert_eq!(s.read(cx, |m| m.model.profile.role.clone()), "AD");
}

#[gpui_kit::test]
fn schedule_toggle_updates_prefs(cx: &mut TestAppContext) {
    let s = settings(cx);
    let (enabled, launch) = s.read(cx, |m| {
        (m.model.prefs.schedule_enabled, m.model.prefs.run_at_launch)
    });
    click(&s, cx, "settings-schedule-toggle");
    assert_eq!(s.read(cx, |m| m.model.prefs.schedule_enabled), !enabled);
    click(&s, cx, "settings-run-at-launch");
    assert_eq!(s.read(cx, |m| m.model.prefs.run_at_launch), !launch);

    // The time field shows the pref and writes edits back as HH:MM.
    let field = s.ws.read_with(cx, |w, _| {
        w.settings.form.as_ref().unwrap().schedule.clone()
    });
    let hm = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
    assert_eq!(field.read_with(cx, |f, _| f.time()), hm(7, 30));
    field.update(cx, |_, cx| cx.emit(TimeFieldEvent::Change(hm(8, 5))));
    cx.run_until_parked();
    assert_eq!(s.read(cx, |m| m.model.prefs.schedule_time.clone()), "08:05");

    // A pref changed elsewhere (e.g. a sync pull) shows up in the field.
    s.edit(cx, |m| m.prefs.schedule_time = "21:45".into());
    s.label(cx, "content-title");
    assert_eq!(field.read_with(cx, |f, _| f.time()), hm(21, 45));

    cx.executor().advance_clock(SAVE_DEBOUNCE);
    cx.run_until_parked();
    let saved = store::load(&s.path).prefs;
    assert_eq!(saved.schedule_enabled, !enabled);
    assert_eq!(saved.schedule_time, "21:45");

    // Open at login shows what the login-item backend reports, and a
    // toggle re-reads it instead of flipping a local mirror. Both calls run
    // off the UI thread. The default backend is always off.
    let autostart = |s: &Shell, cx: &mut TestAppContext| {
        s.ws.read_with(cx, |w, _| w.settings.form.as_ref().unwrap().autostart)
    };
    assert!(!autostart(&s, cx));
    click(&s, cx, "settings-autostart");
    settle(cx);
    assert!(!autostart(&s, cx), "a backend that stays off shows off");

    // A registered login item shows on when Settings opens, and the toggle
    // goes through the backend.
    let registered = Arc::new(AtomicBool::new(true));
    s.model.update(cx, |m, _| {
        let (read, write) = (registered.clone(), registered.clone());
        m.hooks.autostart = autostart::Backend {
            is_enabled: Arc::new(move || read.load(SeqCst)),
            set: Arc::new(move |on| {
                write.store(on, SeqCst);
                Ok(())
            }),
        };
    });
    s.edit(cx, |m| m.prefs.view = View::Briefing);
    s.edit(cx, |m| m.prefs.view = View::Settings);
    s.label(cx, "content-title");
    wait_until(cx, |cx| autostart(&s, cx));
    click(&s, cx, "settings-autostart");
    wait_until(cx, |cx| !autostart(&s, cx));
    assert!(!registered.load(SeqCst), "the backend was switched off");
}

#[gpui_kit::test]
fn sign_in_failure_returns_to_signed_out(cx: &mut TestAppContext) {
    let s = settings(cx);
    let (signed_out, _) = session("http://127.0.0.1:9", None);
    s.model.update(cx, |m, cx| {
        m.session = Some(signed_out);
        m.sign_in_impl = Arc::new(|_, open, _| {
            open("https://clerk.test/oauth/authorize?state=x");
            Box::pin(async { Err(AuthError::Timeout) })
        });
        cx.notify();
    });
    assert_eq!(
        s.label(cx, "settings-row-account-label").as_deref(),
        Some("Sign in")
    );

    click(&s, cx, "settings-sign-in");
    wait_until(cx, |cx| !s.read(cx, |m| m.signing_in));

    assert!(s.read(cx, |m| m.account.is_none() && m.sync.is_none()));
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://clerk.test/oauth/authorize?state=x"),
        "the browser was opened from the UI thread"
    );
    assert_eq!(notifications(&s, cx), 1, "the error is shown");
    assert_eq!(
        s.label(cx, "settings-row-account-label").as_deref(),
        Some("Sign in")
    );
    assert!(s.exists(cx, "settings-sign-in"));
}

#[gpui_kit::test]
fn sign_in_disabled_without_clerk_config(cx: &mut TestAppContext) {
    let s = settings(cx);
    assert!(s.read(cx, |m| m.session.is_none()));
    assert!(s.exists(cx, "settings-sign-in-hint"));
    click(&s, cx, "settings-sign-in");
    assert!(!s.read(cx, |m| m.signing_in));
    assert_eq!(notifications(&s, cx), 0);
}

#[gpui_kit::test]
fn sign_out_resets_resources_and_hides_account(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let remote = json!({"name": "Ada", "role": "AD", "lens": null});
    server.mock(|when, then| {
        when.method(GET).path("/sync/profile");
        then.status(200).json_body(stamped(remote.clone(), "t1"));
    });
    for path in [
        "/sync/categories",
        "/sync/sources",
        "/sync/archives",
        "/sync/prefs",
    ] {
        server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(404);
        });
    }
    serve_states(&server, json!([]));
    server.mock(|when, then| {
        when.method(PUT).path("/sync/prefs");
        then.status(200)
            .json_body(json!({"prefs": {}, "updatedAt": "p1"}));
    });

    let s = settings(cx);
    let (session, store) = session(&server.base_url(), Some("at"));
    let base = server.base_url();
    s.model.update(cx, |m, cx| {
        m.session = Some(session.clone());
        m.server_base = base;
        m.sync_sleep = Some(crate::test_support::instant_sleep());
        m.sign_in_impl = Arc::new(|_, _, _| Box::pin(async { Ok(account("user_1")) }));
        cx.notify();
    });
    click(&s, cx, "settings-sign-in");
    wait_until(cx, |cx| s.read(cx, |m| m.resources_pulled));

    assert_eq!(
        s.label(cx, "settings-row-account-label").as_deref(),
        Some("Signed in")
    );
    assert_eq!(
        s.label(cx, "settings-account-email").as_deref(),
        Some("user_1@example.test")
    );
    assert_eq!(s.read(cx, |m| m.model.profile.name.clone()), "Ada");
    assert!(s.exists(cx, "sync-indicator"));
    assert_eq!(
        s.read(cx, |m| m.resources.profile.etag.clone()).as_deref(),
        Some("t1")
    );

    click(&s, cx, "settings-sign-out");
    s.read(cx, |m| {
        assert!(m.account.is_none() && m.sync.is_none());
        assert!(!m.resources_pulled && !m.article_states_pulled);
        let r = &m.resources;
        assert!(!r.profile.pulled && r.profile.server_key.is_none() && r.profile.etag.is_none());
        assert!(!r.categories.pulled && r.categories.server_key.is_none());
        assert!(!r.sources.pulled && r.sources.server_key.is_none());
        assert!(!r.prefs.pulled && r.prefs.server_key.is_none() && r.prefs.etag.is_none());
        assert!(!r.archives.pulled && r.archives.server_key.is_none());
        assert_eq!(m.model.profile.name, "Ada", "local data stays");
    });
    assert!(!s.exists(cx, "sync-indicator"));
    assert_eq!(
        s.label(cx, "settings-row-account-label").as_deref(),
        Some("Sign in")
    );
    wait_until(cx, |_| store.get(CLERK_OAUTH_USER).unwrap().is_none());
    assert!(!session.is_signed_in());
}

#[gpui_kit::test]
fn sync_indicator_states(cx: &mut TestAppContext) {
    let now = 1_800_000_000_000;
    let st = |f: fn(&mut SyncStatus)| {
        let mut s = SyncStatus::default();
        f(&mut s);
        indicator(&s, now)
    };
    let idle = st(|_| {});
    assert_eq!(idle.tone, Tone::Idle);
    assert_eq!(idle.label, "Synced");
    assert_eq!(idle.title, "All changes saved to Radar");
    let synced = indicator(
        &SyncStatus {
            last_success_at: Some(now - 120_000),
            ..SyncStatus::default()
        },
        now,
    );
    assert_eq!(synced.label, "Synced");
    assert_eq!(synced.title, "All changes saved to Radar · 2m ago");

    let queued = st(|s| s.pending = 2);
    assert_eq!(queued.tone, Tone::Busy);
    assert_eq!(queued.label, "Syncing 2 changes…");
    assert_eq!(
        queued.title,
        "Changes will keep retrying until they reach the server"
    );
    assert_eq!(st(|s| s.pending = 1).label, "Syncing 1 change…");
    assert_eq!(st(|s| s.inflight = true).label, "Syncing…");

    let error = st(|s| s.last_error = Some("503 on /sync/prefs".into()));
    assert_eq!(error.tone, Tone::Error);
    assert_eq!(error.label, "Sync issue");
    assert_eq!(error.title, "503 on /sync/prefs");
    let retrying = st(|s| {
        s.last_error = Some("503 on /sync/prefs".into());
        s.pending = 1;
    });
    assert_eq!(retrying.tone, Tone::Busy, "queued work wins over the error");

    // Rendered in the sidebar only while signed in.
    let s = boot(cx);
    assert!(!s.exists(cx, "sync-indicator"));
    s.model.update(cx, |m, cx| {
        m.account = Some(account("user_1"));
        m.sync_status.pending = 1;
        cx.notify();
    });
    assert_eq!(
        s.label(cx, "sync-indicator").as_deref(),
        Some("Syncing 1 change…")
    );
    assert_eq!(
        description(&s, cx, "sync-indicator").as_deref(),
        Some("Changes will keep retrying until they reach the server")
    );
    s.model.update(cx, |m, cx| {
        m.sync_status = SyncStatus {
            last_error: Some("400 on /sync/prefs".into()),
            ..SyncStatus::default()
        };
        cx.notify();
    });
    assert_eq!(s.label(cx, "sync-indicator").as_deref(), Some("Sync issue"));
    assert_eq!(
        description(&s, cx, "sync-indicator").as_deref(),
        Some("400 on /sync/prefs")
    );
}
