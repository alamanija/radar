# Task 6 report: sync client, offline queue, synced resources, article states

**Status:** DONE_WITH_CONCERNS. The concerns are design notes for the controller and Task 12, not failures. Every file is under 500 non-test lines.

**Commit:** `73352ac feat(core): sync client, offline queue, synced resources` (BASE b038870)

## What I implemented

`radar-core/src/sync/` follows the brief's layout, and `pub mod sync;` is added to `lib.rs`.

### `mod.rs`: SyncClient, Resp, SyncError, SyncStatus

- **`SyncClient::new(base, session)`**
  - `base` has its trailing `/` trimmed.
  - Uses a reqwest client (rustls from the crate features) with a 30 s timeout.
  - The `Queue` sits behind a `tokio::sync::Mutex`. `SyncStatus` goes out through a `watch` channel.
- **Builders:**
  - `with_persist(PersistCallback)`, where `PersistCallback = Box<dyn Fn(&[QueuedReq]) + Send + Sync>`. It is called after every queue mutation.
  - `with_queue(Vec<QueuedReq>)` restores a queue and sets `pending`.
  - `with_sleep(SleepFn)`.
- **Accessors:** `status()`, `session()`, `account_id()` (= `session.account().map(|a| a.id)`), and `queued()` (a snapshot of the queue).
- **`get(path)`** returns `Ok(Resp)` for any HTTP status.
- **`mutate(QueuedReq)`:**
  - A non-retryable status is returned as-is (412, 400 and so on).
  - A retryable status (408/429/5xx) or `SyncError::Network` enqueues the request, tagged with `account_id` and `tries = 0`, persists, and returns `Resp::accepted(body)`. That is a synthetic 202 that echoes the request body, or `{}` when the body is null.
  - `NotSignedIn` returns `Err` and does **not** enqueue. The brief lists only retryable statuses and network errors, and an untagged signed-out item would later drain into whichever account signs in.
- **`drain()`** reads the account, locks the queue and calls `Queue::drain(self, account, persist)`.
- **Sending a request:**
  - The token is fetched per request through `Session::access_token()`, which refreshes near expiry.
  - On a 401, the token is fetched again and the request is resent once. A second 401 is returned as a final status.
  - Headers are built in a `HeaderMap` with `insert`: Content-Type first, then the stored headers (which can override it), then `Authorization` last, marked sensitive. A stored Authorization can never win, and nothing is duplicated.
  - A null body sends no body, matching JS `body != null`.
  - On a 2xx, an empty response body becomes `Value::Null` and invalid JSON becomes `Err(Decode)`. On a non-2xx, the body is parsed leniently and falls back to Null.
- **`SyncError`** has the variants `NotSignedIn | Http{status,path} | Network(String) | Decode(String)`. Network messages are built from the reqwest error *kind* plus the path ("connect failed on /sync/prefs"), never reqwest's Display, which can include the URL. A token-refresh failure maps to `Network("token refresh: <AuthError>")`; `AuthError`'s Display is already secret-free.

### `queue.rs`

- **`QueuedReq`** has the fields `{id, path, method, body, headers: Vec<(String,String)>, group_key, account_id, tries, queued_at}`.
  - Serde keys are camelCase (`groupKey`, `accountId`, `queuedAt`), and every optional field has a default.
  - `headers` is (de)serialized as a JS **object**, which is what the JS wrote. Lists of `[k, v]` pairs are also accepted, and non-string values are skipped.
  - `QueuedReq::new(method, path, body, group_key)` sets a random UUID-shaped id and `queued_at = now_ms`. `.header(k, v)` is a builder.
- **`Queue { items, sleep: SleepFn }`**, where `SleepFn = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>`. The default is `tokio::time::sleep`.
- **`Queue::enqueue`** removes any item with the same `group_key`, then pushes the new one.
- **`Queue::drain(&mut self, client, account_id, persist) -> DrainReport { succeeded: Vec<(group_key, body)>, dropped: usize, remaining: usize }`**, a port of the JS loop:
  - Items tagged with another account are skipped and left in place. Untagged items drain for anyone.
  - On a 2xx: `last_success_at = now`, `last_error = None`. If the item has a group key, `(group_key, body)` goes into `succeeded`. The item is removed and the queue persisted.
  - On a non-retryable 4xx (412 and 401-after-retry included): the item is dropped, a warning is logged, and `last_error = "<status> on <path>"` is set.
  - On a retryable status: `tries += 1`. If `tries > MAX_TRIES` (8), the item is dropped with `last_error = "gave up on <path> after 8 tries"`, matching the JS. Otherwise the queue is persisted, then it sleeps `backoff(tries)` and retries the same item.
  - On `NotSignedIn`, the pass stops.
  - On a network error or token-refresh failure, the pass stops without spending a try (see deviation 2).
  - A 2xx with an unreadable body counts as landed (JS `.json().catch(() => null)`).
- **`backoff(tries)`** is `min(60_000, 500 * 2^min(tries, 6))` ms. **`is_retryable`** is true for 408, 429 and 5xx.
- **Status updates:** `pending` is updated on every queue change. An `InflightGuard` sets `inflight` for the length of a drain and clears it even if the future is dropped.

### `resource.rs`: SyncResource trait and SyncedResource<R>

- **The trait** has `ENDPOINT`, `type Local: Clone`, `push_body`, `from_remote`, `stable_key` and `is_empty`, exactly as in the brief.
- **`SyncedResource<R> { pulled, server_key, etag }`** also provides `new`/`Default`, `group_key()` (`"PUT:<endpoint>"`) and `reset()`.
- **`pull(client, local)`:**
  - It resets the guards first, as the JS effect does.
  - If the server returns a body: `etag = body.updatedAt`, `next = from_remote`, `server_key = stable_key(next)`, and it returns `Some(next)`.
  - If the server returns 404 and local is non-empty: it seeds with a PUT that has **no If-Match**. A returned snapshot is accepted (etag + key). If the PUT was queued or hit a conflict, the key is set optimistically. It returns `None`.
  - If the server returns 404 and local is empty: `server_key = stable_key(local)`.
  - `pulled = true` is set only on success (see deviation 1).
- **`needs_push(local)`** is `pulled && server_key != stable_key(local)`.
- **`push(client, local)`:**
  - It sends a PUT with `If-Match: W/"<etag>"` and the group key `PUT:<endpoint>`.
  - A returned snapshot updates the etag (`?? old`) and sets `server_key = stable_key(from_remote(snap, local))`.
  - A 202 sets the key optimistically.
  - A **412** refetches (a 404 there returns `Ok(None)`), sets `etag = updatedAt`, merges with `from_remote`, and makes **one** retry PUT with the fresh etag. A snapshot from the retry is accepted; a second 412 or a queued retry sets the key optimistically. It returns `Some(merged)`.
  - If the retry fails with an error, the merged value is still returned (the JS had already called `setValue`), the error is logged, and `server_key` is left unchanged so the next push retries.
  - Any other non-2xx returns `Err(Http)`.
- **`on_drained(body, local)`** does nothing for a null body. Otherwise it updates the etag from `updatedAt` and sets `server_key = stable_key(from_remote(body, local))`.

### `resources.rs`

Remote lists are parsed item by item: a bad item is skipped with a warning.

- **ProfileRes:** empty strings are sent as `null`. On pull the server wins, with null becoming `""`. `stable_key` is the Profile JSON, and the profile counts as empty when all three fields are empty.
- **CategoriesRes:** full-list replace. A snapshot without a categories array keeps the local list (the JS would have set `undefined`).
- **SourcesRes:**
  - Only the `sourceWire` fields `{id, name, category, feedUrl, enabled, isDefault}` are sent.
  - `from_remote` keeps this device's `last_fetch_at`/`health` by id. New sources get `None`/`Ok`.
  - `stable_key` covers the wire fields only.
  - A missing array keeps the local list.
- **PrefsRes:**
  - The push body is `{prefs: {the 9 synced keys}}`.
  - `from_remote` reads `snap.prefs` (or `{}`) through the lenient `Prefs` deserializer, so each missing or invalid key falls back to its default. `sidebar_collapsed` and `view` keep the local values.
  - `stable_key` covers the 9 keys only. `is_empty` is always false.
- **ArchivesRes:**
  - Local and remote are merged in insertion order, with remote winning on a duplicate id. The result is stable-sorted by parsed `runAt` descending (an unparseable `runAt` sorts last) and truncated to `ARCHIVE_CAP = 90`.
  - A non-array remote means `[]`, so the result is local, sorted and capped. This matches the JS.

### `article_states.rs`

- **`pull(client)`** returns `HashMap<String, ArticleState>` built from `states[]` (`articleId`, `read`, `bookmarked`). A non-2xx is `Err(Http)`.
- **`overlay(articles, map)`:** server states win for the ids they cover.
- **`seed_local_only(client, local_articles, server_map)`** PATCHes the local read/bookmarked flags for ids the server doesn't have, skipping articles with neither flag set. Each failure is logged and the loop carries on. It returns the seeded `(id, state)` pairs so the app can update `model.article_states`.
- **`patch(client, id, state)`** calls `mutate` with PATCH `/sync/article-states`, body `{articleId, read, bookmarked}`, and `group_key = "PATCH:/sync/article-states:{id}"`.

## Tests and results

35 tests in `sync`. Crate total: **104 passed, 0 failed** (`cargo test -p radar-core`). `cargo clippy -p radar-core --all-targets` reports **no warnings**. `cargo fmt` has been applied.

**All 14 tests the brief requires are present and passing:**

- `pull_404_seeds_from_local`: the mock PUT uses `header_missing("if-match")` and an exact JSON body.
- `pull_404_empty_local_no_push`
- `push_sends_if_match_weak_etag`: header exactly `W/"2026-11-04T10:00:00.123456Z"`.
- `push_412_refetches_merges_retries_once`: archives. The stale If-Match gets 412, the GET returns the remote list, and the retry PUT matches the exact merged body (local-only ids 2 and 1 plus remote id 3, sorted) with `W/"t2"`. Each mock is called once.
- `noop_echo_not_pushed`
- `sources_stable_key_ignores_health`
- `mutate_503_enqueues_returns_accepted`
- `queue_dedups_by_group_key`
- `queue_skips_other_account_items`
- `queue_drops_non_retryable_4xx`: 412 and 400 are both dropped; `last_error = "400 on /sync/bad"`.
- `queue_gives_up_after_8_tries`: 9 sends; recorded sleeps `[1000,2000,4000,8000,16000,32000,32000,32000]` ms; no real waiting.
- `drain_refreshes_expired_token` (Review Focus 4): the session's token expired 120 s ago. The mock `/oauth/token` is called once. The PUT carrying `Bearer at-expired` gets 0 calls; the PUT carrying `Bearer at-fresh` plus the stored If-Match gets 1. Nothing is dropped.
- `legacy_js_queue_item_deserializes`: a JS-shaped item round-trips byte-equal, missing or null optional keys are handled, pair-list headers are accepted, and the queue in `tests/fixtures/tauri_radar.json` parses.
- `article_states_seed_skips_server_known_ids`

**Extra tests:**

- `backoff_formula`
- `mutate_network_error_enqueues`
- `mutate_non_retryable_returned_not_queued`
- `mutate_signed_out_errors_without_queueing`
- `mutate_401_retries_once_then_returns`
- `retryable_then_success_keeps_tries_and_lands`: tries are persisted before each sleep.
- `network_error_stops_drain_without_using_tries`
- `pull_remote_replaces_local_and_tracks_etag`
- `pull_error_keeps_pushes_gated`
- `push_queued_marks_pushed_optimistically` (also covers `on_drained`)
- `push_412_twice_bails_with_merge`
- `reset_clears_guards`
- resources: `profile_body_nulls_empty_fields`, `categories_roundtrip`, `sources_wire_and_health_preserved_by_id`, `prefs_only_nine_keys_merged_over_defaults`, `archives_union_sorted_capped`
- article_states: `pull_maps_states_by_id` (plus overlay), `patch_uses_per_article_group_key`
- mod: `client_and_futures_are_send_sync` (`SyncClient: Send + Sync`; the drain/get/mutate/push/pull futures are `Send`), `errors_carry_status_and_path_only`

## TDD evidence

**RED.** I wrote the full API as signatures with `todo!()` bodies, plus all the tests:

```
$ cargo test -p radar-core sync
...
thread 'sync::resources::tests::archives_union_sorted_capped' panicked at radar-core\src\sync\resources.rs:96:9:
not yet implemented
...
test result: FAILED. 0 passed; 33 failed; 0 ignored; 0 measured; 69 filtered out; finished in 0.00s
```

**GREEN.** After the implementation:

```
$ cargo test -p radar-core sync
running 33 tests
test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 69 filtered out; finished in 2.02s
```

**Final run**, after clippy and fmt and with the 2 extra mod tests added:

```
$ cargo test -p radar-core
test result: ok. 104 passed; 0 failed; ...
$ cargo clippy -p radar-core --all-targets
    Finished `dev` profile ... (no warnings)
```

(The single clippy warning was `type_complexity` on `with_persist`. I fixed it by adding a `pub type PersistCallback`.)

## Files changed

- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\lib.rs` (+`pub mod sync;`)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\sync\mod.rs` (303 code lines + tests)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\sync\queue.rs` (307 + tests)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\sync\resource.rs` (236 + tests)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\sync\resources.rs` (283 + tests)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\sync\article_states.rs` (93 + tests)

## Deliberate deviations from the JS (please rule)

1. **A failed pull leaves `pulled = false`.** The JS set `pulled` in `finally`, so it was set even on failure. With `serverKey` still null, the next push then went out with no If-Match, and the server accepts that unconditionally. A network blip at sign-in could therefore overwrite server data the client had never seen. In Rust, pushes stay gated until a pull succeeds, so the app has to retry the pull (for example on its next tick or when the network comes back). This is tested by `pull_error_keeps_pushes_gated`.
2. **A network error, or a token-endpoint failure, during a drain stops the pass without spending a try.** The JS did spend a try, but only the first failure counted, because its `navigator.onLine` check then stopped the loop. Rust has no online flag. If network errors counted as tries, a few app-scheduled drains while offline would burn through all 8 tries and drop real writes, which goes against Review Focus 4. Retryable *statuses* (408/429/5xx) still count tries and back off exactly as in the JS.
3. **`last_error` is also set on retryable statuses and on network stops,** not only on terminal drops. This follows the JS doc comment ("retryable or terminal"); the JS code only set it on drops. A later success clears it.
4. **Signed-out `mutate` returns `Err(NotSignedIn)` instead of enqueueing.** The reasons are in the `mutate` section above.
5. **Non-array remote lists keep the local value** for categories and sources. The JS would have produced `undefined`.

## Self-review findings and concerns

- **An in-progress drain blocks `mutate`'s enqueue (Task 12).** The decisions put the queue behind an async mutex, and the brief's `Queue::drain(&mut self, …)` holds that mutex for the whole pass, backoff sleeps included. While the server keeps answering 5xx, one pass can take about 2 minutes per item (sleeps of 1+2+4+8+16+32+32+32 s). During that time, any `mutate` that also fails and needs to enqueue waits for the lock. `mutate` calls that succeed are not affected; only the enqueue step waits. The model and UI are already updated optimistically, so nothing freezes; the task's future just resolves late.
  - **Ask:** if that latency matters, Task 12 should spawn pushes and patches as independent tasks rather than awaiting them inline. A follow-up could move the backoff sleep outside the lock, but that needs a different `drain` signature than the brief's.
- **No live-session pull (Task 12).** Nothing in radar-core fetches the token before the first request or schedules drains, as decided. The app needs to:
  - call `drain()` on startup and on a timer while `status.pending > 0`;
  - route `DrainReport.succeeded` entries whose key matches `SyncedResource::<R>::group_key()` into `on_drained`;
  - call `reset()` and then `pull` on an account change.
- **The store fixture's queue item would be dropped.** Items in `tests/fixtures/tauri_radar.json` store `body` as a JSON *string*, while the real JS stored objects. A string body is sent as a JSON string literal, exactly as the JS `JSON.stringify` would, and the server would answer 4xx, so the item gets dropped. Real JS-written queues use object bodies, which `legacy_js_queue_item_deserializes` checks; the fixture parses fine.
- **"Gives up after 8 tries" means 9 sends.** This keeps the JS semantics (`tries > MAX_TRIES`), and the test asserts it explicitly.
- **Archives with a string `id` are skipped on pull.** The server accepts string or number ids, but `types::Archive.id` is `i64`. The app only ever writes numeric ids (`Date.now()`), and each skipped item is logged with a warning.
- **Security:**
  - Tokens are never logged or put in an error. The Authorization header is marked sensitive.
  - Network error strings carry the error kind and path only.
  - Log lines carry method, path and status only, never bodies.

---

## Fix round 1 (review I1–I3, rulings R18–R20)

All three Important findings are fixed. `cargo test -p radar-core`: **114 passed, 0 failed** (up from 104). `cargo clippy -p radar-core --all-targets`: no warnings. `cargo fmt --check`: clean. Non-test lines per file: mod.rs 378, queue.rs 354, resource.rs 293, resources.rs 288, article_states.rs 93, all under 500.

### I1 / R18: the 401 retry now uses a force-refreshed token
- `auth.rs` adds `Session::force_refresh() -> Result<Option<String>, AuthError>`.
  - It refreshes regardless of expiry when a refresh token exists.
  - It uses the same `refresh_lock`.
  - It returns the newer token, without refreshing again, if another caller swapped the tokens while it waited.
  - Without a refresh token it returns the current token.
  - `invalid_grant` clears the tokens and returns `Ok(None)`.
  - The shared refresh path moved into a private `refresh_locked` that both `access_token` and `force_refresh` use.
- `SyncClient::send`: on a 401 it calls `force_refresh()` and resends once with the **new** bearer. A direct `mutate` returns the final 401 `Resp`.
- `Queue::drain`: a 401 after the forced refresh keeps the item. It counts a try (through `bump_try`, which drops the item past MAX_TRIES as before), persists, sets `last_error = "401 on <path>"` and stops the pass.
- Tests:
  - `mutate_401_retries_once_then_returns` was rewritten. The `Bearer at` and `Bearer at-new` mocks are called once each, and the token endpoint once.
  - New: `drain_401_after_refresh_keeps_item_and_counts_try`, `auth::tests::force_refresh_ignores_expiry` (also covers the no-refresh-token and signed-out cases), and `auth::tests::force_refresh_invalid_grant_signs_out`.

### I2 / R19: enqueue never waits for a drain; drains don't stack
- `SyncClient::drain` uses `try_lock`. If a drain is already running it returns at once with an empty report; `remaining` is the current queue length.
- New items go to an inbox held in `std::sync::Mutex<Shared { view, inbox }>`.
  - `view` mirrors the queue as of its last persist.
  - Every enqueue persists `combined()` straight away (view + inbox, with group-key dedup, newer wins) and updates `pending`. `mutate` and enqueue never touch the drain lock.
- `Queue::drain(&mut self, …)` keeps the brief's API. At the start of every iteration it calls `client.take_inbox()` and `absorb`s the items (`push_dedup`, newer wins). The cursor is adjusted when dedup removes items before it.
- `take_inbox` folds the inbox into `view` under the same lock, so the persisted picture never loses absorbed items. `queued()` is now synchronous and returns `combined()`. `DrainReport.remaining` includes inbox items that arrived after an early stop.
- Tests:
  - `mutate_does_not_block_during_drain`: a drain is parked inside a gated sleep. `mutate` returns 202 within a 5 s timeout, and the persisted list is `[/sync/a, /sync/b]`. After release, the drain picks up b.
  - `concurrent_drain_returns_early`
  - `inbox_dedups_against_queue_newer_wins`: only the new body is sent, and the old-body mock is never called.
  - `absorb_keeps_cursor_on_current_item`

### I3 / R20: edits made during failed pulls win on the next pull
- `SyncedResource` gains `pub failed_pull_key: Option<String>`. It holds the stable key from the **first** failed pull since the last success, so a second failure after an edit doesn't move the baseline. `reset()` clears it.
- A failed pull keeps `pulled = false` and records or keeps the baseline.
- On a later successful GET:
  - If `stable_key(local)` differs from the baseline, the code computes `R::merge_offline(remote, local)` and pushes it through `push()` with the fresh etag (a 412 there falls back to push's refetch-and-merge). It returns `Some(pushed value)`.
  - Otherwise it replaces local with remote as before.
  - If the pull fails again, including during that push, the baseline is restored.
- `SyncResource::merge_offline` is a new trait method that defaults to local-wins. `ArchivesRes` overrides it with the union merge.
- The module docs now list the app's obligations: retry failed pulls, `reset` then `pull` on account change, and route drain successes to `on_drained`.
- Tests:
  - `offline_edit_after_failed_pull_wins_on_later_pull`: profile; two failures, then the PUT carries `W/"t1"` and the edited body.
  - `offline_archive_edit_unions_then_pushes`
  - `no_divergence_after_failed_pull_takes_remote`: no PUT.
  - `reset_clears_guards` was extended.

### TDD evidence
- **I3** was test-first. I added the field with no behavior, then ran `cargo test -p radar-core sync::resource`:
  ```
  test sync::resource::tests::reset_clears_guards ... FAILED
  test sync::resource::tests::offline_edit_after_failed_pull_wins_on_later_pull ... FAILED
  test sync::resource::tests::offline_archive_edit_unions_then_pushes ... FAILED
  test result: FAILED. 16 passed; 3 failed
  ```
  `no_divergence…` passed already, because it describes the existing behavior. After the implementation all of them pass.
- **I1 and I2** were implemented before their tests. For RED evidence I temporarily reverted each fix and re-ran the new tests (each fix was restored afterwards):
  - With `force_refresh` swapped back to `access_token`, `cargo test -p radar-core 401` failed both `mutate_401_retries_once_then_returns` and `drain_401_after_refresh_keeps_item_and_counts_try` (0 passed / 2 failed).
  - With the drain using `lock().await` instead of `try_lock`, `concurrent_drain_returns_early` FAILED on the "second drain must not wait" timeout (5.01 s).
  - With `mutate` awaiting the queue lock before enqueueing, `mutate_does_not_block_during_drain` FAILED on the "mutate must not wait for the drain" timeout.
- **Final run:**
  ```
  $ cargo test -p radar-core
  test result: ok. 114 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
  $ cargo clippy -p radar-core --all-targets
      Finished `dev` profile [unoptimized + debuginfo] target(s)   (no warnings)
  ```

### Files changed in this round
- `radar-core/src/auth.rs`
- `radar-core/src/sync/mod.rs`
- `radar-core/src/sync/queue.rs`
- `radar-core/src/sync/resource.rs`
- `radar-core/src/sync/resources.rs`
- `radar-core/src/sync/article_states.rs` (test only: `queued()` is now synchronous)

### Notes
- The persist callback runs under the internal `shared` lock, so it must not call back into the `SyncClient`. This is documented on `with_persist`.
- If another write lands between the post-outage GET and the push of an offline edit, the push gets a 412 and goes through the normal server-wins merge for non-archive resources. That is a real concurrent conflict and is handled the same way as in normal operation.
