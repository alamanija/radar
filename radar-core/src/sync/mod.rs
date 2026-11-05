//! Cross-device sync against `radar-server`.
//!
//! The app owns the model; everything here reads values passed in and returns
//! merged values. No background tasks: draining the offline queue is an
//! explicit [`SyncClient::drain`] call the app schedules.

pub mod article_states;
pub mod last_synced;
pub mod queue;
pub mod resource;
pub mod resources;

use crate::auth::{AuthError, Session};
use queue::{DrainReport, Queue, QueuedReq, SleepFn};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE, ETAG};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Used when neither the runtime nor the build environment names a server.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8787";

/// The radar-server base URL: runtime `RADAR_SERVER_URL`, else the value
/// baked in at compile time, else [`DEFAULT_SERVER_URL`]. Empty values are
/// ignored.
pub fn server_url() -> String {
    resolve_server_url(
        std::env::var("RADAR_SERVER_URL").ok(),
        option_env!("RADAR_SERVER_URL"),
    )
}

fn resolve_server_url(runtime: Option<String>, compiled: Option<&str>) -> String {
    runtime
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            compiled
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string())
}

/// Sync failures. Messages carry status + path only, never tokens or bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    NotSignedIn,
    Http { status: u16, path: String },
    Network(String),
    Decode(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSignedIn => write!(f, "not signed in"),
            Self::Http { status, path } => write!(f, "{status} on {path}"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::Decode(m) => write!(f, "bad response: {m}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// A server response (or the synthetic 202 for a queued mutation).
#[derive(Debug, Clone, PartialEq)]
pub struct Resp {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Value,
}

impl Resp {
    pub const ACCEPTED: u16 = 202;

    /// Synthetic "accepted" response for a mutation that went into the queue.
    /// Echoes the request body.
    pub fn accepted(body: &Value) -> Self {
        Self {
            status: Self::ACCEPTED,
            etag: None,
            body: if body.is_null() {
                Value::Object(Default::default())
            } else {
                body.clone()
            },
        }
    }

    pub fn is_accepted(&self) -> bool {
        self.status == Self::ACCEPTED
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Observable queue status for the sync indicator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStatus {
    /// Items queued, across all accounts.
    pub pending: usize,
    /// A drain is in progress.
    pub inflight: bool,
    /// Most recent failure (retryable or terminal), e.g. `"400 on /sync/prefs"`.
    pub last_error: Option<String>,
    /// Unix ms of the last successfully drained mutation.
    pub last_success_at: Option<i64>,
}

/// Receives the full queue after every queue mutation.
pub type PersistCallback = Box<dyn Fn(&[QueuedReq]) + Send + Sync>;
type PersistFn = Arc<dyn Fn(&[QueuedReq]) + Send + Sync>;

/// HTTP client for `/sync/*` with the bearer from [`Session`] and the
/// persistent offline queue underneath mutations.
///
/// Enqueueing never waits for a drain: new items land in an inbox (behind a
/// short-lived std mutex) that is persisted together with the queue right
/// away, and the running drain merges the inbox into the queue at the start
/// of each iteration (group-key dedup, newer wins).
pub struct SyncClient {
    base: String,
    session: Session,
    http: reqwest::Client,
    /// Held for a whole drain pass; `try_lock` doubles as the drain
    /// re-entrancy guard.
    queue: Mutex<Queue>,
    shared: std::sync::Mutex<Shared>,
    status: watch::Sender<SyncStatus>,
    persist: PersistFn,
}

/// The persisted picture: the queue as of its last change plus the inbox.
#[derive(Default)]
struct Shared {
    view: Vec<QueuedReq>,
    inbox: Vec<QueuedReq>,
}

impl Shared {
    /// What the queue will hold once the inbox is merged.
    fn combined(&self) -> Vec<QueuedReq> {
        let mut all = self.view.clone();
        for r in &self.inbox {
            queue::push_dedup(&mut all, r.clone(), 0);
        }
        all
    }
}

impl SyncClient {
    pub fn new(base: String, session: Session) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("reqwest client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            session,
            http,
            queue: Mutex::new(Queue::default()),
            shared: std::sync::Mutex::new(Shared::default()),
            status: watch::channel(SyncStatus::default()).0,
            persist: Arc::new(|_| {}),
        }
    }

    /// Called with the full queue (including not-yet-merged inbox items)
    /// after every queue mutation; the app writes it into the store's
    /// `sync-queue` key. It runs under an internal lock, so it must not call
    /// back into this client.
    pub fn with_persist(mut self, persist: PersistCallback) -> Self {
        self.persist = Arc::from(persist);
        self
    }

    /// Restores a persisted queue.
    pub fn with_queue(mut self, items: Vec<QueuedReq>) -> Self {
        let pending = items.len();
        self.shared
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .view = items.clone();
        self.queue.get_mut().items = items;
        self.status.send_modify(|s| s.pending = pending);
        self
    }

    /// Replaces the backoff sleep (tests use an instant one).
    pub fn with_sleep(mut self, sleep: SleepFn) -> Self {
        self.queue.get_mut().sleep = sleep;
        self
    }

    pub fn status(&self) -> watch::Receiver<SyncStatus> {
        self.status.subscribe()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The account queued mutations are tagged with.
    pub fn account_id(&self) -> Option<String> {
        self.session.account().map(|a| a.id)
    }

    /// Snapshot of the queued items (what is persisted). Never waits for a
    /// drain.
    pub fn queued(&self) -> Vec<QueuedReq> {
        self.shared().combined()
    }

    /// Authenticated GET. Any HTTP status comes back as `Ok(Resp)`.
    pub async fn get(&self, path: &str) -> Result<Resp, SyncError> {
        self.send(reqwest::Method::GET, path, &Value::Null, &[])
            .await
    }

    /// Mutating request (PUT/PATCH/DELETE). A retryable status (408/429/5xx)
    /// or a network failure enqueues the request and returns the synthetic
    /// 202; any other status is returned as-is for the caller to handle.
    pub async fn mutate(&self, req: QueuedReq) -> Result<Resp, SyncError> {
        let method = parse_method(&req.method)?;
        match self.send(method, &req.path, &req.body, &req.headers).await {
            Ok(resp) if !queue::is_retryable(resp.status) => return Ok(resp),
            Ok(_) | Err(SyncError::Network(_)) => {}
            Err(e) => return Err(e),
        }
        let accepted = Resp::accepted(&req.body);
        self.enqueue(req);
        Ok(accepted)
    }

    /// Drains this account's queued mutations. Items tagged with another
    /// account are left in place. If a drain is already running this returns
    /// at once with nothing done (`remaining` = items currently queued).
    pub async fn drain(&self) -> DrainReport {
        let Ok(mut queue) = self.queue.try_lock() else {
            return DrainReport {
                remaining: self.queued().len(),
                ..DrainReport::default()
            };
        };
        let account = self.account_id();
        queue
            .drain(self, account.as_deref(), |items| self.persist_queue(items))
            .await
    }

    /// Adds to the inbox and persists immediately; never awaits the drain.
    fn enqueue(&self, mut req: QueuedReq) {
        req.account_id = self.account_id();
        req.tries = 0;
        let mut shared = self.shared();
        queue::push_dedup(&mut shared.inbox, req, 0);
        self.persist_locked(&shared);
    }

    /// Moves the inbox into the persisted view (the caller merges the same
    /// items, in order, into the queue) and returns it.
    pub(crate) fn take_inbox(&self) -> Vec<QueuedReq> {
        let mut shared = self.shared();
        let incoming = std::mem::take(&mut shared.inbox);
        for r in &incoming {
            queue::push_dedup(&mut shared.view, r.clone(), 0);
        }
        incoming
    }

    /// The drain's persist hook: record the queue and persist it with the inbox.
    fn persist_queue(&self, items: &[QueuedReq]) {
        let mut shared = self.shared();
        shared.view = items.to_vec();
        self.persist_locked(&shared);
    }

    fn persist_locked(&self, shared: &Shared) {
        let all = shared.combined();
        (self.persist)(&all);
        self.update_status(|s| s.pending = all.len());
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn update_status(&self, f: impl FnOnce(&mut SyncStatus)) {
        self.status.send_modify(f);
    }

    /// One authenticated request. The token is fetched per call (Session
    /// refreshes it when near expiry); a 401 forces a refresh and retries
    /// once with the new token.
    pub(crate) async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
        headers: &[(String, String)],
    ) -> Result<Resp, SyncError> {
        let token = bearer(self.session.access_token().await)?;
        let resp = self
            .send_once(method.clone(), path, body, headers, &token)
            .await?;
        if resp.status != 401 {
            return Ok(resp);
        }
        let fresh = bearer(self.session.force_refresh().await)?;
        self.send_once(method, path, body, headers, &fresh).await
    }

    async fn send_once(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
        headers: &[(String, String)],
        token: &str,
    ) -> Result<Resp, SyncError> {
        let mut map = HeaderMap::new();
        map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (k, v) in headers {
            if let (Ok(k), Ok(v)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                map.insert(k, v);
            }
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| SyncError::NotSignedIn)?;
        auth.set_sensitive(true);
        map.insert(AUTHORIZATION, auth);

        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .headers(map);
        if !body.is_null() {
            req = req.body(body.to_string());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| SyncError::Network(network_message(&e, path)))?;
        let status = resp.status().as_u16();
        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SyncError::Network(network_message(&e, path)))?;
        let body = if bytes.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => v,
                Err(_) if !(200..300).contains(&status) => Value::Null,
                Err(e) => return Err(SyncError::Decode(format!("{path}: {e}"))),
            }
        };
        Ok(Resp { status, etag, body })
    }
}

fn bearer(token: Result<Option<String>, AuthError>) -> Result<String, SyncError> {
    match token {
        Ok(Some(t)) => Ok(t),
        Ok(None) => Err(SyncError::NotSignedIn),
        Err(e) => Err(SyncError::Network(format!("token refresh: {e}"))),
    }
}

fn parse_method(m: &str) -> Result<reqwest::Method, SyncError> {
    reqwest::Method::from_bytes(m.to_ascii_uppercase().as_bytes())
        .map_err(|_| SyncError::Decode(format!("bad method {m:?}")))
}

/// Error kind + path only: reqwest's Display can include the full URL.
fn network_message(e: &reqwest::Error, path: &str) -> String {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect failed"
    } else if e.is_body() || e.is_decode() {
        "body failed"
    } else {
        "request failed"
    };
    format!("{kind} on {path}")
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::auth::{AuthConfig, Session, Tokens};
    use crate::keychain::{KeyStore, MemKeyStore, CLERK_OAUTH_USER};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::queue::SleepFn;
    use super::SyncClient;

    pub fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    pub fn id_token(sub: &str) -> String {
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let p = URL_SAFE_NO_PAD.encode(json!({"sub": sub, "email": "a@b.co"}).to_string());
        format!("{h}.{p}.c2ln")
    }

    /// A session for `sub` holding `access` that expires `ttl` seconds from now.
    pub fn session(base: &str, sub: &str, access: &str, ttl: i64) -> Session {
        let store = Arc::new(MemKeyStore::new());
        let tokens = Tokens {
            access_token: access.into(),
            refresh_token: Some("rt-old".into()),
            expires_at: now() + ttl,
            id_token: Some(id_token(sub)),
        };
        store
            .set(CLERK_OAUTH_USER, &serde_json::to_string(&tokens).unwrap())
            .unwrap();
        let cfg = AuthConfig {
            frontend_api: base.to_string(),
            client_id: "test".into(),
            redirect_port: 0,
        };
        Session::load(cfg, store)
    }

    pub fn signed_out(base: &str) -> Session {
        let cfg = AuthConfig {
            frontend_api: base.to_string(),
            client_id: "test".into(),
            redirect_port: 0,
        };
        Session::load(cfg, Arc::new(MemKeyStore::new()))
    }

    /// Instant sleep that records every requested duration.
    pub fn recording_sleep() -> (SleepFn, Arc<Mutex<Vec<Duration>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let f: SleepFn = Arc::new(move |d| {
            l.lock().unwrap().push(d);
            Box::pin(async {})
        });
        (f, log)
    }

    /// Client on `base` for `user_1` with a fresh token `at` and instant sleep.
    pub fn client(base: &str) -> SyncClient {
        let (sleep, _) = recording_sleep();
        SyncClient::new(base.to_string(), session(base, "user_1", "at", 3600)).with_sleep(sleep)
    }
}

#[cfg(test)]
mod tests {
    use super::resource::SyncedResource;
    use super::resources::ProfileRes;
    use super::test_support::client;
    use super::*;
    use crate::types::Profile;

    fn assert_send<T: Send>(_: &T) {}

    /// The app spawns these on the shared tokio runtime.
    #[test]
    fn client_and_futures_are_send_sync() {
        fn assert_sync<T: Send + Sync>() {}
        assert_sync::<SyncClient>();
        let c = client("http://127.0.0.1:9/");
        assert_eq!(c.base, "http://127.0.0.1:9");
        assert_send(&c.drain());
        assert_send(&c.get("/sync/profile"));
        assert_send(&c.mutate(QueuedReq::new("PUT", "/p", Value::Null, None)));
        let mut r = SyncedResource::<ProfileRes>::new();
        let p = Profile::default();
        assert_send(&r.push(&c, &p));
        let mut r2 = SyncedResource::<ProfileRes>::new();
        assert_send(&r2.pull(&c, &p));
        assert_send(&article_states::pull(&c));
    }

    #[test]
    fn server_url_prefers_runtime_then_compiled_then_default() {
        let url = |rt: Option<&str>, ct| resolve_server_url(rt.map(str::to_string), ct);
        assert_eq!(
            url(Some("https://rt.test"), Some("https://ct.test")),
            "https://rt.test"
        );
        assert_eq!(url(Some("  "), Some("https://ct.test")), "https://ct.test");
        assert_eq!(url(None, Some("https://ct.test")), "https://ct.test");
        assert_eq!(url(None, Some("")), DEFAULT_SERVER_URL);
        assert_eq!(url(None, None), "http://127.0.0.1:8787");
    }

    #[test]
    fn errors_carry_status_and_path_only() {
        let e = SyncError::Http {
            status: 412,
            path: "/sync/prefs".into(),
        };
        assert_eq!(e.to_string(), "412 on /sync/prefs");
        assert_eq!(Resp::accepted(&Value::Null).body, serde_json::json!({}));
    }
}
