//! Full-list synced resource state machine. Port of
//! `src/hooks/useSyncedResource.js`.
//!
//! The app owns the value and the 800 ms debounce: it calls [`SyncedResource::pull`]
//! on sign-in, and when [`SyncedResource::needs_push`] is true after an edit
//! settles, [`SyncedResource::push`]. Any merged value returned must be
//! written back into the model.
//!
//! App obligations:
//! - A failed `pull` leaves `pulled` false, so nothing is pushed for that
//!   resource until a pull succeeds. The app must retry failed pulls (on a
//!   timer or when the network returns). Edits made in the meantime are kept:
//!   the successful pull pushes them instead of taking the server's copy.
//! - Call `reset` on sign-out / account switch, then `pull` for the new
//!   account.
//! - Persist [`SyncedResource::last_synced_digest`] per account, together
//!   with the digest of the local value the same store save wrote (see
//!   `super::last_synced`). Before a session's first pull, seed it back with
//!   [`SyncedResource::seed_last_synced`], but only after checking that the
//!   store loaded cleanly and the loaded value is the one saved with it
//!   (`LastSynced::checked`): any other local value (a damaged or replaced
//!   store) would be pushed over the server's. An edit an earlier run never
//!   got to the server is then checked against the server by the first
//!   pull: pushed if the server still holds the last-synced value, adopted
//!   without a push if the server already holds it, and replaced by the
//!   server's copy if the server changed to anything else since.
//! - Route `DrainReport::succeeded` bodies whose key equals
//!   [`SyncedResource::group_key`] to [`SyncedResource::on_drained`].

use super::queue::QueuedReq;
use super::{Resp, SyncClient, SyncError};
use serde_json::Value;
use std::marker::PhantomData;

/// How one `/sync/<x>` resource maps between the local value and the wire.
pub trait SyncResource {
    const ENDPOINT: &'static str;
    type Local: Clone;

    /// PUT request body.
    fn push_body(local: &Self::Local) -> Value;
    /// Local value built from a server snapshot (GET or PUT response).
    fn from_remote(remote: &Value, prev: &Self::Local) -> Self::Local;
    /// Comparison key for no-op detection.
    fn stable_key(local: &Self::Local) -> String;
    /// Empty locals don't seed an empty server.
    fn is_empty(local: &Self::Local) -> bool;
    /// The value to push when the local value was edited while pulls were
    /// failing. Local wins by default; archives union-merge. An override that
    /// unions must also set [`OFFLINE_MERGE_UNIONS`](Self::OFFLINE_MERGE_UNIONS).
    fn merge_offline(_remote: &Value, local: &Self::Local) -> Self::Local {
        local.clone()
    }
    /// True when [`merge_offline`](Self::merge_offline) unions the server's
    /// value into the local one (archives), so pushing it can't lose another
    /// device's write. False (the default) when local replaces the server's
    /// value; a baseline seeded from an earlier run then pushes only if the
    /// server still holds the last-synced value (see [`SyncedResource::pull`]).
    const OFFLINE_MERGE_UNIONS: bool = false;
}

/// Sync bookkeeping for one resource.
pub struct SyncedResource<R: SyncResource> {
    /// The first pull for this account finished; pushes are gated on it.
    pub pulled: bool,
    /// Stable key of the last value known to be on the server.
    pub server_key: Option<String>,
    /// Last server `updatedAt`, sent back as `If-Match: W/"<etag>"`.
    pub etag: Option<String>,
    /// `stable_key(local)` at the first failed pull since the last success:
    /// the baseline for detecting edits made while the server was unreachable.
    pub failed_pull_key: Option<String>,
    /// The persisted last-synced digest a seeded divergence came from
    /// ([`seed_last_synced`](Self::seed_last_synced)): `Some` only while
    /// `failed_pull_key` is that seeded sentinel. Kept by a failed pull,
    /// cleared by a successful one.
    pub seed_digest: Option<String>,
    _r: PhantomData<fn() -> R>,
}

impl<R: SyncResource> Default for SyncedResource<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: SyncResource> std::fmt::Debug for SyncedResource<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncedResource")
            .field("endpoint", &R::ENDPOINT)
            .field("pulled", &self.pulled)
            .field("server_key", &self.server_key)
            .field("etag", &self.etag)
            .field("failed_pull_key", &self.failed_pull_key)
            .field("seed_digest", &self.seed_digest)
            .finish()
    }
}

impl<R: SyncResource> SyncedResource<R> {
    pub fn new() -> Self {
        Self {
            pulled: false,
            server_key: None,
            etag: None,
            failed_pull_key: None,
            seed_digest: None,
            _r: PhantomData,
        }
    }

    /// Queue group key for this resource's PUTs (`PUT:/sync/<x>`).
    pub fn group_key() -> String {
        format!("PUT:{}", R::ENDPOINT)
    }

    /// Forget everything (account switch / sign-out).
    pub fn reset(&mut self) {
        self.pulled = false;
        self.server_key = None;
        self.etag = None;
        self.failed_pull_key = None;
        self.seed_digest = None;
    }

    /// Pull on sign-in. Returns the value to store locally when the server
    /// has one. A 404 with a non-empty local seeds the server (no If-Match).
    ///
    /// On error `pulled` stays false so pushes remain gated until a pull
    /// succeeds (the JS set `pulled` even on failure, which let a later push
    /// without If-Match overwrite server data it never saw), and the local
    /// key is recorded. If a later pull finds the local value changed since
    /// that first failure, the offline edit wins: it is merged with
    /// [`SyncResource::merge_offline`] and pushed with the fresh etag, and the
    /// pushed value is returned.
    ///
    /// A divergence [seeded](Self::seed_last_synced) from an earlier run is
    /// first checked against the server, unless the resource's offline merge
    /// is a union ([`SyncResource::OFFLINE_MERGE_UNIONS`]):
    /// - the server still holds the last-synced value: the edit never landed,
    ///   so it is pushed as above;
    /// - the server already holds the local value (the earlier run's push
    ///   landed, but it never recorded that): it is adopted, with no PUT;
    /// - the server holds anything else: another device wrote since, and
    ///   this device can't tell that from a conflict with its own stuck
    ///   edit, so the server's copy wins, as for a fresh pull.
    pub async fn pull(
        &mut self,
        client: &SyncClient,
        local: &R::Local,
    ) -> Result<Option<R::Local>, SyncError> {
        self.pulled = false;
        self.server_key = None;
        self.etag = None;
        let baseline = self.failed_pull_key.take();
        let diverged = baseline
            .as_deref()
            .is_some_and(|k| k != R::stable_key(local));
        match self.pull_inner(client, local, diverged).await {
            Ok(out) => {
                self.pulled = true;
                self.seed_digest = None;
                Ok(out)
            }
            Err(e) => {
                self.failed_pull_key = baseline.or_else(|| Some(R::stable_key(local)));
                Err(e)
            }
        }
    }

    async fn pull_inner(
        &mut self,
        client: &SyncClient,
        local: &R::Local,
        diverged: bool,
    ) -> Result<Option<R::Local>, SyncError> {
        let out = match fetch::<R>(client).await? {
            Some(remote) if diverged => {
                self.etag = updated_at(&remote);
                // A baseline seeded from an earlier run, for a resource whose
                // offline edit replaces the server's value: push only if the
                // server is unchanged since that run last synced. This narrows
                // the offline-edit rule for cross-restart baselines only (an
                // in-session failed-pull baseline still pushes), matching a
                // queued PUT that meets a 412 being dropped.
                let seed = self
                    .seed_digest
                    .clone()
                    .filter(|_| !R::OFFLINE_MERGE_UNIONS);
                if let Some(seed) = seed {
                    let next = R::from_remote(&remote, local);
                    let remote_key = R::stable_key(&next);
                    if key_digest(&remote_key) != seed {
                        // Already local (the earlier push landed): adopt it.
                        // Anything else: the server wins.
                        if remote_key != R::stable_key(local) {
                            log::warn!(
                                "[sync] {} changed on the server since the last sync; \
                                 keeping the server's copy over an unsynced local edit",
                                R::ENDPOINT
                            );
                        }
                        self.server_key = Some(remote_key);
                        return Ok(Some(next));
                    }
                }
                let mine = R::merge_offline(&remote, local);
                // A 412 here (a write landed after our GET) falls back to
                // push's refetch + from_remote merge.
                let pushed = self.push(client, &mine).await?;
                Some(pushed.unwrap_or(mine))
            }
            Some(remote) => {
                self.etag = updated_at(&remote);
                let next = R::from_remote(&remote, local);
                self.server_key = Some(R::stable_key(&next));
                Some(next)
            }
            None if !R::is_empty(local) => {
                match put::<R>(client, R::push_body(local), None).await? {
                    Pushed::Snapshot(snap) => self.accept_snapshot(&snap, local),
                    // Queued, or someone created the row in between: treat
                    // the local value as pushed; a drain or the next pull
                    // catches the etag up.
                    Pushed::Queued | Pushed::Conflict => {
                        self.server_key = Some(R::stable_key(local));
                    }
                }
                None
            }
            None => {
                self.server_key = Some(R::stable_key(local));
                None
            }
        };
        Ok(out)
    }

    /// True once pulled and the local value differs from the server's.
    pub fn needs_push(&self, local: &R::Local) -> bool {
        self.pulled && self.server_key.as_deref() != Some(R::stable_key(local).as_str())
    }

    /// PUT the local value with If-Match. On 412: refetch, merge with
    /// `from_remote`, and re-push the merge once; the merged value is
    /// returned as `Some` and must replace the local value.
    pub async fn push(
        &mut self,
        client: &SyncClient,
        local: &R::Local,
    ) -> Result<Option<R::Local>, SyncError> {
        match put::<R>(client, R::push_body(local), self.etag.as_deref()).await? {
            Pushed::Snapshot(snap) => {
                self.accept_snapshot(&snap, local);
                return Ok(None);
            }
            Pushed::Queued => {
                // Optimistic, so the same value isn't pushed again.
                self.server_key = Some(R::stable_key(local));
                return Ok(None);
            }
            Pushed::Conflict => {}
        }
        log::warn!("[sync] conflict on {} push; refetching", R::ENDPOINT);
        let Some(remote) = fetch::<R>(client).await? else {
            return Ok(None);
        };
        self.etag = updated_at(&remote);
        let next = R::from_remote(&remote, local);
        // Re-push the merge with the fresh etag: a no-op for server-wins
        // resources, and how archives' local-only entries reach the server.
        // Single retry; a second 412 means something is thrashing.
        match put::<R>(client, R::push_body(&next), self.etag.as_deref()).await {
            Ok(Pushed::Snapshot(snap)) => self.accept_snapshot(&snap, &next),
            Ok(Pushed::Conflict) => {
                log::warn!("[sync] {} retry after merge also conflicted", R::ENDPOINT);
                self.server_key = Some(R::stable_key(&next));
            }
            Ok(Pushed::Queued) => self.server_key = Some(R::stable_key(&next)),
            // The merge must still reach the model; server_key is unchanged
            // so the next push retries.
            Err(e) => log::warn!("[sync] {} retry after merge failed: {e}", R::ENDPOINT),
        }
        Ok(Some(next))
    }

    /// What the app persists so the next run can [seed](Self::seed_last_synced)
    /// the divergence baseline: the [`key_digest`] of the value last known to
    /// be on the server or, after a failed pull, of the baseline that pull
    /// kept. A seeded baseline that hasn't pulled yet keeps the digest it was
    /// seeded with ([`seed_digest`](Self::seed_digest)), so the next run can
    /// still recognize the server's last-synced value. `None` until any of
    /// them is known.
    pub fn last_synced_digest(&self) -> Option<String> {
        if self.server_key.is_none() && self.failed_pull_key.as_deref() == Some(DIVERGED) {
            if let Some(seed) = &self.seed_digest {
                return Some(seed.clone());
            }
        }
        self.server_key
            .as_deref()
            .or(self.failed_pull_key.as_deref())
            .map(key_digest)
    }

    /// Seeds the divergence baseline (`failed_pull_key`) with a digest an
    /// earlier run persisted from [`last_synced_digest`](Self::last_synced_digest).
    /// Call it before the first pull of a session. If `local` has changed
    /// since that run last synced (an edit that may never have reached the
    /// server: quit, crash, power loss), the pull takes the offline-edit path,
    /// checked against the server as described on [`pull`](Self::pull). A
    /// baseline this run already has is kept.
    pub fn seed_last_synced(&mut self, digest: &str, local: &R::Local) {
        if self.failed_pull_key.is_some() {
            return;
        }
        let key = R::stable_key(local);
        if key_digest(&key) == digest {
            self.failed_pull_key = Some(key);
        } else {
            // The pre-edit key itself isn't persisted, only its digest.
            self.failed_pull_key = Some(DIVERGED.to_string());
            self.seed_digest = Some(digest.to_string());
        }
    }

    /// A queued PUT for this resource landed (from `DrainReport::succeeded`).
    ///
    /// A pending [seed](Self::seed_digest) moves to the value that landed.
    /// The seed stands for the last value this device knows is on the
    /// server, and a drained body is exactly that: this device's own write.
    /// Left at the older digest, the next pull would take this write for
    /// another device's and drop an edit made since.
    pub fn on_drained(&mut self, body: &Value, local: &R::Local) {
        if body.is_null() {
            return;
        }
        if let Some(t) = updated_at(body) {
            self.etag = Some(t);
        }
        self.server_key = Some(R::stable_key(&R::from_remote(body, local)));
        if self.seed_digest.is_some() {
            self.seed_digest = self.server_key.as_deref().map(key_digest);
        }
    }

    fn accept_snapshot(&mut self, snap: &Value, base: &R::Local) {
        if let Some(t) = updated_at(snap) {
            self.etag = Some(t);
        }
        self.server_key = Some(R::stable_key(&R::from_remote(snap, base)));
    }
}

/// A baseline no stable key (JSON text) can equal: "changed since the last
/// sync" when only a digest of the old key survived a restart.
const DIVERGED: &str = "\u{0}diverged";

/// SHA-256 of a stable key, in hex: the persisted form of a last-synced key
/// (archive keys run to megabytes).
pub fn key_digest(key: &str) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

enum Pushed {
    Snapshot(Value),
    Conflict,
    Queued,
}

fn updated_at(v: &Value) -> Option<String> {
    v.get("updatedAt")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// GET the resource; `None` on 404.
async fn fetch<R: SyncResource>(client: &SyncClient) -> Result<Option<Value>, SyncError> {
    let resp = client.get(R::ENDPOINT).await?;
    match resp.status {
        404 => Ok(None),
        _ if resp.ok() => Ok(Some(resp.body)),
        status => Err(http_error::<R>(status)),
    }
}

async fn put<R: SyncResource>(
    client: &SyncClient,
    body: Value,
    etag: Option<&str>,
) -> Result<Pushed, SyncError> {
    let mut req = QueuedReq::new(
        "PUT",
        R::ENDPOINT,
        body,
        Some(SyncedResource::<R>::group_key()),
    );
    if let Some(etag) = etag {
        req = req.header("If-Match", &format!("W/\"{etag}\""));
    }
    let resp: Resp = client.mutate(req).await?;
    match resp.status {
        412 => Ok(Pushed::Conflict),
        Resp::ACCEPTED => Ok(Pushed::Queued),
        _ if resp.ok() => Ok(Pushed::Snapshot(resp.body)),
        status => Err(http_error::<R>(status)),
    }
}

fn http_error<R: SyncResource>(status: u16) -> SyncError {
    SyncError::Http {
        status,
        path: R::ENDPOINT.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::resources::{ArchivesRes, ProfileRes, SourcesRes};
    use crate::sync::test_support::client;
    use crate::types::{Archive, Profile, Source, SourceHealth};
    use httpmock::prelude::*;
    use serde_json::json;

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.into(),
            role: "AD".into(),
            lens: String::new(),
        }
    }

    fn archive(id: i64, run_at: &str) -> Archive {
        Archive {
            id,
            run_at: run_at.into(),
            articles: vec![],
            errors: vec![],
        }
    }

    #[tokio::test]
    async fn pull_404_seeds_from_local() {
        let server = MockServer::start_async().await;
        let get = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/sync/profile")
                    .header("authorization", "Bearer at");
                then.status(404);
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/profile")
                    .header_missing("if-match")
                    .json_body(json!({"name": "Ada", "role": "AD", "lens": null}));
                then.status(200).json_body(json!({
                    "name": "Ada", "role": "AD", "lens": null,
                    "updatedAt": "2026-11-04T10:00:00Z"
                }));
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        let local = profile("Ada");
        let out = r.pull(&c, &local).await.unwrap();
        get.assert_calls_async(1).await;
        put.assert_calls_async(1).await;
        assert_eq!(out, None);
        assert!(r.pulled);
        assert_eq!(r.etag.as_deref(), Some("2026-11-04T10:00:00Z"));
        assert!(!r.needs_push(&local));
    }

    #[tokio::test]
    async fn pull_404_empty_local_no_push() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(404);
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/profile");
                then.status(200).json_body(json!({}));
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        let local = Profile::default();
        assert_eq!(r.pull(&c, &local).await.unwrap(), None);
        put.assert_calls_async(0).await;
        assert!(r.pulled);
        assert_eq!(r.etag, None);
        assert!(!r.needs_push(&local));
        assert!(r.needs_push(&profile("Ada")));
    }

    #[tokio::test]
    async fn pull_remote_replaces_local_and_tracks_etag() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(json!({
                    "name": "Remote", "role": null, "lens": "L", "updatedAt": "t1"
                }));
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        let got = r.pull(&c, &profile("Local")).await.unwrap().unwrap();
        assert_eq!(
            got,
            Profile {
                name: "Remote".into(),
                role: String::new(),
                lens: "L".into()
            }
        );
        assert_eq!(r.etag.as_deref(), Some("t1"));
        assert!(!r.needs_push(&got));
    }

    #[tokio::test]
    async fn pull_error_keeps_pushes_gated() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(500);
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        let err = r.pull(&c, &profile("Ada")).await.unwrap_err();
        assert_eq!(
            err,
            SyncError::Http {
                status: 500,
                path: "/sync/profile".into()
            }
        );
        assert!(!r.pulled);
        assert!(!r.needs_push(&profile("Ada")));
    }

    #[tokio::test]
    async fn push_sends_if_match_weak_etag() {
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/profile")
                    .header("if-match", "W/\"2026-11-04T10:00:00.123456Z\"");
                then.status(200).json_body(json!({
                    "name": "Bea", "role": "AD", "lens": null, "updatedAt": "t2"
                }));
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        r.pulled = true;
        r.etag = Some("2026-11-04T10:00:00.123456Z".into());
        let local = profile("Bea");
        assert!(r.needs_push(&local));
        assert_eq!(r.push(&c, &local).await.unwrap(), None);
        put.assert_calls_async(1).await;
        assert_eq!(r.etag.as_deref(), Some("t2"));
        assert!(!r.needs_push(&local));
    }

    #[tokio::test]
    async fn push_queued_marks_pushed_optimistically() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/profile");
                then.status(503);
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        r.pulled = true;
        r.etag = Some("t1".into());
        let local = profile("Bea");
        assert_eq!(r.push(&c, &local).await.unwrap(), None);
        assert!(!r.needs_push(&local));
        assert_eq!(r.etag.as_deref(), Some("t1"));
        let q = c.queued();
        assert_eq!(q[0].group_key.as_deref(), Some("PUT:/sync/profile"));
        assert_eq!(
            q[0].headers,
            vec![("If-Match".to_string(), "W/\"t1\"".to_string())]
        );

        // The drained response catches etag + server key up.
        r.on_drained(
            &json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t9"}),
            &local,
        );
        assert_eq!(r.etag.as_deref(), Some("t9"));
        assert!(!r.needs_push(&local));
    }

    #[tokio::test]
    async fn push_412_refetches_merges_retries_once() {
        let server = MockServer::start_async().await;
        let stale = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/archives")
                    .header("if-match", "W/\"t1\"");
                then.status(412);
            })
            .await;
        let get = server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/archives");
                then.status(200).json_body(json!({
                    "archives": [archive(3, "2026-11-03T08:00:00Z")],
                    "updatedAt": "t2"
                }));
            })
            .await;
        let merged_body = json!({"archives": [
            archive(2, "2026-11-04T08:00:00Z"),
            archive(3, "2026-11-03T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ]});
        let retry = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/archives")
                    .header("if-match", "W/\"t2\"")
                    .json_body(merged_body.clone());
                let mut echo = merged_body.clone();
                echo["updatedAt"] = json!("t3");
                then.status(200).json_body(echo);
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ArchivesRes>::new();
        r.pulled = true;
        r.etag = Some("t1".into());
        let local = vec![
            archive(2, "2026-11-04T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ];
        let merged = r.push(&c, &local).await.unwrap().expect("merged value");
        stale.assert_calls_async(1).await;
        get.assert_calls_async(1).await;
        retry.assert_calls_async(1).await;
        assert_eq!(
            merged.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert_eq!(r.etag.as_deref(), Some("t3"));
        assert!(!r.needs_push(&merged));
    }

    #[tokio::test]
    async fn push_412_twice_bails_with_merge() {
        let server = MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/profile");
                then.status(412);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Srv", "role": null, "lens": null, "updatedAt": "t2"}),
                );
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        r.pulled = true;
        r.etag = Some("t1".into());
        let merged = r.push(&c, &profile("Mine")).await.unwrap().unwrap();
        put.assert_calls_async(2).await;
        assert_eq!(merged.name, "Srv");
        assert_eq!(r.etag.as_deref(), Some("t2"));
        assert!(!r.needs_push(&merged));
    }

    #[tokio::test]
    async fn noop_echo_not_pushed() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Ada", "role": "AD", "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        let pulled = r.pull(&c, &Profile::default()).await.unwrap().unwrap();
        // Writing the pulled value back into the model is not an edit.
        assert_eq!(r.server_key, Some(ProfileRes::stable_key(&pulled)));
        assert!(!r.needs_push(&pulled));
        assert!(!r.needs_push(&profile("Ada")));
    }

    #[test]
    fn sources_stable_key_ignores_health() {
        let mut r = SyncedResource::<SourcesRes>::new();
        let mut s = vec![Source {
            id: 1,
            name: "Dieline".into(),
            category: "packaging".into(),
            feed_url: Some("https://x/feed".into()),
            enabled: true,
            is_default: true,
            last_fetch_at: None,
            health: SourceHealth::Ok,
        }];
        r.pulled = true;
        r.server_key = Some(SourcesRes::stable_key(&s));
        s[0].last_fetch_at = Some(1_767_261_000_000);
        s[0].health = SourceHealth::Warn;
        assert!(!r.needs_push(&s));
        s[0].enabled = false;
        assert!(r.needs_push(&s));
    }

    /// Fails one pull with a 500, then removes that mock.
    async fn fail_pull<R: SyncResource>(
        server: &MockServer,
        c: &SyncClient,
        r: &mut SyncedResource<R>,
        local: &R::Local,
    ) {
        let down = server
            .mock_async(|when, then| {
                when.method(GET).path(R::ENDPOINT);
                then.status(500);
            })
            .await;
        assert!(r.pull(c, local).await.is_err());
        assert!(!r.pulled);
        down.delete_async().await;
    }

    #[tokio::test]
    async fn offline_edit_after_failed_pull_wins_on_later_pull() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        fail_pull(&server, &c, &mut r, &profile("Ada")).await;
        // Edited while the server was unreachable; a second failure must not
        // forget the pre-edit baseline.
        let edited = profile("Bea");
        fail_pull(&server, &c, &mut r, &edited).await;
        // An in-session baseline, not seeded from an earlier run: the edit
        // still wins over the server's third value ("Srv").
        assert_eq!(r.seed_digest, None);

        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Srv", "role": null, "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/profile")
                    .header("if-match", "W/\"t1\"")
                    .json_body(json!({"name": "Bea", "role": "AD", "lens": null}));
                then.status(200).json_body(
                    json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t2"}),
                );
            })
            .await;
        let got = r.pull(&c, &edited).await.unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(got, Some(edited.clone()));
        assert!(r.pulled);
        assert_eq!(r.etag.as_deref(), Some("t2"));
        assert!(!r.needs_push(&edited));
        assert_eq!(r.failed_pull_key, None);
    }

    #[tokio::test]
    async fn offline_archive_edit_unions_then_pushes() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ArchivesRes>::new();
        let before = vec![archive(1, "2026-11-01T08:00:00Z")];
        fail_pull(&server, &c, &mut r, &before).await;
        let local = vec![
            archive(2, "2026-11-04T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ];

        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/archives");
                then.status(200).json_body(json!({
                    "archives": [archive(3, "2026-11-03T08:00:00Z")], "updatedAt": "t1"
                }));
            })
            .await;
        let union = json!({"archives": [
            archive(2, "2026-11-04T08:00:00Z"),
            archive(3, "2026-11-03T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ]});
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/archives")
                    .header("if-match", "W/\"t1\"")
                    .json_body(union.clone());
                let mut echo = union.clone();
                echo["updatedAt"] = json!("t2");
                then.status(200).json_body(echo);
            })
            .await;
        let got = r.pull(&c, &local).await.unwrap().unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(got.iter().map(|a| a.id).collect::<Vec<_>>(), vec![2, 3, 1]);
        assert!(!r.needs_push(&got));
    }

    #[tokio::test]
    async fn no_divergence_after_failed_pull_takes_remote() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        fail_pull(&server, &c, &mut r, &profile("Ada")).await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Srv", "role": null, "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/profile");
                then.status(200).json_body(json!({}));
            })
            .await;
        let got = r.pull(&c, &profile("Ada")).await.unwrap().unwrap();
        put.assert_calls_async(0).await;
        assert_eq!(got.name, "Srv");
        assert_eq!(r.etag.as_deref(), Some("t1"));
        assert_eq!(r.failed_pull_key, None);
        assert!(!r.needs_push(&got));
    }

    fn digest_of(p: &Profile) -> String {
        key_digest(&ProfileRes::stable_key(p))
    }

    /// Serves `Srv` (etag t1) and accepts a PUT of `body` sent with
    /// If-Match t1.
    async fn srv_row_and_put(server: &MockServer, body: Value) -> httpmock::Mock<'_> {
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Srv", "role": null, "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/profile")
                    .header("if-match", "W/\"t1\"")
                    .json_body(body.clone());
                let mut echo = body.clone();
                echo["updatedAt"] = json!("t2");
                then.status(200).json_body(echo);
            })
            .await
    }

    /// `Srv` as [`srv_row_and_put`] serves it, once pulled.
    fn srv() -> Profile {
        Profile {
            name: "Srv".into(),
            role: String::new(),
            lens: String::new(),
        }
    }

    #[tokio::test]
    async fn seeded_baseline_pushes_an_edit_made_since_the_last_sync() {
        let server = MockServer::start_async().await;
        let put =
            srv_row_and_put(&server, json!({"name": "Bea", "role": "AD", "lens": null})).await;
        let c = client(&server.base_url());
        // An earlier run last synced "Srv", which the server still holds;
        // "Bea" never reached it.
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&srv()), &profile("Bea"));
        assert_eq!(r.seed_digest, Some(digest_of(&srv())));
        let got = r.pull(&c, &profile("Bea")).await.unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(got, Some(profile("Bea")));
        assert!(r.pulled && !r.needs_push(&profile("Bea")));
        assert_eq!(r.etag.as_deref(), Some("t2"));
        assert_eq!(r.seed_digest, None);
    }

    #[tokio::test]
    async fn seeded_baseline_adopts_a_server_that_already_has_local() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT).path("/sync/profile");
                then.status(200).json_body(json!({}));
            })
            .await;
        let c = client(&server.base_url());
        // An earlier run last synced "Ada"; its push of "Bea" landed, but
        // it never recorded that.
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&profile("Ada")), &profile("Bea"));
        let got = r.pull(&c, &profile("Bea")).await.unwrap();
        put.assert_calls_async(0).await;
        assert_eq!(got, Some(profile("Bea")));
        assert_eq!(r.server_key, Some(ProfileRes::stable_key(&profile("Bea"))));
        assert_eq!(r.etag.as_deref(), Some("t1"));
        assert!(r.pulled && !r.needs_push(&profile("Bea")));
        assert_eq!((r.failed_pull_key, r.seed_digest), (None, None));
    }

    #[tokio::test]
    async fn seeded_baseline_keeps_a_server_value_changed_since_the_last_sync() {
        let server = MockServer::start_async().await;
        let put =
            srv_row_and_put(&server, json!({"name": "Bea", "role": "AD", "lens": null})).await;
        let c = client(&server.base_url());
        // An earlier run last synced "Ada" and never got "Bea" to the
        // server; another device has since written "Srv".
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&profile("Ada")), &profile("Bea"));
        let got = r.pull(&c, &profile("Bea")).await.unwrap();
        put.assert_calls_async(0).await;
        assert_eq!(got, Some(srv()), "the server's copy wins");
        assert_eq!(r.server_key, Some(ProfileRes::stable_key(&srv())));
        assert_eq!(r.etag.as_deref(), Some("t1"));
        assert!(r.pulled && !r.needs_push(&srv()));
        assert_eq!((r.failed_pull_key, r.seed_digest), (None, None));
    }

    #[tokio::test]
    async fn seeded_archives_union_even_when_the_server_changed() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let synced = vec![archive(1, "2026-11-01T08:00:00Z")];
        let local = vec![
            archive(2, "2026-11-04T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ];
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/archives");
                then.status(200).json_body(json!({
                    "archives": [archive(3, "2026-11-03T08:00:00Z")], "updatedAt": "t1"
                }));
            })
            .await;
        let union = json!({"archives": [
            archive(2, "2026-11-04T08:00:00Z"),
            archive(3, "2026-11-03T08:00:00Z"),
            archive(1, "2026-11-01T08:00:00Z"),
        ]});
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/archives")
                    .header("if-match", "W/\"t1\"")
                    .json_body(union.clone());
                let mut echo = union.clone();
                echo["updatedAt"] = json!("t2");
                then.status(200).json_body(echo);
            })
            .await;
        let mut r = SyncedResource::<ArchivesRes>::new();
        r.seed_last_synced(&key_digest(&ArchivesRes::stable_key(&synced)), &local);
        let got = r.pull(&c, &local).await.unwrap().unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(got.iter().map(|a| a.id).collect::<Vec<_>>(), vec![2, 3, 1]);
        assert!(!r.needs_push(&got));
    }

    /// I-1: this device's own queued edit drains while a seeded baseline is
    /// pending. The drained value is now the last-synced one, so a newer
    /// edit made this session is pushed over it rather than dropped as a
    /// "third value", and a further failed pull persists the drained digest.
    #[tokio::test]
    async fn drained_own_write_advances_a_seeded_baseline() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let synced = profile("Ada"); // last synced by the earlier run
        let stuck = profile("Bea"); // its quit push, queued
        let newer = profile("Cy"); // edited this session while offline
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&synced), &stuck);
        fail_pull(&server, &c, &mut r, &stuck).await;

        // The queued PUT of "Bea" lands.
        r.on_drained(
            &json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t2"}),
            &stuck,
        );
        assert_eq!(r.seed_digest, Some(digest_of(&stuck)));
        fail_pull(&server, &c, &mut r, &newer).await;
        assert_eq!(r.last_synced_digest(), Some(digest_of(&stuck)));

        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t2"}),
                );
            })
            .await;
        let put = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path("/sync/profile")
                    .header("if-match", "W/\"t2\"")
                    .json_body(json!({"name": "Cy", "role": "AD", "lens": null}));
                then.status(200).json_body(
                    json!({"name": "Cy", "role": "AD", "lens": null, "updatedAt": "t3"}),
                );
            })
            .await;
        let got = r.pull(&c, &newer).await.unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(got, Some(newer.clone()));
        assert_eq!(r.etag.as_deref(), Some("t3"));
        assert!(r.pulled && !r.needs_push(&newer));
        assert_eq!(r.seed_digest, None);
    }

    /// Without a seeded baseline a drain leaves the seed alone (nothing to
    /// advance).
    #[test]
    fn drain_without_a_pending_seed_adds_none() {
        let mut r = SyncedResource::<ProfileRes>::new();
        r.on_drained(
            &json!({"name": "Bea", "role": "AD", "lens": null, "updatedAt": "t2"}),
            &profile("Bea"),
        );
        assert_eq!(r.seed_digest, None);
        assert_eq!(r.last_synced_digest(), Some(digest_of(&profile("Bea"))));
    }

    #[tokio::test]
    async fn last_synced_digest_is_the_seed_while_a_seeded_pull_fails() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&srv()), &profile("Bea"));
        fail_pull(&server, &c, &mut r, &profile("Bea")).await;
        assert_eq!(r.last_synced_digest(), Some(digest_of(&srv())));
        assert_eq!(r.seed_digest, Some(digest_of(&srv())), "kept");
        assert_ne!(
            r.failed_pull_key,
            Some(ProfileRes::stable_key(&profile("Bea")))
        );

        // So the next run, seeded from it, still pushes over the unchanged
        // server row; and the successful pull clears the seed.
        let mut next_run = SyncedResource::<ProfileRes>::new();
        next_run.seed_last_synced(&r.last_synced_digest().unwrap(), &profile("Bea"));
        let put =
            srv_row_and_put(&server, json!({"name": "Bea", "role": "AD", "lens": null})).await;
        next_run.pull(&c, &profile("Bea")).await.unwrap();
        put.assert_calls_async(1).await;
        assert_eq!(next_run.seed_digest, None);
        assert_eq!(
            next_run.last_synced_digest(),
            Some(digest_of(&profile("Bea")))
        );
    }

    #[tokio::test]
    async fn seeded_baseline_without_an_edit_takes_remote() {
        let server = MockServer::start_async().await;
        let put =
            srv_row_and_put(&server, json!({"name": "Ada", "role": "AD", "lens": null})).await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        r.seed_last_synced(&digest_of(&profile("Ada")), &profile("Ada"));
        let got = r.pull(&c, &profile("Ada")).await.unwrap().unwrap();
        put.assert_calls_async(0).await;
        assert_eq!(got.name, "Srv");
        assert_eq!(r.failed_pull_key, None);
    }

    #[tokio::test]
    async fn last_synced_digest_follows_server_key_then_baseline() {
        let server = MockServer::start_async().await;
        let c = client(&server.base_url());
        let mut r = SyncedResource::<ProfileRes>::new();
        assert_eq!(r.last_synced_digest(), None, "nothing known yet");

        // A failed pull: the baseline is what the next run compares against.
        fail_pull(&server, &c, &mut r, &profile("Ada")).await;
        assert_eq!(r.last_synced_digest(), Some(digest_of(&profile("Ada"))));

        // A seeded divergence survives a failed pull and the next restart.
        let mut seeded = SyncedResource::<ProfileRes>::new();
        seeded.seed_last_synced(&digest_of(&profile("Ada")), &profile("Bea"));
        fail_pull(&server, &c, &mut seeded, &profile("Bea")).await;
        let persisted = seeded.last_synced_digest().unwrap();
        let mut next_run = SyncedResource::<ProfileRes>::new();
        next_run.seed_last_synced(&persisted, &profile("Bea"));
        assert_ne!(
            next_run.failed_pull_key,
            Some(ProfileRes::stable_key(&profile("Bea")))
        );

        // A successful pull: the server's value.
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/profile");
                then.status(200).json_body(
                    json!({"name": "Srv", "role": null, "lens": null, "updatedAt": "t1"}),
                );
            })
            .await;
        let got = r.pull(&c, &profile("Ada")).await.unwrap().unwrap();
        assert_eq!(r.last_synced_digest(), Some(digest_of(&got)));

        // A seed never overrides a baseline this run already has.
        let mut r = SyncedResource::<ProfileRes>::new();
        r.failed_pull_key = Some("k".into());
        r.seed_last_synced(&digest_of(&profile("Ada")), &profile("Ada"));
        assert_eq!(r.failed_pull_key.as_deref(), Some("k"));
    }

    #[test]
    fn key_digest_is_sha256_hex() {
        assert_eq!(
            key_digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn reset_clears_guards() {
        let mut r = SyncedResource::<ProfileRes>::new();
        r.pulled = true;
        r.server_key = Some("k".into());
        r.etag = Some("e".into());
        r.failed_pull_key = Some("f".into());
        r.seed_digest = Some("s".into());
        r.reset();
        assert!(!r.pulled && r.server_key.is_none() && r.etag.is_none());
        assert!(r.failed_pull_key.is_none() && r.seed_digest.is_none());
        assert_eq!(
            SyncedResource::<ProfileRes>::group_key(),
            "PUT:/sync/profile"
        );
    }
}
