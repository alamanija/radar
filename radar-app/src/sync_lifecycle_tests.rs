//! Sync lifecycle tests against httpmock servers: account switch, pull
//! gating and retry, the persisted queue, timer drains feeding etags back,
//! `flush_now`, and a lost session. Run with
//! `cargo test -p radar-app sync_lifecycle`.

use super::super::AppModel;
use super::*;
use crate::app_model::SAVE_DEBOUNCE;
use crate::test_support::{
    account, id_token, serve_empty, serve_states, session, session_with, settle, sign_in_as,
    stamped, wait_until,
};
use gpui_kit::{Entity, TestAppContext};
use httpmock::prelude::*;
use radar_core::keychain::{KeyStore, CLERK_OAUTH_USER};
use radar_core::store;
use radar_core::sync::resource::SyncResource;
use radar_core::sync::resources::{ArchivesRes, CategoriesRes, PrefsRes};
use radar_core::types::{Archive, Prefs, SourceHealth, Theme, View};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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

/// Every resource GET except `except` answers 404 (article states: empty),
/// and the prefs seed PUT is echoed.
fn serve_empty_except(server: &MockServer, except: &str) {
    let paths = [
        "/sync/profile",
        "/sync/categories",
        "/sync/sources",
        "/sync/prefs",
        "/sync/archives",
    ];
    not_found(
        server,
        &paths
            .into_iter()
            .filter(|p| *p != except)
            .collect::<Vec<_>>(),
    );
    serve_states(server, json!([]));
    if except != "/sync/prefs" {
        echo_prefs(server);
    }
}

#[gpui_kit::test]
fn prefs_pull_keeps_per_device_prefs_changed_meanwhile(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/prefs");
    let mut remote = PrefsRes::push_body(&Prefs::default());
    remote["prefs"]["theme"] = json!("dark");
    let get = server.mock(|when, then| {
        when.method(GET).path("/sync/prefs");
        then.status(200)
            .delay(Duration::from_millis(600))
            .json_body(stamped(remote.clone(), "p1"));
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |_| get.calls() == 1);
    assert!(h.read(cx, |m| m.resources_out), "the pull is still running");
    // Per-device navigation while the prefs GET is in flight.
    h.edit(cx, |r| {
        r.prefs.view = View::Archive;
        r.prefs.sidebar_collapsed = true;
    });
    wait_until(cx, |cx| h.read(cx, |m| m.resources.prefs.pulled));
    h.read(cx, |m| {
        assert_eq!(m.model.prefs.theme, Theme::Dark, "the synced prefs landed");
        assert_eq!(m.model.prefs.view, View::Archive, "navigation survives");
        assert!(
            m.model.prefs.sidebar_collapsed,
            "the sidebar state survives"
        );
        assert!(!m.resources.prefs.needs_push(&m.model.prefs));
    });
}

fn archive(id: i64, run_at: &str) -> Archive {
    Archive {
        id,
        run_at: run_at.into(),
        articles: vec![],
        errors: vec![],
    }
}

#[gpui_kit::test]
fn archive_added_during_pull_survives_and_is_pushed(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/archives");
    let remote = json!({"archives": [archive(1, "2026-11-01T07:30:00Z")]});
    let get = server.mock(|when, then| {
        when.method(GET).path("/sync/archives");
        then.status(200)
            .delay(Duration::from_millis(600))
            .json_body(stamped(remote.clone(), "a1"));
    });

    let h = host(cx);
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |_| get.calls() == 1);
    assert!(h.read(cx, |m| m.resources_out), "the pull is still running");
    // A briefing finishes while the archives GET is in flight.
    h.edit(cx, |r| {
        r.archives.insert(0, archive(2, "2026-11-04T07:30:00Z"))
    });
    wait_until(cx, |cx| h.read(cx, |m| m.resources.archives.pulled));

    let ids = h.read(cx, |m| {
        m.model.archives.iter().map(|a| a.id).collect::<Vec<_>>()
    });
    assert_eq!(
        ids,
        vec![2, 1],
        "the new snapshot joins the server's, newest first"
    );
    assert!(
        h.read(cx, |m| m.resources.archives.needs_push(&m.model.archives)),
        "the server doesn't have the new snapshot yet"
    );
    let local = h.read(cx, |m| ArchivesRes::push_body(&m.model.archives));
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/archives")
            .header("if-match", "W/\"a1\"")
            .json_body(local.clone());
        then.status(200).json_body(stamped(local.clone(), "a2"));
    });
    cx.executor().advance_clock(PUSH_DEBOUNCE);
    wait_until(cx, |_| put.calls() == 1);
}

#[gpui_kit::test]
fn source_stamps_during_pull_survive(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/sources");
    let remote = json!({"sources": [
        {"id": 1, "name": "Dieline", "category": "packaging", "feedUrl": null,
         "enabled": true, "isDefault": false}
    ]});
    let get = server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .delay(Duration::from_millis(600))
            .json_body(stamped(remote.clone(), "s1"));
    });

    let h = host(cx);
    h.edit(cx, |r| {
        r.add_source("Dieline".into(), "packaging".into(), None);
    });
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |_| get.calls() == 1);
    assert!(h.read(cx, |m| m.resources_out), "the pull is still running");
    // A briefing stamps the source while the sources GET is in flight.
    h.edit(cx, |r| {
        r.sources[0].last_fetch_at = Some(1_800_000_000_000);
        r.sources[0].health = SourceHealth::Warn;
    });
    wait_until(cx, |cx| h.read(cx, |m| m.resources.sources.pulled));
    h.read(cx, |m| {
        let s = &m.model.sources[0];
        assert_eq!(s.last_fetch_at, Some(1_800_000_000_000));
        assert_eq!(s.health, SourceHealth::Warn);
        assert!(
            !m.resources.sources.needs_push(&m.model.sources),
            "per-device fields never make sources dirty"
        );
    });
}

/// Account A's drain is asleep in its retry backoff when A signs out and B
/// signs in on the same `Session` (clones share tokens). A's queued write
/// must never go out afterwards, least of all with B's bearer token.
#[gpui_kit::test]
fn account_boundary_aborts_in_flight_drain(cx: &mut TestAppContext) {
    let server = MockServer::start();
    not_found(
        &server,
        &[
            "/sync/profile",
            "/sync/categories",
            "/sync/sources",
            "/sync/prefs",
            "/sync/archives",
        ],
    );
    serve_states(&server, json!([]));
    echo_prefs(&server);
    let as_a = server.mock(|when, then| {
        when.method(PATCH)
            .path("/sync/article-states")
            .header("authorization", "Bearer at-a");
        then.status(503);
    });
    let as_b = server.mock(|when, then| {
        when.method(PATCH)
            .path("/sync/article-states")
            .header("authorization", "Bearer at-b");
        then.status(200).json_body(json!({}));
    });
    server.mock(|when, then| {
        when.method(POST).path("/oauth/token");
        then.status(200).json_body(json!({
            "access_token": "at-b",
            "refresh_token": "rt-b",
            "expires_in": 3600,
            "id_token": id_token("user_b"),
        }));
    });
    let file = json!({
        "version": 1,
        "sync-queue": [
            {"id": "q1", "path": "/sync/article-states", "method": "PATCH",
             "body": {"articleId": "7", "read": true, "bookmarked": false},
             "groupKey": "PATCH:/sync/article-states:7", "accountId": "user_a",
             "tries": 0, "queuedAt": 1}
        ]
    });
    let h = host_with(cx, Some(file));
    let (session, keys) = session_with(&server.base_url(), Some("at-a"), Some("user_a"));
    let base = server.base_url();
    h.model.update(cx, |m, cx| {
        m.session = Some(session.clone());
        m.account = Some(account("user_a"));
        m.server_base = base;
        // The retry backoff: long enough for the account switch to happen.
        m.sync_sleep = Some(Arc::new(|_| {
            Box::pin(tokio::time::sleep(Duration::from_secs(3)))
        }));
        m.start_sync(cx);
    });
    wait_until(cx, |_| as_a.calls() == 1);
    let boundary = std::time::Instant::now();

    // A signs out; B signs in through the real browser flow on the same
    // session (the "browser" is this test hitting the loopback callback).
    h.model.update(cx, |m, cx| m.sign_out(cx));
    wait_until(cx, |_| !session.is_signed_in());
    h.model.update(cx, |m, cx| m.sign_in(cx));
    wait_until(cx, |cx| cx.opened_url().is_some());
    let authorize = url::Url::parse(&cx.opened_url().unwrap()).unwrap();
    let param = |k: &str| {
        authorize
            .query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    let callback = url::Url::parse(&param("redirect_uri")).unwrap();
    let state = param("state");
    std::thread::spawn(move || {
        use std::io::{Read as _, Write as _};
        let addr = (callback.host_str().unwrap(), callback.port().unwrap());
        let mut conn = std::net::TcpStream::connect(addr).unwrap();
        write!(
            conn,
            "GET /callback?code=c1&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
        )
        .unwrap();
        let mut sink = Vec::new();
        let _ = conn.read_to_end(&mut sink);
    });
    wait_until(cx, |cx| {
        h.read(cx, |m| {
            m.account.as_ref().is_some_and(|a| a.id == "user_b") && m.resources_pulled
        })
    });
    assert!(
        session.is_signed_in(),
        "B is signed in on the shared session"
    );
    assert!(keys.get(CLERK_OAUTH_USER).unwrap().is_some());

    // Past the end of A's backoff sleep.
    while boundary.elapsed() < Duration::from_millis(4000) {
        settle(cx);
    }
    assert_eq!(as_b.calls(), 0, "A's queued write went out with B's token");
    assert_eq!(
        as_a.calls(),
        1,
        "A's drain did not stop at the account boundary"
    );
}
