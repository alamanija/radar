//! Helpers shared by the app tests: signed-in sessions backed by
//! `MemKeyStore` (never the OS keychain), httpmock servers that answer the
//! sign-in pulls, a real-time wait for work on the tokio runtime, and
//! [`Fakes`], recording stand-ins for the model's OS and pipeline hooks.

use crate::app_model::{AppModel, Hooks};
use crate::platform::{autostart, tray, updater};
use chrono::{DateTime, Local, TimeZone, Utc};
use gpui_kit::{Entity, TestAppContext};
use httpmock::prelude::*;
use radar_core::auth::{AuthConfig, Session, Tokens};
use radar_core::keychain::{KeyStore, MemKeyStore, CLERK_OAUTH_USER};
use radar_core::model::BriefingOutcome;
use radar_core::sync::queue::SleepFn;
use radar_core::sync::resource::SyncResource;
use radar_core::sync::resources::PrefsRes;
use radar_core::types::{Account, Article, BriefingResponse, Prefs};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// An unsigned id_token naming `sub` (the session reads the account from it).
pub fn id_token(sub: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let head = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD
        .encode(json!({"sub": sub, "email": format!("{sub}@example.test")}).to_string());
    format!("{head}.{body}.c2ln")
}

/// A session on `base` (also its token endpoint) holding `access`, or a
/// signed-out one for `None`. The tokens live in the returned store.
pub fn session(base: &str, access: Option<&str>) -> (Session, Arc<MemKeyStore>) {
    session_with(base, access, None)
}

/// [`session`] whose tokens carry an id_token for `sub` (queued writes are
/// then tagged with that account).
pub fn session_with(
    base: &str,
    access: Option<&str>,
    sub: Option<&str>,
) -> (Session, Arc<MemKeyStore>) {
    let store = Arc::new(MemKeyStore::new());
    if let Some(access) = access {
        let tokens = Tokens {
            access_token: access.into(),
            refresh_token: Some("rt".into()),
            expires_at: Utc::now().timestamp() + 3600,
            id_token: sub.map(id_token),
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

/// `hh:mm` local time on a fixed day without a DST change.
pub fn at(h: u32, m: u32) -> DateTime<Local> {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
    Local
        .from_local_datetime(&day.and_hms_opt(h, m, 0).unwrap())
        .single()
        .unwrap()
}

/// One OS wake sync: `(enabled, hour, minute)`.
pub type WakeCall = (bool, Option<u32>, Option<u32>);

/// Recording stand-ins for every [`Hooks`] integration. Nothing reaches the
/// OS or the network: the briefing runner answers a fixed response, the
/// clock is fixed, and notifications, tray pushes and wake syncs are only
/// recorded.
#[derive(Clone, Default)]
pub struct Fakes {
    /// Briefing runner calls.
    pub runs: Arc<AtomicUsize>,
    pub notified: Rc<RefCell<Vec<BriefingOutcome>>>,
    /// `(title, tooltip)` pushes to the tray.
    pub tray: Rc<RefCell<Vec<tray::Status>>>,
    pub wakes: Arc<Mutex<Vec<WakeCall>>>,
}

impl Fakes {
    /// Hooks whose briefing answers `articles` and whose clock reads `now`.
    pub fn hooks(&self, articles: Vec<Article>, now: DateTime<Local>) -> Hooks {
        let runs = self.runs.clone();
        let resp = BriefingResponse {
            articles,
            errors: Vec::new(),
            fetched_at: now.with_timezone(&Utc),
        };
        let notified = self.notified.clone();
        let tray = self.tray.clone();
        let wakes = self.wakes.clone();
        Hooks {
            briefing: Arc::new(move |_| {
                runs.fetch_add(1, Ordering::SeqCst);
                let resp = resp.clone();
                Box::pin(async move { Ok(resp) })
            }),
            clock: Rc::new(move || now),
            notify: Rc::new(move |_, out| notified.borrow_mut().push(out)),
            tray: Rc::new(move |title, tooltip| tray.borrow_mut().push((title, tooltip))),
            wake: Some(Arc::new(move |enabled, hour, minute| {
                wakes.lock().unwrap().push((enabled, hour, minute));
                Ok(())
            })),
            autostart: autostart::Backend::off(),
            updater: Arc::new(updater::Inert),
            quit: Rc::new(|_| {}),
        }
    }

    pub fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }

    pub fn wakes(&self) -> Vec<WakeCall> {
        self.wakes.lock().unwrap().clone()
    }

    /// The newest tray push.
    pub fn tray_now(&self) -> Option<tray::Status> {
        self.tray.borrow().last().cloned()
    }
}

/// A model on a fresh store in `dir`, with `fakes` hooked in and one
/// source the briefing can fetch. No window is open.
pub fn faked_model(
    cx: &mut TestAppContext,
    dir: &tempfile::TempDir,
    fakes: &Fakes,
    articles: Vec<Article>,
    now: DateTime<Local>,
) -> Entity<AppModel> {
    // The briefing and wake hooks resolve on tokio threads.
    cx.executor().allow_parking();
    let path = dir.path().join("radar.json");
    let model = cx.update(|cx| AppModel::init(path, None, cx));
    model.update(cx, |m, cx| {
        m.set_hooks(fakes.hooks(articles, now), cx);
        m.update_model(cx, |r| {
            r.add_source(
                "Feed".into(),
                "tools".into(),
                Some("https://feed.test/rss".into()),
            )
        });
    });
    model
}
