//! Sync lifecycle tests against httpmock servers: account switch, pull
//! gating and retry, the persisted queue, timer drains feeding etags back,
//! `flush_now`, and a lost session. Run with
//! `cargo test -p radar-app sync_lifecycle`.

use super::super::AppModel;
use super::*;
use crate::app_model::SAVE_DEBOUNCE;
use crate::test_support::{
    account, serve_empty, serve_states, session, settle, sign_in_as, stamped, wait_until,
};
use gpui_kit::{Entity, TestAppContext};
use httpmock::prelude::*;
use radar_core::keychain::{KeyStore, CLERK_OAUTH_USER};
use radar_core::store;
use radar_core::sync::resource::SyncResource;
use radar_core::sync::resources::{CategoriesRes, PrefsRes};
use radar_core::types::Prefs;
use serde_json::json;
use std::path::PathBuf;

struct Host {
    _dir: tempfile::TempDir,
    path: PathBuf,
    model: Entity<AppModel>,
}

fn host_with(cx: &mut TestAppContext, file: Option<serde_json::Value>) -> Host {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("radar.json");
    if let Some(v) = file {
        std::fs::write(&path, v.to_string()).unwrap();
    }
    let p = path.clone();
    let model = cx.update(|cx| AppModel::init(p, None, cx));
    Host {
        _dir: dir,
        path,
        model,
    }
}

fn host(cx: &mut TestAppContext) -> Host {
    host_with(cx, None)
}

impl Host {
    fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&AppModel) -> R) -> R {
        self.model.read_with(cx, |m, _| f(m))
    }

    fn edit(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut radar_core::model::RadarModel)) {
        self.model.update(cx, |m, cx| m.update_model(cx, f));
    }
}

fn keys_and_etags(r: &SyncResources) -> [(Option<String>, Option<String>); 5] {
    [
        (r.profile.server_key.clone(), r.profile.etag.clone()),
        (r.categories.server_key.clone(), r.categories.etag.clone()),
        (r.sources.server_key.clone(), r.sources.etag.clone()),
        (r.prefs.server_key.clone(), r.prefs.etag.clone()),
        (r.archives.server_key.clone(), r.archives.etag.clone()),
    ]
}

fn not_found(server: &MockServer, paths: &[&str]) {
    for path in paths {
        let path = *path;
        server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(404);
        });
    }
}

fn echo_prefs(server: &MockServer) {
    server.mock(|when, then| {
        when.method(PUT).path("/sync/prefs");
        then.status(200)
            .json_body(stamped(PrefsRes::push_body(&Prefs::default()), "p1"));
    });
}

#[gpui_kit::test]
fn account_switch_a_to_b_does_not_leak_server_key(cx: &mut TestAppContext) {
    let profile = json!({"name": "Ada", "role": "AD", "lens": null});
    let a = MockServer::start();
    let get = |server: &MockServer, path: &str, body: serde_json::Value| {
        server.mock(|when, then| {
            when.method(GET)
                .path(path)
                .header("authorization", "Bearer at-a");
            then.status(200).json_body(body);
        });
    };
    get(&a, "/sync/profile", stamped(profile.clone(), "a1"));
    get(
        &a,
        "/sync/categories",
        json!({"categories": [], "updatedAt": "a2"}),
    );
    get(
        &a,
        "/sync/sources",
        json!({"sources": [], "updatedAt": "a3"}),
    );
    get(
        &a,
        "/sync/prefs",
        stamped(PrefsRes::push_body(&Prefs::default()), "a4"),
    );
    get(
        &a,
        "/sync/archives",
        json!({"archives": [], "updatedAt": "a5"}),
    );
    serve_states(&a, json!([]));

    let b = MockServer::start();
    not_found(
        &b,
        &[
            "/sync/profile",
            "/sync/categories",
            "/sync/sources",
            "/sync/prefs",
            "/sync/archives",
        ],
    );
    serve_states(&b, json!([]));
    let leaked = b.mock(|when, then| {
        when.header_exists("if-match");
        then.status(500);
    });
    let seed_profile = b.mock(|when, then| {
        when.method(PUT)
            .path("/sync/profile")
            .header("authorization", "Bearer at-b")
            .header_missing("if-match")
            .json_body(profile.clone());
        then.status(200).json_body(stamped(profile.clone(), "b1"));
    });
    let seed_prefs = b.mock(|when, then| {
        when.method(PUT)
            .path("/sync/prefs")
            .header_missing("if-match");
        then.status(200)
            .json_body(stamped(PrefsRes::push_body(&Prefs::default()), "b4"));
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &a.base_url(), "user_a", "at-a");
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));
    h.read(cx, |m| {
        assert_eq!(m.resources.profile.etag.as_deref(), Some("a1"));
        assert_eq!(m.resources.archives.etag.as_deref(), Some("a5"));
        assert!(m.resources.profile.server_key.is_some());
        assert_eq!(m.model.profile.name, "Ada");
    });

    let (session_b, _) = session(&b.base_url(), Some("at-b"));
    let base_b = b.base_url();
    h.model.update(cx, |m, cx| {
        m.session = Some(session_b);
        m.account = Some(account("user_b"));
        m.server_base = base_b;
        m.start_sync(cx);
        // The reset happens before anything is sent for B.
        for (key, etag) in keys_and_etags(&m.resources) {
            assert!(key.is_none() && etag.is_none(), "A's state leaked");
        }
        assert!(!m.resources_pulled && !m.article_states_pulled);
    });
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));

    seed_profile.assert();
    seed_prefs.assert();
    assert_eq!(leaked.calls(), 0, "no If-Match from account A reached B");
    h.read(cx, |m| {
        assert_eq!(m.resources.profile.etag.as_deref(), Some("b1"));
        assert_eq!(m.resources.prefs.etag.as_deref(), Some("b4"));
        assert_eq!(m.resources.categories.etag, None);
    });
}

#[gpui_kit::test]
fn hydrated_waits_for_all_pulls(cx: &mut TestAppContext) {
    let server = MockServer::start();
    not_found(
        &server,
        &[
            "/sync/profile",
            "/sync/categories",
            "/sync/sources",
            "/sync/prefs",
        ],
    );
    serve_states(&server, json!([]));
    echo_prefs(&server);
    let mut archives_down = server.mock(|when, then| {
        when.method(GET).path("/sync/archives");
        then.status(500);
    });

    let h = host(cx);
    assert!(
        h.read(cx, |m| m.hydrated()),
        "signed out: the disk is enough"
    );
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    assert!(
        !h.read(cx, |m| m.hydrated()),
        "signed in: wait for the pulls"
    );

    wait_until(cx, |cx| h.read(cx, |m| m.resources.profile.pulled));
    h.read(cx, |m| {
        let r = &m.resources;
        assert!(r.categories.pulled && r.sources.pulled && r.prefs.pulled);
        assert!(!r.archives.pulled, "the archives pull failed");
        assert!(!m.resources_pulled);
        assert!(!m.hydrated());
    });

    // The failed pull is retried on the drain schedule.
    archives_down.delete();
    let archives_up = server.mock(|when, then| {
        when.method(GET).path("/sync/archives");
        then.status(404);
    });
    cx.executor().advance_clock(DRAIN_INTERVAL);
    wait_until(cx, |cx| h.read(cx, |m| m.hydrated()));
    archives_up.assert();
    assert!(h.read(cx, |m| m.resources_pulled && m.resources.archives.pulled));
}

#[gpui_kit::test]
fn failed_pull_is_retried_and_keeps_offline_edit(cx: &mut TestAppContext) {
    let server = MockServer::start();
    not_found(
        &server,
        &[
            "/sync/profile",
            "/sync/sources",
            "/sync/prefs",
            "/sync/archives",
        ],
    );
    serve_states(&server, json!([]));
    echo_prefs(&server);
    let mut down = server.mock(|when, then| {
        when.method(GET).path("/sync/categories");
        then.status(503);
    });
    let mut early_put = server.mock(|when, then| {
        when.method(PUT).path("/sync/categories");
        then.status(500);
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |cx| h.read(cx, |m| m.resources.profile.pulled));
    assert!(!h.read(cx, |m| m.resources.categories.pulled));

    // Edited while the server can't be read: kept locally, not pushed.
    h.edit(cx, |r| {
        r.add_category("Type".into(), "Fonts".into(), "olive".into());
    });
    cx.executor().advance_clock(PUSH_DEBOUNCE);
    settle(cx);
    assert_eq!(
        early_put.calls(),
        0,
        "pushes stay gated until a pull succeeds"
    );

    down.delete();
    early_put.delete();
    let remote =
        json!({"categories": [{"id": "x", "label": "X", "description": "", "accent": "rust"}]});
    server.mock(|when, then| {
        when.method(GET).path("/sync/categories");
        then.status(200).json_body(stamped(remote.clone(), "c1"));
    });
    let local = h.read(cx, |m| CategoriesRes::push_body(&m.model.categories));
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/categories")
            .header("if-match", "W/\"c1\"")
            .json_body(local.clone());
        then.status(200).json_body(stamped(local.clone(), "c2"));
    });
    cx.executor().advance_clock(DRAIN_INTERVAL);
    wait_until(cx, |cx| h.read(cx, |m| m.resources.categories.pulled));
    put.assert();
    h.read(cx, |m| {
        assert_eq!(m.model.categories.len(), 1);
        assert_eq!(m.model.categories[0].label, "Type", "the offline edit wins");
        assert_eq!(m.resources.categories.etag.as_deref(), Some("c2"));
        assert!(m.resources_pulled);
    });
}

#[gpui_kit::test]
fn persisted_queue_parses_item_by_item(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty(&server);
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/categories")
            .json_body(json!({"categories": []}));
        then.status(200)
            .json_body(json!({"categories": [], "updatedAt": "c1"}));
    });
    let patch = server.mock(|when, then| {
        when.method(PATCH)
            .path("/sync/article-states")
            .json_body(json!({"articleId": "7", "read": true, "bookmarked": false}));
        then.status(200).json_body(json!({}));
    });
    let file = json!({
        "version": 1,
        "sync-queue": [
            {"id": "q1", "path": "/sync/categories", "method": "PUT",
             "body": {"categories": []}, "headers": {},
             "groupKey": "PUT:/sync/categories", "accountId": null, "tries": 0, "queuedAt": 1},
            42,
            {"method": "PUT"},
            {"id": "q2", "path": "/sync/article-states", "method": "PATCH",
             "body": {"articleId": "7", "read": true, "bookmarked": false},
             "groupKey": "PATCH:/sync/article-states:7", "tries": 0, "queuedAt": 2}
        ]
    });
    let h = host_with(cx, Some(file));
    assert_eq!(h.read(cx, |m| lock(&m.sync_queue).items.len()), 4);

    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |_| put.calls() == 1 && patch.calls() == 1);
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));
    let client = h.read(cx, |m| m.sync.clone().unwrap());
    assert!(client.queued().is_empty(), "both readable items drained");
    assert!(h.read(cx, |m| lock(&m.sync_queue).items.is_empty()));
    cx.executor().advance_clock(SAVE_DEBOUNCE);
    cx.run_until_parked();
    assert!(store::load(&h.path).sync_queue.is_empty());
}

#[gpui_kit::test]
fn queued_put_drains_on_timer_and_feeds_on_drained(cx: &mut TestAppContext) {
    let server = MockServer::start();
    not_found(
        &server,
        &[
            "/sync/profile",
            "/sync/sources",
            "/sync/prefs",
            "/sync/archives",
        ],
    );
    serve_states(&server, json!([]));
    echo_prefs(&server);
    server.mock(|when, then| {
        when.method(GET).path("/sync/categories");
        then.status(200)
            .json_body(json!({"categories": [], "updatedAt": "t1"}));
    });
    // No token can be had: the push and every drain stop without using up a
    // try, so the write waits in the queue.
    let mut rejected = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/categories")
            .header("if-match", "W/\"t1\"");
        then.status(401);
    });
    server.mock(|when, then| {
        when.method(POST).path("/oauth/token");
        then.status(500);
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));
    h.edit(cx, |r| {
        r.add_category("Type".into(), String::new(), "olive".into());
    });
    cx.executor().advance_clock(PUSH_DEBOUNCE);
    wait_until(cx, |cx| h.read(cx, |m| m.sync_status.pending == 1));
    let client = h.read(cx, |m| m.sync.clone().unwrap());
    wait_until(cx, |_| !client.status().borrow().inflight);
    settle(cx);
    let attempts = rejected.calls();
    assert!(attempts >= 1);
    settle(cx);
    assert_eq!(
        rejected.calls(),
        attempts,
        "nothing retries before the timer"
    );

    rejected.delete();
    let local = h.read(cx, |m| CategoriesRes::push_body(&m.model.categories));
    let accepted = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/categories")
            .header("if-match", "W/\"t1\"")
            .json_body(local.clone());
        then.status(200).json_body(stamped(local.clone(), "t2"));
    });
    cx.executor().advance_clock(DRAIN_INTERVAL);
    wait_until(cx, |cx| {
        accepted.calls() == 1 && h.read(cx, |m| m.sync_status.pending == 0)
    });
    wait_until(cx, |cx| {
        h.read(cx, |m| m.resources.categories.etag.as_deref() == Some("t2"))
    });
    assert!(h.read(cx, |m| !m
        .resources
        .categories
        .needs_push(&m.model.categories)));

    // The next push sends the etag the drain brought back.
    let next = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/categories")
            .header("if-match", "W/\"t2\"");
        then.status(200).json_body(json!({"updatedAt": "t3"}));
    });
    h.edit(cx, |r| {
        r.add_category("Ink".into(), String::new(), "ink".into());
    });
    cx.executor().advance_clock(PUSH_DEBOUNCE);
    wait_until(cx, |_| next.calls() == 1);
}

#[gpui_kit::test]
fn flush_now_pushes_without_debounce_and_flushes_store(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty(&server);
    let ada = json!({"name": "Ada", "role": null, "lens": null});
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/profile")
            .header_missing("if-match")
            .json_body(ada.clone());
        then.status(200).json_body(stamped(ada.clone(), "t2"));
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));
    h.edit(cx, |r| r.profile.name = "Ada".into());
    assert_ne!(store::load(&h.path).profile.name, "Ada");

    let flushed = h.model.update(cx, |m, cx| m.flush_now(cx));
    assert_eq!(
        store::load(&h.path).profile.name,
        "Ada",
        "the store is written before flush_now returns"
    );
    futures::executor::block_on(flushed);
    assert_eq!(put.calls(), 1, "pushed without waiting for the debounce");
    wait_until(cx, |cx| {
        h.read(cx, |m| m.resources.profile.etag.as_deref() == Some("t2"))
    });

    // Server unreachable at quit: the push lands in the queue, and the queue
    // is on disk once the quit future resolves.
    server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/profile")
            .header("if-match", "W/\"t2\"");
        then.status(401);
    });
    server.mock(|when, then| {
        when.method(POST).path("/oauth/token");
        then.status(500);
    });
    h.edit(cx, |r| r.profile.name = "Bea".into());
    let flushed = h.model.update(cx, |m, cx| m.flush_now(cx));
    futures::executor::block_on(flushed);
    let queued = store::load(&h.path).sync_queue;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["path"], "/sync/profile");
    assert_eq!(queued[0]["body"]["name"], "Bea");
}

#[gpui_kit::test]
fn lost_session_moves_to_signed_out(cx: &mut TestAppContext) {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/sync/profile");
        then.status(401);
    });
    server.mock(|when, then| {
        when.method(POST).path("/oauth/token");
        then.status(400)
            .json_body(json!({"error": "invalid_grant"}));
    });
    let h = host(cx);
    let keys = sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |cx| h.read(cx, |m| m.account.is_none()));
    h.read(cx, |m| {
        assert!(m.sync.is_none());
        assert!(!m.resources_pulled);
        assert!(m.hydrated(), "back to local-only");
    });
    assert!(keys.get(CLERK_OAUTH_USER).unwrap().is_none());
}
