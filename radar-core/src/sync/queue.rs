//! Persistent offline queue for mutating sync requests. Port of
//! `src/syncQueue.js`.
//!
//! Items are stored in the store's `sync-queue` key with the JS key names
//! (`groupKey`, `accountId`, `queuedAt`) so a queue written by the React app
//! drains here. The Authorization header is never stored; it is minted per
//! item at drain time, so a token that expired while the item sat in the
//! queue is refreshed by [`Session`](crate::auth::Session) before the send.

use super::{parse_method, SyncClient, SyncError};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub const MAX_TRIES: u32 = 8;
const MIN_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 60_000;

/// Backoff sleep; injectable so tests don't wait.
pub type SleepFn = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

/// `min(60_000, 500 * 2^min(tries, 6))` ms.
pub fn backoff(tries: u32) -> Duration {
    let ms = MIN_BACKOFF_MS.saturating_mul(1 << tries.min(6));
    Duration::from_millis(ms.min(MAX_BACKOFF_MS))
}

/// 408, 429 and 5xx are transient; everything else is final.
pub fn is_retryable(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn random_id() -> String {
    let n: u128 = rand::random();
    let h = format!("{n:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// One queued mutation, in the JS item shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedReq {
    #[serde(default)]
    pub id: String,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub body: Value,
    #[serde(default, with = "header_map")]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub group_key: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub tries: u32,
    #[serde(default)]
    pub queued_at: i64,
}

impl QueuedReq {
    /// A fresh request (new id, `queued_at` = now, no tries yet).
    pub fn new(method: &str, path: &str, body: Value, group_key: Option<String>) -> Self {
        Self {
            id: random_id(),
            path: path.to_string(),
            method: method.to_string(),
            body,
            headers: Vec::new(),
            group_key,
            account_id: None,
            tries: 0,
            queued_at: now_ms(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// JS stored headers as an object; accept that or `[name, value]` pairs and
/// always write the object form.
mod header_map {
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(h: &[(String, String)], s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(h.len()))?;
        for (k, v) in h {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }

    /// Non-string values are skipped rather than failing the whole item.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Object(m) => m
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
                .collect(),
            Value::Array(pairs) => pairs
                .into_iter()
                .filter_map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([Value::String(k), Value::String(v)]) => Some((k.clone(), v.clone())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
    }
}

/// Removes items sharing `req`'s group key, then appends `req`. Returns how
/// many removed items sat before index `before`.
pub(crate) fn push_dedup(items: &mut Vec<QueuedReq>, req: QueuedReq, before: usize) -> usize {
    let mut removed_before = 0;
    if let Some(key) = &req.group_key {
        let mut i = 0;
        items.retain(|it| {
            let keep = it.group_key.as_ref() != Some(key);
            if !keep && i < before {
                removed_before += 1;
            }
            i += 1;
            keep
        });
    }
    items.push(req);
    removed_before
}

/// What one drain pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrainReport {
    /// `(group_key, server body)` for each drained item that had a group key;
    /// the app routes `PUT:/sync/<x>` bodies to `SyncedResource::on_drained`.
    pub succeeded: Vec<(String, Value)>,
    /// Items removed without landing (non-retryable status or out of tries).
    pub dropped: usize,
    /// Items still queued (all accounts).
    pub remaining: usize,
}

pub struct Queue {
    pub items: Vec<QueuedReq>,
    pub sleep: SleepFn,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue")
            .field("items", &self.items)
            .finish_non_exhaustive()
    }
}

impl Queue {
    pub fn new(items: Vec<QueuedReq>) -> Self {
        Self {
            items,
            sleep: Arc::new(|d| Box::pin(tokio::time::sleep(d))),
        }
    }

    /// Appends `req`, first removing any queued item with the same group key
    /// (the latest full-list PUT supersedes earlier ones).
    pub fn enqueue(&mut self, req: QueuedReq) {
        push_dedup(&mut self.items, req, 0);
    }

    /// Merges inbox items in order (newer wins), keeping `cursor` on the
    /// same item when dedup removes entries before it.
    fn absorb(&mut self, incoming: Vec<QueuedReq>, cursor: &mut usize) {
        for req in incoming {
            *cursor -= push_dedup(&mut self.items, req, *cursor);
        }
    }

    /// Sends this account's items in order. A retryable status backs off and
    /// retries the same item (dropping it after [`MAX_TRIES`]); a final
    /// status removes it. A network failure or a signed-out session stops
    /// the pass and leaves the queue as is, so being offline never uses up
    /// an item's tries.
    pub async fn drain(
        &mut self,
        client: &SyncClient,
        account_id: Option<&str>,
        persist: impl Fn(&[QueuedReq]),
    ) -> DrainReport {
        let mut report = DrainReport::default();
        let _inflight = InflightGuard::start(client);
        let mut cursor = 0;
        loop {
            // Mutations enqueued while we were sending or sleeping.
            self.absorb(client.take_inbox(), &mut cursor);
            if cursor >= self.items.len() {
                break;
            }
            let item = &self.items[cursor];
            if item
                .account_id
                .as_deref()
                .is_some_and(|a| Some(a) != account_id)
            {
                cursor += 1;
                continue;
            }
            let path = item.path.clone();
            let Ok(method) = parse_method(&item.method) else {
                log::warn!(
                    "[sync] dropping queued {:?} {path}: bad method",
                    item.method
                );
                client.update_status(|s| s.last_error = Some(format!("bad method on {path}")));
                self.remove(cursor, &persist);
                report.dropped += 1;
                continue;
            };
            match client.send(method, &path, &item.body, &item.headers).await {
                Ok(resp) if is_retryable(resp.status) => {
                    let msg = format!("{} on {path}", resp.status);
                    client.update_status(|s| s.last_error = Some(msg));
                }
                Ok(resp) if resp.status == 401 => {
                    // Still rejected after a forced refresh: keep the write,
                    // count a try, and stop until auth recovers.
                    client.update_status(|s| s.last_error = Some(format!("401 on {path}")));
                    self.bump_try(cursor, &path, client, &persist, &mut report);
                    break;
                }
                Ok(resp) => {
                    if resp.ok() {
                        client.update_status(|s| {
                            s.last_success_at = Some(now_ms());
                            s.last_error = None;
                        });
                        if let Some(key) = self.items[cursor].group_key.clone() {
                            report.succeeded.push((key, resp.body));
                        }
                    } else {
                        // 412 included: the resource's next pull/push resolves it.
                        log::warn!(
                            "[sync] dropping queued {} {path} ({})",
                            item.method,
                            resp.status
                        );
                        let msg = format!("{} on {path}", resp.status);
                        client.update_status(|s| s.last_error = Some(msg));
                        report.dropped += 1;
                    }
                    self.remove(cursor, &persist);
                    continue;
                }
                Err(SyncError::Decode(_)) => {
                    // A 2xx with an unreadable body still landed.
                    client.update_status(|s| {
                        s.last_success_at = Some(now_ms());
                        s.last_error = None;
                    });
                    self.remove(cursor, &persist);
                    continue;
                }
                Err(SyncError::NotSignedIn) => break,
                Err(e) => {
                    // Offline (or the token endpoint is unreachable): stop and
                    // keep the item's tries for when the app drains again.
                    client.update_status(|s| s.last_error = Some(e.to_string()));
                    break;
                }
            }

            if let Some(tries) = self.bump_try(cursor, &path, client, &persist, &mut report) {
                (self.sleep)(backoff(tries)).await;
            }
        }
        // Includes inbox items that arrived after an early stop.
        report.remaining = client.queued().len();
        report
    }

    /// Counts a failed attempt on `items[at]` and persists. Past
    /// [`MAX_TRIES`] the item is dropped and `None` returned.
    fn bump_try(
        &mut self,
        at: usize,
        path: &str,
        client: &SyncClient,
        persist: &impl Fn(&[QueuedReq]),
        report: &mut DrainReport,
    ) -> Option<u32> {
        let item = &mut self.items[at];
        item.tries += 1;
        if item.tries > MAX_TRIES {
            log::error!(
                "[sync] giving up on {} {path} after {MAX_TRIES} tries",
                item.method
            );
            let msg = format!("gave up on {path} after {MAX_TRIES} tries");
            client.update_status(|s| s.last_error = Some(msg));
            self.remove(at, persist);
            report.dropped += 1;
            return None;
        }
        let tries = item.tries;
        persist(&self.items);
        Some(tries)
    }

    fn remove(&mut self, at: usize, persist: &impl Fn(&[QueuedReq])) {
        self.items.remove(at);
        persist(&self.items);
    }
}

/// Marks the status inflight for the drain's lifetime, even if the drain
/// future is dropped part-way.
struct InflightGuard<'a>(&'a SyncClient);

impl<'a> InflightGuard<'a> {
    fn start(client: &'a SyncClient) -> Self {
        client.update_status(|s| s.inflight = true);
        Self(client)
    }
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.0.update_status(|s| s.inflight = false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::{client, recording_sleep, session};
    use crate::sync::SyncClient;
    use httpmock::prelude::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use tokio::sync::Notify;

    fn item(path: &str, group: &str, account: Option<&str>) -> QueuedReq {
        let mut r = QueuedReq::new("PUT", path, json!({"x": 1}), Some(group.to_string()));
        r.account_id = account.map(str::to_string);
        r
    }

    #[test]
    fn backoff_formula() {
        assert_eq!(backoff(0), Duration::from_millis(500));
        assert_eq!(backoff(1), Duration::from_millis(1000));
        assert_eq!(backoff(6), Duration::from_millis(32_000));
        assert_eq!(backoff(20), Duration::from_millis(32_000));
        assert!(is_retryable(408) && is_retryable(429) && is_retryable(500) && is_retryable(599));
        assert!(
            !is_retryable(400) && !is_retryable(401) && !is_retryable(412) && !is_retryable(200)
        );
    }

    #[tokio::test]
    async fn mutate_503_enqueues_returns_accepted() {
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .header("authorization", "Bearer at");
                then.status(503);
            })
            .await;
        let persisted = Arc::new(Mutex::new(Vec::<Vec<QueuedReq>>::new()));
        let p = persisted.clone();
        let c = client(&server.base_url()).with_persist(Box::new(move |items| {
            p.lock().unwrap().push(items.to_vec())
        }));
        let body = json!({"prefs": {"theme": "dark"}});
        let resp = c
            .mutate(QueuedReq::new(
                "PUT",
                "/sync/prefs",
                body.clone(),
                Some("PUT:/sync/prefs".into()),
            ))
            .await
            .unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(resp.status, 202);
        assert!(resp.is_accepted());
        assert_eq!(resp.body, body);
        let q = c.queued();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].account_id.as_deref(), Some("user_1"));
        assert_eq!(q[0].tries, 0);
        assert_eq!(persisted.lock().unwrap().last().unwrap().len(), 1);
        assert_eq!(c.status().borrow().pending, 1);
    }

    #[tokio::test]
    async fn mutate_network_error_enqueues() {
        // Nothing listens on port 9 (discard) on loopback.
        let c = client("http://127.0.0.1:9");
        let resp = c
            .mutate(QueuedReq::new(
                "PATCH",
                "/sync/article-states",
                json!({}),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status, 202);
        assert_eq!(c.queued().len(), 1);
    }

    #[tokio::test]
    async fn mutate_non_retryable_returned_not_queued() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/prefs");
                then.status(412);
            })
            .await;
        let c = client(&server.base_url());
        let resp = c
            .mutate(QueuedReq::new("PUT", "/sync/prefs", json!({}), None))
            .await
            .unwrap();
        assert_eq!(resp.status, 412);
        assert!(c.queued().is_empty());
    }

    #[tokio::test]
    async fn mutate_signed_out_errors_without_queueing() {
        let server = MockServer::start_async().await;
        let c = SyncClient::new(
            server.base_url(),
            crate::sync::test_support::signed_out(&server.base_url()),
        );
        let err = c
            .mutate(QueuedReq::new("PUT", "/sync/prefs", json!({}), None))
            .await
            .unwrap_err();
        assert_eq!(err, crate::sync::SyncError::NotSignedIn);
        assert!(c.queued().is_empty());
    }

    #[tokio::test]
    async fn mutate_401_retries_once_then_returns() {
        let server = MockServer::start_async().await;
        let token = refresh_mock(&server).await;
        let old = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .header("authorization", "Bearer at");
                then.status(401);
            })
            .await;
        let new = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .header("authorization", "Bearer at-new");
                then.status(401);
            })
            .await;
        let c = client(&server.base_url());
        let resp = c
            .mutate(QueuedReq::new("PUT", "/sync/prefs", json!({}), None))
            .await
            .unwrap();
        // The retry carries the force-refreshed bearer, not the cached one.
        token.assert_calls_async(1).await;
        old.assert_calls_async(1).await;
        new.assert_calls_async(1).await;
        assert_eq!(resp.status, 401);
        assert!(c.queued().is_empty());
    }

    async fn refresh_mock(server: &MockServer) -> httpmock::Mock<'_> {
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("refresh_token", "rt-old");
                then.status(200).json_body(json!({
                    "access_token": "at-new", "expires_in": 3600, "refresh_token": "rt-new"
                }));
            })
            .await
    }

    #[tokio::test]
    async fn drain_401_after_refresh_keeps_item_and_counts_try() {
        let server = MockServer::start_async().await;
        let token = refresh_mock(&server).await;
        let old = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/a")
                    .header("authorization", "Bearer at");
                then.status(401);
            })
            .await;
        let new = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/a")
                    .header("authorization", "Bearer at-new");
                then.status(401);
            })
            .await;
        let b = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/b");
                then.status(200);
            })
            .await;
        let c = client(&server.base_url()).with_queue(vec![
            item("/sync/a", "a", Some("user_1")),
            item("/sync/b", "b", Some("user_1")),
        ]);
        let report = c.drain().await;
        token.assert_calls_async(1).await;
        old.assert_calls_async(1).await;
        new.assert_calls_async(1).await;
        // The pass stops: the next item isn't sent into a broken session.
        b.assert_calls_async(0).await;
        assert_eq!(report.dropped, 0);
        assert_eq!(report.remaining, 2);
        let q = c.queued();
        assert_eq!(q[0].path, "/sync/a");
        assert_eq!(q[0].tries, 1);
        assert_eq!(q[1].tries, 0);
        let st = c.status().borrow().clone();
        assert_eq!(st.last_error.as_deref(), Some("401 on /sync/a"));
        assert!(!st.inflight);
    }

    /// A sleep whose first call parks until `release` fires (after signalling
    /// `entered`); later calls return at once.
    fn gated_sleep() -> (SleepFn, Arc<Notify>, Arc<Notify>) {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first = Arc::new(AtomicBool::new(true));
        let (e, r) = (entered.clone(), release.clone());
        let f: SleepFn = Arc::new(move |_| {
            let (e, r, first) = (e.clone(), r.clone(), first.clone());
            Box::pin(async move {
                if first.swap(false, Ordering::SeqCst) {
                    e.notify_one();
                    r.notified().await;
                }
            })
        });
        (f, entered, release)
    }

    #[tokio::test]
    async fn mutate_does_not_block_during_drain() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/a");
                then.status(503);
            })
            .await;
        let b = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/b");
                then.status(503);
            })
            .await;
        let (sleep, entered, release) = gated_sleep();
        let persisted = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let p = persisted.clone();
        let c = Arc::new(
            client(&server.base_url())
                .with_sleep(sleep)
                .with_queue(vec![item("/sync/a", "a", Some("user_1"))])
                .with_persist(Box::new(move |items| {
                    p.lock()
                        .unwrap()
                        .push(items.iter().map(|i| i.path.clone()).collect())
                })),
        );
        let c2 = c.clone();
        let drain = tokio::spawn(async move { c2.drain().await });
        entered.notified().await; // the drain is parked in its backoff sleep

        let resp = tokio::time::timeout(
            Duration::from_secs(5),
            c.mutate(QueuedReq::new(
                "PUT",
                "/sync/b",
                json!({"b": 1}),
                Some("b".into()),
            )),
        )
        .await
        .expect("mutate must not wait for the drain")
        .unwrap();
        assert_eq!(resp.status, 202);
        // Persisted at once, together with the queue the drain holds.
        assert_eq!(
            persisted.lock().unwrap().last().unwrap(),
            &vec!["/sync/a".to_string(), "/sync/b".to_string()]
        );
        assert_eq!(c.queued().len(), 2);
        assert_eq!(c.status().borrow().pending, 2);

        release.notify_one();
        let report = drain.await.unwrap();
        // The running drain picked the inbox item up: 1 direct send + 9 drained.
        b.assert_calls_async(10).await;
        assert_eq!(report.dropped, 2);
        assert!(c.queued().is_empty());
        assert_eq!(c.status().borrow().pending, 0);
    }

    #[tokio::test]
    async fn concurrent_drain_returns_early() {
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/a");
                then.status(503);
            })
            .await;
        let (sleep, entered, release) = gated_sleep();
        let c = Arc::new(
            client(&server.base_url())
                .with_sleep(sleep)
                .with_queue(vec![item("/sync/a", "a", Some("user_1"))]),
        );
        let c2 = c.clone();
        let first = tokio::spawn(async move { c2.drain().await });
        entered.notified().await;

        let second = tokio::time::timeout(Duration::from_secs(5), c.drain())
            .await
            .expect("second drain must not wait");
        assert!(second.succeeded.is_empty());
        assert_eq!(second.dropped, 0);
        assert_eq!(second.remaining, 1);
        put.assert_calls_async(1).await;

        release.notify_one();
        let report = first.await.unwrap();
        put.assert_calls_async(9).await;
        assert_eq!(report.dropped, 1);
    }

    #[tokio::test]
    async fn inbox_dedups_against_queue_newer_wins() {
        let server = MockServer::start_async().await;
        let down = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/prefs");
                then.status(503);
            })
            .await;
        let persisted = Arc::new(Mutex::new(Vec::<Vec<Value>>::new()));
        let p = persisted.clone();
        let mut old = item("/sync/prefs", "PUT:/sync/prefs", Some("user_1"));
        old.body = json!({"v": "old"});
        let c = client(&server.base_url())
            .with_queue(vec![old])
            .with_persist(Box::new(move |items| {
                p.lock()
                    .unwrap()
                    .push(items.iter().map(|i| i.body.clone()).collect())
            }));
        let resp = c
            .mutate(QueuedReq::new(
                "PUT",
                "/sync/prefs",
                json!({"v": "new"}),
                Some("PUT:/sync/prefs".into()),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status, 202);
        let q = c.queued();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].body, json!({"v": "new"}));
        assert_eq!(
            persisted.lock().unwrap().last().unwrap(),
            &vec![json!({"v": "new"})]
        );

        down.delete_async().await;
        let new = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .json_body(json!({"v": "new"}));
                then.status(200).json_body(json!({"updatedAt": "t"}));
            })
            .await;
        let stale = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .json_body(json!({"v": "old"}));
                then.status(200);
            })
            .await;
        let report = c.drain().await;
        new.assert_calls_async(1).await;
        stale.assert_calls_async(0).await;
        assert_eq!(
            report.succeeded,
            vec![("PUT:/sync/prefs".to_string(), json!({"updatedAt": "t"}))]
        );
        assert!(c.queued().is_empty());
    }

    #[test]
    fn absorb_keeps_cursor_on_current_item() {
        let mut q = Queue::new(vec![
            item("/sync/x", "g1", Some("user_2")),
            item("/sync/y", "g2", Some("user_1")),
        ]);
        let mut cursor = 1;
        q.absorb(vec![item("/sync/x2", "g1", Some("user_2"))], &mut cursor);
        assert_eq!(cursor, 0);
        assert_eq!(q.items[cursor].path, "/sync/y");
        assert_eq!(q.items[1].path, "/sync/x2");
    }

    #[test]
    fn queue_dedups_by_group_key() {
        let mut q = Queue::default();
        q.enqueue(item("/sync/prefs", "PUT:/sync/prefs", None));
        q.enqueue(item("/sync/sources", "PUT:/sync/sources", None));
        let mut newer = item("/sync/prefs", "PUT:/sync/prefs", None);
        newer.body = json!({"x": 2});
        q.enqueue(newer);
        let mut ungrouped = QueuedReq::new("PATCH", "/a", json!({}), None);
        ungrouped.id = "u1".into();
        q.enqueue(ungrouped.clone());
        ungrouped.id = "u2".into();
        q.enqueue(ungrouped);
        let keys: Vec<_> = q.items.iter().map(|i| i.group_key.clone()).collect();
        assert_eq!(
            keys,
            vec![
                Some("PUT:/sync/sources".into()),
                Some("PUT:/sync/prefs".into()),
                None,
                None
            ]
        );
        assert_eq!(q.items[1].body, json!({"x": 2}));
    }

    #[tokio::test]
    async fn queue_skips_other_account_items() {
        let server = MockServer::start_async().await;
        let mine = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/mine");
                then.status(200).json_body(json!({"updatedAt": "t1"}));
            })
            .await;
        let other = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/other");
                then.status(200);
            })
            .await;
        let untagged = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/untagged");
                then.status(200);
            })
            .await;
        let c = client(&server.base_url()).with_queue(vec![
            item("/sync/other", "g-other", Some("user_2")),
            item("/sync/mine", "g-mine", Some("user_1")),
            item("/sync/untagged", "g-untagged", None),
        ]);
        let report = c.drain().await;
        other.assert_calls_async(0).await;
        mine.assert_calls_async(1).await;
        untagged.assert_calls_async(1).await;
        assert_eq!(report.remaining, 1);
        assert_eq!(report.dropped, 0);
        assert_eq!(
            report.succeeded[0],
            ("g-mine".to_string(), json!({"updatedAt": "t1"}))
        );
        let left = c.queued();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].account_id.as_deref(), Some("user_2"));
        let st = c.status().borrow().clone();
        assert_eq!(st.pending, 1);
        assert!(!st.inflight);
        assert!(st.last_success_at.is_some());
    }

    #[tokio::test]
    async fn queue_drops_non_retryable_4xx() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/conflict");
                then.status(412);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/bad");
                then.status(400);
            })
            .await;
        let persisted = Arc::new(Mutex::new(Vec::<usize>::new()));
        let p = persisted.clone();
        let c = client(&server.base_url())
            .with_queue(vec![
                item("/sync/conflict", "a", Some("user_1")),
                item("/sync/bad", "b", Some("user_1")),
            ])
            .with_persist(Box::new(move |items| p.lock().unwrap().push(items.len())));
        let report = c.drain().await;
        assert_eq!(report.dropped, 2);
        assert_eq!(report.remaining, 0);
        assert!(report.succeeded.is_empty());
        assert!(c.queued().is_empty());
        assert_eq!(*persisted.lock().unwrap(), vec![1, 0]);
        let st = c.status().borrow().clone();
        assert_eq!(st.last_error.as_deref(), Some("400 on /sync/bad"));
        assert_eq!(st.pending, 0);
        assert!(st.last_success_at.is_none());
    }

    #[tokio::test]
    async fn queue_gives_up_after_8_tries() {
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/prefs");
                then.status(503);
            })
            .await;
        let (sleep, slept) = recording_sleep();
        let c = client(&server.base_url())
            .with_sleep(sleep)
            .with_queue(vec![item("/sync/prefs", "p", Some("user_1"))]);
        let report = c.drain().await;
        // JS semantics: `tries > MAX_TRIES` gives up, so 1 + 8 retries.
        put.assert_calls_async(9).await;
        let ms: Vec<u64> = slept
            .lock()
            .unwrap()
            .iter()
            .map(|d| d.as_millis() as u64)
            .collect();
        assert_eq!(ms, vec![1000, 2000, 4000, 8000, 16000, 32000, 32000, 32000]);
        assert_eq!(report.dropped, 1);
        assert_eq!(report.remaining, 0);
        assert_eq!(
            c.status().borrow().last_error.as_deref(),
            Some("gave up on /sync/prefs after 8 tries")
        );
    }

    #[tokio::test]
    async fn retryable_then_success_keeps_tries_and_lands() {
        // A 503 bumps tries and persists before the backoff sleep.
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/prefs");
                then.status(503);
            })
            .await;
        let (sleep, _) = recording_sleep();
        let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
        let s = seen.clone();
        let c = client(&server.base_url())
            .with_sleep(sleep)
            .with_queue(vec![item("/sync/prefs", "p", Some("user_1"))])
            .with_persist(Box::new(move |items| {
                if let Some(i) = items.first() {
                    s.lock().unwrap().push(i.tries);
                }
            }));
        c.drain().await;
        put.assert_calls_async(9).await;
        assert_eq!(*seen.lock().unwrap(), (1..=8).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn network_error_stops_drain_without_using_tries() {
        let (sleep, slept) = recording_sleep();
        let c = SyncClient::new(
            "http://127.0.0.1:9".into(),
            session("http://127.0.0.1:9", "user_1", "at", 3600),
        )
        .with_sleep(sleep)
        .with_queue(vec![
            item("/sync/a", "a", Some("user_1")),
            item("/sync/b", "b", Some("user_1")),
        ]);
        let report = c.drain().await;
        assert_eq!(report.remaining, 2);
        assert_eq!(report.dropped, 0);
        assert!(slept.lock().unwrap().is_empty());
        assert!(c.queued().iter().all(|i| i.tries == 0));
        assert!(!c.status().borrow().inflight);
    }

    #[tokio::test]
    async fn drain_refreshes_expired_token() {
        let server = MockServer::start_async().await;
        let token = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("refresh_token", "rt-old");
                then.status(200).json_body(json!({
                    "access_token": "at-fresh", "expires_in": 3600, "refresh_token": "rt-new"
                }));
            })
            .await;
        let stale = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .header("authorization", "Bearer at-expired");
                then.status(401);
            })
            .await;
        let fresh = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/prefs")
                    .header("authorization", "Bearer at-fresh")
                    .header("if-match", "W/\"t0\"")
                    .json_body(json!({"x": 1}));
                then.status(200)
                    .json_body(json!({"prefs": {}, "updatedAt": "t1"}));
            })
            .await;
        let sess = session(&server.base_url(), "user_1", "at-expired", -120);
        let (sleep, _) = recording_sleep();
        let c = SyncClient::new(server.base_url(), sess)
            .with_sleep(sleep)
            .with_queue(vec![item("/sync/prefs", "PUT:/sync/prefs", Some("user_1"))
                .header("If-Match", "W/\"t0\"")]);
        let report = c.drain().await;
        token.assert_calls_async(1).await;
        stale.assert_calls_async(0).await;
        fresh.assert_calls_async(1).await;
        assert_eq!(report.dropped, 0);
        assert_eq!(report.remaining, 0);
        assert_eq!(report.succeeded.len(), 1);
        assert_eq!(report.succeeded[0].0, "PUT:/sync/prefs");
    }

    #[test]
    fn legacy_js_queue_item_deserializes() {
        // Shape written by src/syncQueue.js (see tests/fixtures/tauri_radar.json).
        let raw = json!({
            "id": "6f1c5d2e-1b7a-4a39-9d64-2f0c8e1a7b55",
            "path": "/sync/article-states",
            "method": "PATCH",
            "body": {"articleId": "1001", "read": true, "bookmarked": false},
            "headers": {"If-Match": "W/\"2026-01-01T00:00:00Z\""},
            "groupKey": "PATCH:/sync/article-states:1001",
            "accountId": "user_abc",
            "tries": 2,
            "queuedAt": 1767261005000i64
        });
        let r: QueuedReq = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(r.id, "6f1c5d2e-1b7a-4a39-9d64-2f0c8e1a7b55");
        assert_eq!(
            r.headers,
            vec![(
                "If-Match".to_string(),
                "W/\"2026-01-01T00:00:00Z\"".to_string()
            )]
        );
        assert_eq!(
            r.group_key.as_deref(),
            Some("PATCH:/sync/article-states:1001")
        );
        assert_eq!(r.account_id.as_deref(), Some("user_abc"));
        assert_eq!(r.tries, 2);
        assert_eq!(r.queued_at, 1767261005000);
        assert_eq!(serde_json::to_value(&r).unwrap(), raw);

        // Missing optional keys (JS `groupKey: undefined`, `accountId: null`).
        let min: QueuedReq = serde_json::from_value(json!({
            "id": "x", "path": "/sync/prefs", "method": "PUT", "body": {},
            "headers": {}, "accountId": null, "tries": 0, "queuedAt": 1
        }))
        .unwrap();
        assert_eq!(min.group_key, None);
        assert_eq!(min.account_id, None);
        assert!(min.headers.is_empty());

        // Pair-list headers are accepted too.
        let pairs: QueuedReq = serde_json::from_value(json!({
            "path": "/p", "method": "PUT", "headers": [["A", "1"]]
        }))
        .unwrap();
        assert_eq!(pairs.headers, vec![("A".to_string(), "1".to_string())]);

        // The store fixture's queue parses as a whole.
        let fixture = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tauri_radar.json"),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&fixture).unwrap();
        let q: Vec<QueuedReq> = serde_json::from_value(v["sync-queue"].clone()).unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q[1].tries, 2);
    }
}
