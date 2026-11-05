### Task 6: Sync client, offline queue, synced resources, article states

**Files:**
- Create: `radar-core/src/sync/{mod,queue,resource,resources,article_states}.rs`

**Interfaces:**
- Consumes:
  - `auth::Session::access_token`
  - `model::RadarModel` (resources read and write through closures, so sync never owns the model)
  - `store` (the queue persists through a callback)
- Produces:
  - `sync::SyncClient::new(base: String, session: Session) -> Self`
  - `get(path) -> Result<Resp, SyncError>`
  - `mutate(QueuedReq) -> Result<Resp, SyncError>`. On a retryable status or a network error it enqueues and returns `Resp::Accepted` (port of `syncQueue.queuedMutation`).
  - `Resp { status: u16, etag: Option<String>, body: serde_json::Value }`
  - `sync::queue::Queue`:
    - `{ items: Vec<QueuedReq> }`
    - `QueuedReq { id, path, method, body, headers: Vec<(String,String)>, group_key: Option<String>, account_id: Option<String>, tries: u32, queued_at: i64 }`. Serde keys match the JS (`groupKey`, `accountId`, `queuedAt`) so an old queue drains.
    - `Queue::enqueue(req)` dedups by `group_key`.
    - `Queue::drain(&mut self, client, account_id, persist: impl Fn(&[QueuedReq])) -> DrainReport { succeeded: Vec<(String /*group_key*/, Value)>, dropped, remaining }`.
  - `sync::SyncStatus { pending: usize, inflight: bool, last_error: Option<String>, last_success_at: Option<i64> }` through `SyncClient::status() -> watch::Receiver<SyncStatus>`.
  - `sync::resource::SyncResource` trait:
    - `const ENDPOINT: &str; type Local: Clone;`
    - `fn push_body(&Local) -> Value; fn from_remote(&Value, prev: &Local) -> Local; fn stable_key(&Local) -> String; fn is_empty(&Local) -> bool;`
  - `sync::resource::SyncedResource<R>`:
    - `{ pulled: bool, server_key: Option<String>, etag: Option<String> }`
    - `async fn pull(&mut self, client, local: &R::Local) -> Result<Option<R::Local>, SyncError>`, which seeds the server when it returns 404 and local is non-empty.
    - `fn needs_push(&self, local) -> bool`
    - `async fn push(&mut self, client, local) -> Result<Option<R::Local>, SyncError>`. `Some` is the merged value after a 412 refetch + merge + single retry.
    - `fn on_drained(&mut self, body: &Value, local)`
    - `fn reset()`, for account switches.
  - `resources::{ProfileRes, CategoriesRes, SourcesRes, PrefsRes, ArchivesRes}` implement the trait. Their bodies and merges are ported 1:1 from `App.jsx:193-381`: `sourceWire` fields only, health preserved by id, prefs limited to the 9 synced keys merged over defaults, and archives union-merged + sorted by `runAt` desc + capped at 90.
  - `article_states::{pull(client) -> HashMap<String, ArticleState>, seed_local_only(client, local_articles, server_map), patch(client, id, state)}`, where `group_key = "PATCH:/sync/article-states:{id}"`.

- [ ] **Step 1: Write failing tests** with `httpmock`, mirroring the JS:
  - `pull_404_seeds_from_local`: PUT is called with no `If-Match`.
  - `pull_404_empty_local_no_push`.
  - `push_sends_if_match_weak_etag`: the header is exactly `W/"<updatedAt>"`.
  - `push_412_refetches_merges_retries_once`, which also covers the archives union so local-only entries reach the server.
  - `noop_echo_not_pushed`: the stable key equals the server key, so `needs_push` is false.
  - `sources_stable_key_ignores_health`: bumping `last_fetch_at` doesn't trigger a push.
  - `mutate_503_enqueues_returns_accepted`.
  - `queue_dedups_by_group_key`.
  - `queue_skips_other_account_items`.
  - `queue_drops_non_retryable_4xx`: 412 and 400 are dropped, and `last_error` is set.
  - `queue_gives_up_after_8_tries`, using an injectable sleep fn so the test is instant.
  - `drain_refreshes_expired_token` (Review Focus 4): the session holds an expired token, and the mock token endpoint plus mock PUT see a fresh bearer.
  - `legacy_js_queue_item_deserializes`.
  - `article_states_seed_skips_server_known_ids`.
- [ ] **Step 2: Run `cargo test -p radar-core sync`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** The backoff formula is from Global Constraints. The sleep function is a field on `Queue` (`Arc<dyn Fn(Duration) -> BoxFuture>`), defaulting to `tokio::time::sleep`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): sync client, offline queue, synced resources"`.

