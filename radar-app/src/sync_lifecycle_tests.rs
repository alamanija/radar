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
use radar_core::sync::resources::{ArchivesRes, CategoriesRes, PrefsRes, SourcesRes};
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

fn sources_row(names: &[&str]) -> serde_json::Value {
    let sources: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            json!({"id": i + 1, "name": name, "category": "packaging", "feedUrl": null,
                   "enabled": true, "isDefault": false})
        })
        .collect();
    json!({ "sources": sources })
}

/// Signs in as `id` and waits for the pulls, then for the debounced save
/// that writes the store and the last-synced digests.
fn synced(h: &Host, cx: &mut TestAppContext, server: &MockServer, id: &str) {
    sign_in_as(&h.model, cx, &server.base_url(), id, "at");
    wait_until(cx, |cx| h.read(cx, |m| m.resources_pulled));
    cx.executor().advance_clock(SAVE_DEBOUNCE);
    settle(cx);
}

/// What a quit app leaves behind: its data dir and store path.
type Leftover = (tempfile::TempDir, PathBuf);

/// The process is gone: its runtime work stops and nothing of it runs again.
fn kill(cx: &mut TestAppContext, h: Host) -> Leftover {
    h.model.update(cx, |m, _| m.rt_work.abort_all());
    let Host { _dir, path, model } = h;
    drop(model);
    cx.run_until_parked();
    (_dir, path)
}

/// Adds a source, then quits while the server never answers the quit push
/// (GPUI stops waiting after 200 ms), and the process ends.
fn edit_then_quit_unanswered(cx: &mut TestAppContext, h: Host, server: &MockServer) -> Leftover {
    let mut silent = server.mock(|when, then| {
        when.method(PUT).path("/sync/sources");
        then.status(200)
            .delay(Duration::from_secs(60))
            .json_body(json!({}));
    });
    h.edit(cx, |r| {
        r.add_source("Brand New".into(), "packaging".into(), None);
    });
    drop(h.model.update(cx, |m, cx| m.flush_now(cx)));
    wait_until(cx, |_| silent.calls() == 1);
    let left = kill(cx, h);
    silent.delete();
    left
}

/// The next launch on what the last one left, signed in as `id`.
fn relaunch(cx: &mut TestAppContext, (dir, path): Leftover, base: &str, id: &str) -> Host {
    let p = path.clone();
    let model = cx.update(|cx| AppModel::init(p, None, cx));
    sign_in_as(&model, cx, base, id, "at");
    wait_until(cx, |cx| model.read_with(cx, |m, _| m.resources_pulled));
    Host {
        _dir: dir,
        path,
        model,
    }
}

/// I1: an edit that never reached the server (the quit push went
/// unanswered) survives the next launch. The last-synced digest shows the
/// local value changed since the last sync, so the first pull pushes it,
/// with the fresh etag, instead of taking the server's older row.
#[gpui_kit::test]
fn unsynced_edit_at_quit_is_pushed_on_relaunch(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/sources");
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .json_body(stamped(sources_row(&["Dieline"]), "s1"));
    });
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let left = edit_then_quit_unanswered(cx, h, &server);
    let local = SourcesRes::push_body(&store::load(&left.1).sources);
    assert_eq!(local["sources"].as_array().unwrap().len(), 2);

    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/sources")
            .header("if-match", "W/\"s1\"")
            .json_body(local.clone());
        then.status(200).json_body(stamped(local.clone(), "s2"));
    });
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    put.assert();
    h.read(cx, |m| {
        assert_eq!(m.model.sources.len(), 2, "the edit survived the restart");
        assert_eq!(m.resources.sources.etag.as_deref(), Some("s2"));
        assert!(!m.resources.sources.needs_push(&m.model.sources));
    });
}

/// The control: nothing changed locally since the last sync, so the
/// server's newer row (another device) replaces the local copy.
#[gpui_kit::test]
fn unchanged_local_takes_the_newer_row_on_relaunch(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/sources");
    let mut old_row = server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .json_body(stamped(sources_row(&["Dieline"]), "s1"));
    });
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let left = kill(cx, h);

    old_row.delete();
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .json_body(stamped(sources_row(&["Dieline", "Other"]), "s2"));
    });
    let put = server.mock(|when, then| {
        when.method(PUT).path("/sync/sources");
        then.status(200).json_body(json!({}));
    });
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    assert_eq!(put.calls(), 0, "nothing local to push");
    let names = h.read(cx, |m| {
        m.model
            .sources
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, ["Dieline", "Other"]);
}

/// Digests written for account A never seed account B: B's row replaces
/// the local copy (A's unsynced edit is not pushed to B).
#[gpui_kit::test]
fn another_accounts_digests_never_seed(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty_except(&server, "/sync/sources");
    let mut a_row = server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .json_body(stamped(sources_row(&["Dieline"]), "s1"));
    });
    let h = host(cx);
    synced(&h, cx, &server, "user_a");
    let left = edit_then_quit_unanswered(cx, h, &server);
    let sidecar =
        radar_core::sync::last_synced::load(&radar_core::sync::last_synced::path_beside(&left.1));
    assert_eq!(sidecar.map(|s| s.account_id).as_deref(), Some("user_a"));

    a_row.delete();
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200)
            .json_body(stamped(sources_row(&["B's feed"]), "b1"));
    });
    let put = server.mock(|when, then| {
        when.method(PUT).path("/sync/sources");
        then.status(200).json_body(json!({}));
    });
    let h = relaunch(cx, left, &server.base_url(), "user_b");
    assert_eq!(put.calls(), 0, "A's edit went to B");
    assert_eq!(h.read(cx, |m| m.model.sources.len()), 1);
}

/// Signing out forgets the digests with the session: the next sign-in's
/// pull takes the server's copy (the sign-out policy).
#[gpui_kit::test]
fn sign_out_removes_the_last_synced_file(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_empty(&server);
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let sidecar = radar_core::sync::last_synced::path_beside(&h.path);
    let state = radar_core::sync::last_synced::load(&sidecar).expect("written with the store");
    assert_eq!(state.account_id, "user_1");
    assert!(state.digests.prefs.is_some());
    assert_eq!(
        state.saved,
        radar_core::sync::last_synced::snapshot_digests(&store::load(&h.path)),
        "saved: what the same save wrote to the store"
    );

    h.model.update(cx, |m, cx| m.sign_out(cx));
    cx.executor().advance_clock(SAVE_DEBOUNCE);
    settle(cx);
    assert!(!sidecar.exists());
}

const ALL: [&str; 5] = [
    "/sync/profile",
    "/sync/categories",
    "/sync/sources",
    "/sync/prefs",
    "/sync/archives",
];

/// The row the server holds for `path` (without `updatedAt`), and its etag.
fn server_row(path: &str) -> (serde_json::Value, &'static str) {
    use crate::workspace::shell_tests::article;
    match path {
        "/sync/profile" => (
            json!({"name": "Ada", "role": "AD", "lens": "Packaging studio"}),
            "pr1",
        ),
        "/sync/categories" => (
            json!({"categories": [
                {"id": "packaging", "label": "Packaging", "description": "Boxes.", "accent": "camel"}
            ]}),
            "c1",
        ),
        "/sync/sources" => (sources_row(&["Dieline"]), "s1"),
        "/sync/prefs" => {
            let prefs = Prefs {
                theme: Theme::Dark,
                ..Prefs::default()
            };
            (PrefsRes::push_body(&prefs), "pf1")
        }
        "/sync/archives" => {
            let archive = Archive {
                id: 1_767_261_000_000,
                run_at: "2026-01-01T09:30:00.000Z".into(),
                articles: vec![article(13_612_357_342_556_406_000, "packaging", false)],
                errors: vec![],
            };
            (ArchivesRes::push_body(&vec![archive]), "a1")
        }
        other => panic!("no row for {other}"),
    }
}

/// A server holding a row for every resource; a relaunch finds the same
/// (old) rows again.
fn serve_rows(server: &MockServer) {
    for path in ALL {
        let (row, etag) = server_row(path);
        server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(200).json_body(stamped(row, etag));
        });
    }
    serve_states(server, json!([]));
}

/// Counts the PUTs each of `paths` receives.
fn count_puts<'a>(
    server: &'a MockServer,
    paths: &[&'static str],
) -> Vec<(&'static str, httpmock::Mock<'a>)> {
    paths
        .iter()
        .map(|&path| {
            let mock = server.mock(|when, then| {
                when.method(PUT).path(path);
                then.status(200).json_body(json!({}));
            });
            (path, mock)
        })
        .collect()
}

/// What the local model holds for `path`, in the server row's shape.
fn local_row(m: &AppModel, path: &str) -> serde_json::Value {
    use radar_core::sync::resources::ProfileRes;
    match path {
        "/sync/profile" => ProfileRes::push_body(&m.model.profile),
        "/sync/categories" => CategoriesRes::push_body(&m.model.categories),
        "/sync/sources" => SourcesRes::push_body(&m.model.sources),
        "/sync/prefs" => PrefsRes::push_body(&m.model.prefs),
        "/sync/archives" => ArchivesRes::push_body(&m.model.archives),
        other => panic!("no resource at {other}"),
    }
}

/// No PUT reached `paths`, and local holds the server's row for each.
fn took_server_rows(
    h: &Host,
    cx: &mut TestAppContext,
    puts: &[(&'static str, httpmock::Mock<'_>)],
) {
    let pushed: Vec<_> = puts
        .iter()
        .filter(|(_, put)| put.calls() > 0)
        .map(|(path, put)| (*path, put.calls()))
        .collect();
    assert!(pushed.is_empty(), "pushed over the server: {pushed:?}");
    for (path, _) in puts {
        let local = h.read(cx, |m| local_row(m, path));
        assert_eq!(local, server_row(path).0, "{path} kept the local copy");
    }
}

/// Synced with a server holding every row, then quit with an unsynced
/// source edit on disk (`edit_then_quit_unanswered`): a seed from what
/// this leaves would diverge for sources.
fn synced_then_unsynced_edit(cx: &mut TestAppContext, server: &MockServer) -> Leftover {
    serve_rows(server);
    let h = host(cx);
    synced(&h, cx, server, "user_1");
    let left = edit_then_quit_unanswered(cx, h, server);
    assert_eq!(store::load(&left.1).sources.len(), 2);
    assert_seeds_everything(&left.1);
    left
}

/// The sidecar beside `store` would seed every resource: it holds synced
/// and saved digests for all five, and the saved ones match the store.
fn assert_seeds_everything(store_path: &std::path::Path) {
    use radar_core::sync::last_synced;
    let state = last_synced::load(&last_synced::path_beside(store_path)).expect("sidecar");
    let loaded = store::load_checked(store_path);
    let saved = last_synced::snapshot_digests(&loaded.snap);
    assert_eq!(state.saved, saved);
    let kept = state.checked(&saved, loaded.clean);
    for d in [
        &kept.digests.profile,
        &kept.digests.categories,
        &kept.digests.sources,
        &kept.digests.prefs,
        &kept.digests.archives,
    ] {
        assert!(d.is_some(), "{:?}", kept.digests);
    }
}

/// N1: `radar.json` is gone while `sync-state.json` stayed. The empty
/// store isn't what the digests were saved with, so nothing seeds: every
/// resource takes the server row, and nothing empty is pushed over it.
#[gpui_kit::test]
fn deleted_store_never_pushes_over_the_server(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let left = synced_then_unsynced_edit(cx, &server);
    std::fs::remove_file(&left.1).unwrap();
    assert!(radar_core::sync::last_synced::path_beside(&left.1).exists());

    let puts = count_puts(&server, &ALL);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    took_server_rows(&h, cx, &puts);
}

/// N1: an unparseable `radar.json` loads as defaults; those are not an
/// edit to push.
#[gpui_kit::test]
fn unparseable_store_never_pushes_over_the_server(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let left = synced_then_unsynced_edit(cx, &server);
    std::fs::write(&left.1, "{not json").unwrap();

    let puts = count_puts(&server, &ALL);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    took_server_rows(&h, cx, &puts);
}

/// N1: another writer (a Tauri downgrade) replaced one resource in
/// `radar.json` with a different valid value. That resource takes the
/// server row; an untouched resource's unsynced edit, still exactly what
/// was saved, is pushed as before.
#[gpui_kit::test]
fn externally_replaced_value_takes_the_server_row(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let left = synced_then_unsynced_edit(cx, &server);
    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&left.1).unwrap()).unwrap();
    raw["profile"] = json!({"name": "Tauri", "role": "Old", "lens": ""});
    std::fs::write(&left.1, raw.to_string()).unwrap();
    let local_sources = SourcesRes::push_body(&store::load(&left.1).sources);

    let others = count_puts(
        &server,
        &[
            "/sync/profile",
            "/sync/categories",
            "/sync/prefs",
            "/sync/archives",
        ],
    );
    let sources_put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/sources")
            .header("if-match", "W/\"s1\"")
            .json_body(local_sources.clone());
        then.status(200)
            .json_body(stamped(local_sources.clone(), "s2"));
    });
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    took_server_rows(&h, cx, &others);
    sources_put.assert();
    h.read(cx, |m| {
        assert_eq!(m.model.sources.len(), 2, "the saved edit was pushed");
        assert_eq!(m.resources.sources.etag.as_deref(), Some("s2"));
    });
}

/// N1: the store write landed but the sidecar write didn't, so
/// `sync-state.json` describes an older save. The changed resource takes
/// the server row instead of being pushed on the strength of stale digests.
#[gpui_kit::test]
fn stale_sidecar_takes_the_server_row(cx: &mut TestAppContext) {
    let server = MockServer::start();
    serve_rows(&server);
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    assert_seeds_everything(&h.path);
    let sidecar = radar_core::sync::last_synced::path_beside(&h.path);
    let older = std::fs::read(&sidecar).unwrap();
    let left = edit_then_quit_unanswered(cx, h, &server);
    assert_eq!(store::load(&left.1).sources.len(), 2);
    std::fs::write(&sidecar, older).unwrap();

    let puts = count_puts(&server, &ALL);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    took_server_rows(&h, cx, &puts);
}

/// N1 re-review M2: damage to a key no digest covers (one malformed
/// article) still makes the load not clean, and that alone stops the seed.
/// Every resource matches `saved`, yet the unsynced edit is not pushed and
/// local takes the server rows.
#[gpui_kit::test]
fn unclean_store_seeds_nothing_even_when_every_resource_matches(cx: &mut TestAppContext) {
    use radar_core::sync::last_synced;
    let server = MockServer::start();
    let left = synced_then_unsynced_edit(cx, &server);
    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&left.1).unwrap()).unwrap();
    raw["articles"] = json!([{"id": "not an article"}]);
    std::fs::write(&left.1, raw.to_string()).unwrap();
    let loaded = store::load_checked(&left.1);
    assert!(!loaded.clean);
    let state = last_synced::load(&last_synced::path_beside(&left.1)).unwrap();
    assert_eq!(
        last_synced::snapshot_digests(&loaded.snap),
        state.saved,
        "only the articles key is damaged"
    );

    let puts = count_puts(&server, &ALL);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    took_server_rows(&h, cx, &puts);
}

/// A server holding `row` for sources (etag `s1`) and nothing else.
fn serve_sources(server: &MockServer, row: serde_json::Value) -> httpmock::Mock<'_> {
    serve_empty_except(server, "/sync/sources");
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200).json_body(stamped(row, "s1"));
    })
}

/// P1: the quit push landed, but the process ended before it could record
/// that. The server already holds the edit, so the relaunch adopts it
/// without pushing it again.
#[gpui_kit::test]
fn landed_quit_push_is_adopted_on_relaunch(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let mut old_row = serve_sources(&server, sources_row(&["Dieline"]));
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let left = edit_then_quit_unanswered(cx, h, &server);
    let edit = assert_stuck_edit(&left.1);

    old_row.delete();
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200).json_body(stamped(edit.clone(), "s2"));
    });
    let puts = count_puts(&server, &["/sync/sources"]);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    assert_eq!(puts[0].1.calls(), 0, "the edit is already on the server");
    h.read(cx, |m| {
        assert_eq!(SourcesRes::push_body(&m.model.sources), edit);
        assert_eq!(m.resources.sources.etag.as_deref(), Some("s2"));
        assert!(!m.resources.sources.needs_push(&m.model.sources));
    });
}

/// P1: another device wrote sources after this device last synced them,
/// and this device's quit push never landed. The server's newer row wins;
/// the stuck edit is not pushed over it.
#[gpui_kit::test]
fn newer_server_row_wins_over_a_stuck_edit_on_relaunch(cx: &mut TestAppContext) {
    let server = MockServer::start();
    let mut old_row = serve_sources(&server, sources_row(&["Dieline"]));
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let left = edit_then_quit_unanswered(cx, h, &server);
    assert_stuck_edit(&left.1);

    old_row.delete();
    let newer = sources_row(&["Dieline", "Other"]);
    server.mock(|when, then| {
        when.method(GET).path("/sync/sources");
        then.status(200).json_body(stamped(newer.clone(), "s2"));
    });
    let puts = count_puts(&server, &["/sync/sources"]);
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    assert_eq!(puts[0].1.calls(), 0, "the stuck edit overwrote a newer row");
    h.read(cx, |m| {
        assert_eq!(SourcesRes::push_body(&m.model.sources), newer);
        assert_eq!(m.resources.sources.etag.as_deref(), Some("s2"));
        assert!(!m.resources.sources.needs_push(&m.model.sources));
    });
}

/// P1 step 3: a launch whose first pull fails keeps the last-synced digest
/// it was seeded with, so the launch after it still recognizes the
/// server's unchanged row and pushes the stuck edit.
#[gpui_kit::test]
fn failed_relaunch_keeps_the_seed_for_the_next_launch(cx: &mut TestAppContext) {
    use radar_core::sync::last_synced;
    let server = MockServer::start();
    serve_sources(&server, sources_row(&["Dieline"]));
    let h = host(cx);
    synced(&h, cx, &server, "user_1");
    let (dir, path) = edit_then_quit_unanswered(cx, h, &server);
    let edit = assert_stuck_edit(&path);
    let sidecar = last_synced::path_beside(&path);
    let seeded_from = last_synced::load(&sidecar).unwrap().digests.sources;

    // Second launch: the server is down, so every pull fails. A per-device
    // change (not synced) makes the quit flush rewrite the sidecar.
    let down = MockServer::start();
    let gets = down.mock(|when, then| {
        when.method(GET);
        then.status(503);
    });
    let p = path.clone();
    let model = cx.update(|cx| AppModel::init(p, None, cx));
    sign_in_as(&model, cx, &down.base_url(), "user_1", "at");
    wait_until(cx, |cx| {
        gets.calls() >= 6 && !model.read_with(cx, |m, _| m.resources_out)
    });
    assert!(!model.read_with(cx, |m, _| m.resources.sources.pulled));
    model.update(cx, |m, cx| {
        m.update_model(cx, |r| r.prefs.sidebar_collapsed = true)
    });
    drop(model.update(cx, |m, cx| m.flush_now(cx)));
    let left = kill(
        cx,
        Host {
            _dir: dir,
            path,
            model,
        },
    );
    assert!(store::load(&left.1).prefs.sidebar_collapsed, "flushed");
    assert_eq!(
        last_synced::load(&sidecar).unwrap().digests.sources,
        seeded_from,
        "the failed launch kept the digest it was seeded with"
    );

    // Third launch: the server is back with the row last synced.
    let put = server.mock(|when, then| {
        when.method(PUT)
            .path("/sync/sources")
            .header("if-match", "W/\"s1\"")
            .json_body(edit.clone());
        then.status(200).json_body(stamped(edit.clone(), "s2"));
    });
    let h = relaunch(cx, left, &server.base_url(), "user_1");
    put.assert();
    h.read(cx, |m| {
        assert_eq!(SourcesRes::push_body(&m.model.sources), edit);
        assert_eq!(m.resources.sources.etag.as_deref(), Some("s2"));
    });
}

/// The store beside the sidecar holds an edit to sources the server never
/// confirmed: `saved` describes it, the last-synced digest an older value.
/// Returns the edit in the wire shape.
fn assert_stuck_edit(store_path: &std::path::Path) -> serde_json::Value {
    use radar_core::sync::last_synced;
    let state = last_synced::load(&last_synced::path_beside(store_path)).expect("sidecar");
    let snap = store::load(store_path);
    assert_eq!(snap.sources.len(), 2);
    assert_eq!(state.saved, last_synced::snapshot_digests(&snap));
    assert!(state.digests.sources.is_some());
    assert_ne!(state.digests.sources, state.saved.sources);
    SourcesRes::push_body(&snap.sources)
}

/// M2: a toggle made while the article-states pull is in flight is newer
/// than the server's copy and survives the pull; the server's rows still
/// land for the articles nobody touched.
#[gpui_kit::test]
fn toggle_during_states_pull_survives(cx: &mut TestAppContext) {
    use crate::workspace::shell_tests::article;
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
    echo_prefs(&server);
    let get = server.mock(|when, then| {
        when.method(GET).path("/sync/article-states");
        then.status(200)
            .delay(Duration::from_millis(600))
            .json_body(json!({"states": [
                {"articleId": "1", "read": false, "bookmarked": true},
                {"articleId": "2", "read": true, "bookmarked": false},
            ]}));
    });
    let patch = server.mock(|when, then| {
        when.method(PATCH)
            .path("/sync/article-states")
            .json_body(json!({"articleId": "1", "read": true, "bookmarked": false}));
        then.status(200).json_body(json!({}));
    });

    let h = host(cx);
    h.edit(cx, |r| {
        r.articles = vec![article(1, "tools", false), article(2, "tools", false)]
    });
    sign_in_as(&h.model, cx, &server.base_url(), "user_1", "at");
    wait_until(cx, |_| get.calls() == 1);
    assert!(h.read(cx, |m| m.resources_out), "the pull is still running");
    h.model.update(cx, |m, cx| m.toggle_read(cx, 1));
    wait_until(cx, |cx| h.read(cx, |m| m.article_states_pulled));

    wait_until(cx, |_| patch.calls() == 1);
    h.read(cx, |m| {
        let a1 = &m.model.articles[0];
        assert!(a1.read && !a1.bookmarked, "the toggle beats the older row");
        assert_eq!(
            m.model.article_states.get("1").copied(),
            Some(ArticleState {
                read: true,
                bookmarked: false
            })
        );
        assert!(
            m.model.articles[1].read,
            "the untouched article takes the row"
        );
    });
}

/// M3: the flush's queue rewrite writes the flush-time snapshot, so once a
/// newer save has landed (a briefing during the updater's await) it must
/// leave the file alone; while nothing newer landed it still writes.
#[test]
fn queue_rewrite_never_lands_over_a_newer_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("radar.json");
    let newer = radar_core::store::Snapshot {
        profile: radar_core::types::Profile {
            name: "Newer".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    store::save(&path, &newer).unwrap();
    let queued = json!({"id": "q", "path": "/sync/profile", "method": "PUT"});
    let rewrite = |generation: u64, written: u64| QueueRewrite {
        path: path.clone(),
        snap: radar_core::store::Snapshot::default(),
        mirror: Arc::new(std::sync::Mutex::new(PersistedQueue {
            items: vec![queued.clone()],
            rev: 1,
            epoch: 0,
        })),
        rev: 0,
        generation,
        written_gen: Arc::new(std::sync::Mutex::new(written)),
    };

    rewrite(3, 4).run();
    assert_eq!(
        store::load(&path).profile.name,
        "Newer",
        "a newer save landed"
    );
    assert!(store::load(&path).sync_queue.is_empty());

    rewrite(4, 4).run();
    assert_eq!(store::load(&path).sync_queue, vec![queued.clone()]);
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
