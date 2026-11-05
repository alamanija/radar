//! Helpers shared by the sync-lifecycle and Settings tests: signed-in
//! sessions backed by `MemKeyStore` (never the OS keychain), httpmock
//! servers that answer the sign-in pulls, and a real-time wait for work on
//! the tokio runtime.

use crate::app_model::AppModel;
use chrono::Utc;
use gpui_kit::{Entity, TestAppContext};
use httpmock::prelude::*;
use radar_core::auth::{AuthConfig, Session, Tokens};
use radar_core::keychain::{KeyStore, MemKeyStore, CLERK_OAUTH_USER};
use radar_core::sync::queue::SleepFn;
use radar_core::sync::resource::SyncResource;
use radar_core::sync::resources::PrefsRes;
use radar_core::types::{Account, Prefs};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// Runs GPUI tasks and waits in real time until `done` (tokio work wakes
/// GPUI tasks from its own threads). Panics after ~10 s.
pub fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    cx.executor().allow_parking();
    for _ in 0..1000 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition not reached in time");
}

/// Lets tokio work started by the last step run, then runs GPUI tasks.
pub fn settle(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.run_until_parked();
}

pub fn account(id: &str) -> Account {
    Account {
        id: id.into(),
        email: format!("{id}@example.test"),
        name: String::new(),
        image_url: None,
    }
}

/// A session on `base` (also its token endpoint) holding `access`, or a
/// signed-out one for `None`. The tokens live in the returned store.
pub fn session(base: &str, access: Option<&str>) -> (Session, Arc<MemKeyStore>) {
    let store = Arc::new(MemKeyStore::new());
    if let Some(access) = access {
        let tokens = Tokens {
            access_token: access.into(),
            refresh_token: Some("rt".into()),
            expires_at: Utc::now().timestamp() + 3600,
            id_token: None,
        };
        store
            .set(CLERK_OAUTH_USER, &serde_json::to_string(&tokens).unwrap())
            .unwrap();
    }
    let cfg = AuthConfig {
        frontend_api: base.to_string(),
        client_id: "test".into(),
        redirect_port: 0,
    };
    (Session::load(cfg, store.clone()), store)
}

pub fn instant_sleep() -> SleepFn {
    Arc::new(|_| Box::pin(async {}))
}

/// Signs `model` in as `id` against `base` (token `access`) and starts sync.
pub fn sign_in_as(
    model: &Entity<AppModel>,
    cx: &mut TestAppContext,
    base: &str,
    id: &str,
    access: &str,
) -> Arc<MemKeyStore> {
    let (session, store) = session(base, Some(access));
    model.update(cx, |m, cx| {
        m.session = Some(session);
        m.account = Some(account(id));
        m.server_base = base.to_string();
        m.sync_sleep = Some(instant_sleep());
        m.start_sync(cx);
    });
    store
}

/// `body` plus a server-stamped `updatedAt`.
pub fn stamped(mut body: Value, updated_at: &str) -> Value {
    body["updatedAt"] = Value::from(updated_at);
    body
}

/// A server with no rows: every resource GET is 404, article states are
/// empty, and the prefs seed PUT (prefs are never empty) is echoed.
pub fn serve_empty(server: &MockServer) {
    for path in [
        "/sync/profile",
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
    serve_states(server, json!([]));
    server.mock(|when, then| {
        when.method(PUT).path("/sync/prefs");
        then.status(200)
            .json_body(stamped(PrefsRes::push_body(&Prefs::default()), "p1"));
    });
}

pub fn serve_states(server: &MockServer, states: Value) {
    server.mock(|when, then| {
        when.method(GET).path("/sync/article-states");
        then.status(200).json_body(json!({ "states": states }));
    });
}
