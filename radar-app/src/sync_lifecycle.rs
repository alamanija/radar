//! Sign-in, sign-out and the live sync loop: ports of the sync effects in
//! `App.jsx`, `useSyncedResource.js`, `syncQueue.js` and `flushRegistry.js`.
//!
//! # Shape
//! - One signed-in session owns a [`Lifecycle`]: a worker task that runs
//!   sync work one op at a time (pull, push, drain), a task forwarding the
//!   client's [`SyncStatus`], a 30 s ticker and the persist-queue listener.
//!   Dropping it (sign-out, account switch) cancels all of them.
//! - Pulls and pushes call `&mut` methods across awaits on the tokio
//!   runtime, so the five [`SyncResources`] are *lent* to the op (moved
//!   into the tokio job and handed back). While they are out,
//!   `AppModel::resources` holds a placeholder and other ops wait.
//! - `sync_epoch` is bumped on every account boundary. Work that started
//!   under an older epoch never writes its results back, and its tokio
//!   tasks are aborted ([`RuntimeWork`]): `Session` clones share their
//!   tokens, so a request of account A still running after the switch would
//!   otherwise go out with B's bearer token.
//! - Results are merged with the *live* model when applied: per-device
//!   prefs and source stamps, and archives added while the op ran, survive.
//! - The model is only ever touched on the UI thread. Network work runs on
//!   tokio and keychain work on its blocking pool, never the UI thread.
//!   That includes `Session::is_signed_in`, whose mutex `Session` holds
//!   during keychain writes.

use super::AppModel;
use crate::bridge;
use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::StreamExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AsyncApp, Context, Task, WeakEntity};
use radar_core::auth::{AuthError, Session};
use radar_core::keychain::KeyStore;
use radar_core::model::ArticleState;
use radar_core::sync::article_states;
use radar_core::sync::last_synced::{Digests, LastSynced};
use radar_core::sync::queue::{DrainReport, QueuedReq};
use radar_core::sync::resource::SyncedResource;
use radar_core::sync::resources::{
    ArchivesRes, CategoriesRes, PrefsRes, ProfileRes, SourcesRes, ARCHIVE_CAP,
};
use radar_core::sync::{SyncClient, SyncError, SyncStatus};
use radar_core::types::{Account, Archive, Article, Category, Prefs, Profile, Source};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Debounce between the last model change and the sync push (JS 800 ms).
pub const PUSH_DEBOUNCE: Duration = Duration::from_millis(800);
/// How often the queue is drained while it holds items, and failed pulls
/// are retried.
pub const DRAIN_INTERVAL: Duration = Duration::from_secs(30);
/// How long the browser sign-in may take.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Opens the authorize URL in the browser.
pub type OpenUrl = Box<dyn FnOnce(&str) + Send>;
/// Resolves when the user cancels the sign-in (or its sender is dropped).
pub type SignInCancel = oneshot::Receiver<()>;
/// The browser sign-in: `Session::sign_in` in the app, injectable in tests.
/// The returned future runs on the tokio runtime. On the cancel signal it
/// should stop the flow (freeing the loopback port) with
/// `AuthError::Cancelled`; the app doesn't wait for it either way.
pub type SignInFn = Arc<
    dyn Fn(
            Session,
            OpenUrl,
            Duration,
            SignInCancel,
        ) -> BoxFuture<'static, Result<Account, AuthError>>
        + Send
        + Sync,
>;

/// Sync bookkeeping of the five full-list resources.
#[derive(Debug, Default)]
pub struct SyncResources {
    pub profile: SyncedResource<ProfileRes>,
    pub categories: SyncedResource<CategoriesRes>,
    pub sources: SyncedResource<SourcesRes>,
    pub prefs: SyncedResource<PrefsRes>,
    pub archives: SyncedResource<ArchivesRes>,
    /// Last-synced digests from an earlier run of this account, applied by
    /// the session's first pull ([`seed`](Self::seed)).
    seed: Digests,
}

impl SyncResources {
    /// Forgets everything learned from the server (sign-out, account switch).
    pub fn reset(&mut self) {
        self.profile.reset();
        self.categories.reset();
        self.sources.reset();
        self.prefs.reset();
        self.archives.reset();
        self.seed = Digests::default();
    }

    /// Hands the first pull the digests an earlier run persisted for this
    /// account (call after `reset`).
    fn seed(&mut self, digests: Digests) {
        self.seed = digests;
    }

    /// Seeds each resource's divergence baseline before its first pull
    /// (on tokio: it serializes the local values).
    fn apply_seed(&mut self, l: &Locals) {
        let seed = std::mem::take(&mut self.seed);
        if let Some(d) = &seed.profile {
            self.profile.seed_last_synced(d, &l.profile);
        }
        if let Some(d) = &seed.categories {
            self.categories.seed_last_synced(d, &l.categories);
        }
        if let Some(d) = &seed.sources {
            self.sources.seed_last_synced(d, &l.sources);
        }
        if let Some(d) = &seed.prefs {
            self.prefs.seed_last_synced(d, &l.prefs);
        }
        if let Some(d) = &seed.archives {
            self.archives.seed_last_synced(d, &l.archives);
        }
    }

    /// What to persist so the next run can seed (hashes the server keys).
    fn digests(&self) -> Digests {
        Digests {
            profile: self.profile.last_synced_digest(),
            categories: self.categories.last_synced_digest(),
            sources: self.sources.last_synced_digest(),
            prefs: self.prefs.last_synced_digest(),
            archives: self.archives.last_synced_digest(),
        }
    }

    pub fn all_pulled(&self) -> bool {
        self.profile.pulled
            && self.categories.pulled
            && self.sources.pulled
            && self.prefs.pulled
            && self.archives.pulled
    }

    /// A queued PUT landed: its server body updates that resource's etag and
    /// server key, so the next If-Match isn't stale (JS `onDrainSuccess`).
    /// Returns that resource's new last-synced digest.
    fn on_drained(&mut self, key: &str, body: &Value, local: &LocalRefs) -> Digests {
        let mut d = Digests::default();
        if key == SyncedResource::<ProfileRes>::group_key() {
            self.profile.on_drained(body, local.profile);
            d.profile = self.profile.last_synced_digest();
        } else if key == SyncedResource::<CategoriesRes>::group_key() {
            self.categories.on_drained(body, local.categories);
            d.categories = self.categories.last_synced_digest();
        } else if key == SyncedResource::<SourcesRes>::group_key() {
            self.sources.on_drained(body, local.sources);
            d.sources = self.sources.last_synced_digest();
        } else if key == SyncedResource::<PrefsRes>::group_key() {
            self.prefs.on_drained(body, local.prefs);
            d.prefs = self.prefs.last_synced_digest();
        } else if key == SyncedResource::<ArchivesRes>::group_key() {
            self.archives.on_drained(body, local.archives);
            d.archives = self.archives.last_synced_digest();
        }
        d
    }
}

/// The store's `sync-queue` key, kept in step with the sync client.
pub(super) struct PersistedQueue {
    /// Queue items in the JS shape (what the store holds).
    pub items: Vec<Value>,
    /// Bumped on every change.
    pub rev: u64,
    /// Only the client of this epoch may write `items`.
    pub epoch: u64,
}

impl PersistedQueue {
    pub fn new(items: Vec<Value>) -> Self {
        Self {
            items,
            rev: 0,
            epoch: 0,
        }
    }
}

/// Shared between the model (snapshots) and the client's persist callback.
pub(super) type QueueMirror = Arc<Mutex<PersistedQueue>>;

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The tokio tasks of the signed-in session (pulls with their seeding,
/// pushes, drains, article-state PATCHes). [`abort_all`](Self::abort_all)
/// runs at every account boundary, so none of them can send another request
/// for the old account (with the new account's token, since `Session`
/// clones share tokens).
#[derive(Clone, Default)]
pub(super) struct RuntimeWork(Arc<Mutex<Vec<tokio::task::AbortHandle>>>);

impl RuntimeWork {
    /// Starts `fut` on the runtime right away. Resolves to `None` if it was
    /// aborted; a panic is resumed in the awaiting task.
    pub fn spawn<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
    ) -> impl Future<Output = Option<T>> {
        let handle = radar_core::runtime().spawn(fut);
        {
            let mut tasks = lock(&self.0);
            tasks.retain(|t| !t.is_finished());
            tasks.push(handle.abort_handle());
        }
        async move {
            match handle.await {
                Ok(v) => Some(v),
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                Err(_) => None,
            }
        }
    }

    /// Cancels every task still running (each stops at its next await).
    pub fn abort_all(&self) {
        for task in lock(&self.0).drain(..) {
            task.abort();
        }
    }
}

/// Per-device prefs (`view`, `sidebarCollapsed`) never travel: keep the live
/// ones, which may have changed while the op ran (JS `setSyncedPrefs` never
/// touched them).
fn merge_prefs(synced: Prefs, live: &Prefs) -> Prefs {
    Prefs {
        view: live.view,
        sidebar_collapsed: live.sidebar_collapsed,
        ..synced
    }
}

/// Sources from the server with this device's `lastFetchAt`/`health` as
/// they are now (a briefing may have stamped them while the op ran).
fn merge_sources(mut synced: Vec<Source>, live: &[Source]) -> Vec<Source> {
    for s in &mut synced {
        if let Some(l) = live.iter().find(|l| l.id == s.id) {
            s.last_fetch_at = l.last_fetch_at;
            s.health = l.health;
        }
    }
    synced
}

fn run_at_ms(a: &Archive) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&a.run_at)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// The op's archives plus any the live model gained while it ran (a
/// briefing finished): union by id, the op's copy wins, newest `runAt`
/// first (unparseable last), capped at [`ARCHIVE_CAP`]. Without new local
/// archives this is the op's list unchanged.
fn merge_archives(synced: Vec<Archive>, live: &[Archive]) -> Vec<Archive> {
    let mut out = synced;
    let fresh: Vec<Archive> = live
        .iter()
        .filter(|l| !out.iter().any(|a| a.id == l.id))
        .cloned()
        .collect();
    if fresh.is_empty() {
        return out;
    }
    out.extend(fresh);
    out.sort_by_key(|a| std::cmp::Reverse(run_at_ms(a)));
    out.truncate(ARCHIVE_CAP);
    out
}

/// The OS keychain in the app; tests never touch it.
pub(super) fn default_key_store() -> Arc<dyn KeyStore> {
    #[cfg(test)]
    {
        Arc::new(radar_core::keychain::MemKeyStore::new())
    }
    #[cfg(not(test))]
    {
        Arc::new(radar_core::keychain::OsKeyStore)
    }
}

/// `Session::sign_in` on the runtime's blocking pool: it writes the tokens
/// to the keychain (a blocking call) while holding the session's mutex.
///
/// A cancel drops the flow inside `block_on`, which closes its loopback
/// listener (port 17823). `Session::sign_in` stores the tokens only once the
/// code exchange is done, with no await in between, so a cancel never
/// leaves half a session behind. One flow runs at a time: a new one waits
/// until a cancelled one has let go of the port.
pub(super) fn browser_sign_in() -> SignInFn {
    let one_at_a_time = Arc::new(tokio::sync::Mutex::new(()));
    Arc::new(
        move |session: Session, open: OpenUrl, timeout: Duration, cancel: SignInCancel| {
            let gate = one_at_a_time.clone();
            Box::pin(async move {
                let rt = radar_core::runtime().handle().clone();
                bridge::on_blocking(move || {
                    rt.block_on(async move {
                        let flow = async {
                            let _turn = gate.lock().await;
                            session.sign_in(open, timeout).await
                        };
                        // Cancel first: one sent before the flow starts
                        // must not open the browser.
                        tokio::select! {
                            biased;
                            _ = cancel => Err(AuthError::Cancelled),
                            result = flow => result,
                        }
                    })
                })
                .await
            })
        },
    )
}

/// Items of the persisted queue that parse; each bad one is skipped and
/// logged on its own, so one malformed entry never drops the rest. Only the
/// position and error class are logged: bodies can hold user text.
fn parse_queue(items: &[Value]) -> Vec<QueuedReq> {
    items
        .iter()
        .enumerate()
        .filter_map(
            |(i, v)| match serde_json::from_value::<QueuedReq>(v.clone()) {
                Ok(req) => Some(req),
                Err(e) => {
                    log::warn!(
                        "[sync] skipping unreadable queued item #{i} ({:?})",
                        e.classify()
                    );
                    None
                }
            },
        )
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    /// The push debounce settled (or `flush_now` found an op running).
    Push,
    /// An enqueue happened, or the window was activated.
    Drain,
    /// The 30 s timer.
    Tick,
}

/// Commands that piled up while an op ran, run once each.
#[derive(Default)]
struct Wanted {
    push: bool,
    drain: bool,
    tick: bool,
}

impl Wanted {
    fn add(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Push => self.push = true,
            Cmd::Drain => self.drain = true,
            Cmd::Tick => self.tick = true,
        }
    }
}

/// The running sync session's tasks. Dropping it cancels them.
pub(super) struct Lifecycle {
    tx: mpsc::UnboundedSender<Cmd>,
    push_debounce: Option<Task<()>>,
    _tasks: Vec<Task<()>>,
}

impl Lifecycle {
    fn send(&self, cmd: Cmd) {
        let _ = self.tx.unbounded_send(cmd);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Pull,
    Push,
}

/// Local values an op works from (taken on the UI thread at its start).
struct Locals {
    profile: Profile,
    categories: Vec<Category>,
    sources: Vec<Source>,
    prefs: Prefs,
    archives: Vec<Archive>,
    articles: Vec<Article>,
}

struct LocalRefs<'a> {
    profile: &'a Profile,
    categories: &'a Vec<Category>,
    sources: &'a Vec<Source>,
    prefs: &'a Prefs,
    archives: &'a Vec<Archive>,
}

/// `None`: not attempted. `Ok(Some(v))`: `v` replaces the local value.
type Step<T> = Option<Result<Option<T>, SyncError>>;
type States = (HashMap<String, ArticleState>, Vec<(String, ArticleState)>);

/// What one pull or push did.
#[derive(Default)]
struct Outcome {
    profile: Step<Profile>,
    categories: Step<Vec<Category>>,
    sources: Step<Vec<Source>>,
    prefs: Step<Prefs>,
    archives: Step<Vec<Archive>>,
    /// Server states and the local-only states seeded to the server.
    states: Option<Result<States, SyncError>>,
    /// The resources' last-synced digests once the op is done.
    digests: Digests,
}

/// A pull or push with the resources it borrowed; runs on tokio.
struct Job {
    op: Op,
    client: Arc<SyncClient>,
    res: SyncResources,
    locals: Locals,
    pull_states: bool,
}

impl Job {
    async fn run(self) -> (SyncResources, Outcome) {
        let Job {
            op,
            client,
            mut res,
            locals: l,
            pull_states,
        } = self;
        let c = &*client;
        let mut out = Outcome::default();
        match op {
            // The order of the `App.jsx` effects: profile, categories,
            // sources, article states, prefs, archives. Only what hasn't
            // pulled yet (a retry skips what already succeeded).
            Op::Pull => {
                // The first pull of a session: edits made since the last
                // sync of an earlier run count as offline edits.
                res.apply_seed(&l);
                if !res.profile.pulled {
                    out.profile = Some(res.profile.pull(c, &l.profile).await);
                }
                if !res.categories.pulled {
                    out.categories = Some(res.categories.pull(c, &l.categories).await);
                }
                if !res.sources.pulled {
                    out.sources = Some(res.sources.pull(c, &l.sources).await);
                }
                if pull_states {
                    out.states = Some(pull_article_states(c, &l.articles).await);
                }
                if !res.prefs.pulled {
                    out.prefs = Some(res.prefs.pull(c, &l.prefs).await);
                }
                if !res.archives.pulled {
                    out.archives = Some(res.archives.pull(c, &l.archives).await);
                }
            }
            Op::Push => {
                if res.profile.needs_push(&l.profile) {
                    out.profile = Some(res.profile.push(c, &l.profile).await);
                }
                if res.categories.needs_push(&l.categories) {
                    out.categories = Some(res.categories.push(c, &l.categories).await);
                }
                if res.sources.needs_push(&l.sources) {
                    out.sources = Some(res.sources.push(c, &l.sources).await);
                }
                if res.prefs.needs_push(&l.prefs) {
                    out.prefs = Some(res.prefs.push(c, &l.prefs).await);
                }
                if res.archives.needs_push(&l.archives) {
                    out.archives = Some(res.archives.push(c, &l.archives).await);
                }
            }
        }
        out.digests = res.digests();
        (res, out)
    }
}

/// Pull, then seed the flags the server doesn't know yet (from the local
/// articles as they were before the overlay; the server's copy wins).
async fn pull_article_states(c: &SyncClient, local: &[Article]) -> Result<States, SyncError> {
    let server = article_states::pull(c).await?;
    let seeded = article_states::seed_local_only(c, local, &server).await;
    Ok((server, seeded))
}

/// Takes the value of a step; logs a failure (status and path only).
fn value<T>(step: Step<T>, what: &str, auth_lost: &mut bool) -> Option<T> {
    match step? {
        Ok(v) => v,
        Err(e) => {
            *auth_lost |= e == SyncError::NotSignedIn;
            log::warn!("[sync] {what} sync failed: {e}");
            None
        }
    }
}

enum Begin {
    Started(Box<Job>),
    /// Another op holds the resources; retry when this fires.
    Wait(oneshot::Receiver<()>),
    /// The session this op belonged to is over.
    Stale,
}

impl AppModel {
    /// Browser sign-in through Clerk (OAuth + PKCE). Runs `sign_in_impl` on
    /// the runtime with [`SIGN_IN_TIMEOUT`]; the browser is opened from the
    /// UI thread. Success starts sync; an error is shown as a notification
    /// and the app stays signed out. [`cancel_sign_in`](Self::cancel_sign_in)
    /// abandons it.
    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.signing_in || self.account.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            log::info!("[auth] Clerk not configured; sign-in unavailable");
            return;
        };
        self.signing_in = true;
        self.sign_in_attempt += 1;
        let attempt = self.sign_in_attempt;
        let (cancel_tx, cancel) = oneshot::channel();
        self.sign_in_cancel = Some(cancel_tx);
        cx.notify();
        let (url_tx, mut url_rx) = mpsc::unbounded::<String>();
        cx.spawn(async move |_, cx| {
            while let Some(url) = url_rx.next().await {
                cx.update(|cx| cx.open_url(&url));
            }
        })
        .detach();
        let open: OpenUrl = Box::new(move |url: &str| {
            let _ = url_tx.unbounded_send(url.to_string());
        });
        let sign_in = self.sign_in_impl.clone();
        let pending_sign_out = self.sign_out_task.take();
        cx.spawn(async move |this, cx| {
            // A sign-out's keychain delete must not land after this login.
            if let Some(task) = pending_sign_out {
                task.await;
            }
            let result = bridge::on_rt(sign_in(session, open, SIGN_IN_TIMEOUT, cancel)).await;
            let _ = this.update(cx, |m, cx| m.finish_sign_in(attempt, result, cx));
        })
        .detach();
    }

    /// Abandons the running browser sign-in ("Cancel" beside "Loading
    /// auth…"): the flow is told to stop, which frees the loopback port, and
    /// the app is signed out again with no error shown. A new sign-in can
    /// start right away; the abandoned flow's result is ignored when it
    /// arrives (see [`finish_sign_in`](Self::finish_sign_in)).
    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if !self.signing_in {
            return;
        }
        self.signing_in = false;
        self.sign_in_attempt += 1;
        if let Some(cancel) = self.sign_in_cancel.take() {
            let _ = cancel.send(());
        }
        log::info!("[auth] sign-in cancelled");
        cx.notify();
    }

    fn finish_sign_in(
        &mut self,
        attempt: u64,
        result: Result<Account, AuthError>,
        cx: &mut Context<Self>,
    ) {
        let current = attempt == self.sign_in_attempt;
        if current {
            self.signing_in = false;
            self.sign_in_cancel = None;
        }
        match result {
            Ok(account) if self.account.is_none() => {
                if !current {
                    // The code exchange had already stored the tokens when
                    // Cancel was pressed. Show the session the keychain now
                    // holds (and stop a newer attempt) rather than hide it.
                    log::info!("[auth] sign-in completed as it was cancelled; keeping it");
                    self.cancel_sign_in(cx);
                }
                log::info!("[auth] signed in");
                self.account = Some(account);
                self.start_sync(cx);
            }
            Ok(_) => log::info!("[auth] extra sign-in result ignored; already signed in"),
            Err(AuthError::Cancelled) => {}
            Err(e) if current => {
                log::warn!("[auth] sign-in failed: {e}");
                self.account = None;
                show_error(cx, "Sign-in failed", e.to_string());
            }
            Err(e) => log::info!("[auth] cancelled sign-in ended: {e}"),
        }
        cx.notify();
    }

    /// Signs out: forgets the server state of every resource, drops the
    /// client and its tasks, then deletes the keychain entry off the UI
    /// thread. Local data stays.
    ///
    /// Account-boundary policy (T6): edits still inside the push debounce are
    /// not pushed (JS parity: the effect cleanup cleared the timer). A
    /// mutation that meets a rejected refresh token (`invalid_grant`) comes
    /// back `NotSignedIn` and is not queued, and `reset()` also clears
    /// `failed_pull_key`. The last-synced digests are dropped too (the next
    /// save removes `sync-state.json`). Such an edit stays local only, and
    /// the next sign-in's pull replaces it with the server copy.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.stop_sync();
        self.account = None;
        self.forget_last_synced(cx);
        cx.notify();
        let Some(session) = self.session.clone() else {
            return;
        };
        self.sign_out_task = Some(cx.spawn(async move |_, _| {
            match bridge::on_blocking(move || session.sign_out()).await {
                Ok(()) => log::info!("[auth] signed out"),
                Err(e) => log::warn!("[auth] sign-out keychain delete failed: {e}"),
            }
        }));
    }

    /// Starts sync for the signed-in account: a fresh client (with the
    /// persisted queue), then, in order, a drain of writes queued earlier, the
    /// pulls in `App.jsx` effect order, and the push/drain loops.
    ///
    /// Every start is an account boundary: whatever was learned from the
    /// previous account (server keys, etags) is reset first. The first pull
    /// is then seeded with the digests an earlier run of this same account
    /// persisted (never another account's), so an edit that never reached
    /// the server is pushed rather than replaced.
    pub fn start_sync(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(account_id) = self.account.as_ref().map(|a| a.id.clone()) else {
            return;
        };
        self.stop_sync();
        let seed = LastSynced::seed_for(self.last_synced.take(), &account_id);
        self.resources.seed(seed.clone());
        self.last_synced = Some(LastSynced {
            account_id,
            digests: seed,
            // Set by each save from the snapshot it writes.
            saved: Digests::default(),
        });
        let epoch = self.sync_epoch;
        let (changed_tx, mut changed_rx) = mpsc::unbounded::<()>();
        let client = Arc::new(self.build_client(session, epoch, changed_tx));
        self.sync = Some(client.clone());
        self.sync_status = client.status().borrow().clone();
        let (tx, rx) = mpsc::unbounded();
        let status = client.status();
        let worker_client = client.clone();
        let tasks = vec![
            cx.spawn(async move |this, cx| worker(this, cx, epoch, worker_client, rx).await),
            cx.spawn(async move |this, cx| status_loop(this, cx, epoch, status).await),
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(DRAIN_INTERVAL).await;
                if this.update(cx, |m, _| m.send(epoch, Cmd::Tick)).is_err() {
                    return;
                }
            }),
            // The queue changed: save it with the rest of the store.
            cx.spawn(async move |this, cx| {
                while changed_rx.next().await.is_some() {
                    let ok = this.update(cx, |m, cx| {
                        if m.sync_epoch == epoch {
                            m.schedule_save(cx);
                        }
                    });
                    if ok.is_err() {
                        return;
                    }
                }
            }),
        ];
        self.lifecycle = Some(Lifecycle {
            tx,
            push_debounce: None,
            _tasks: tasks,
        });
        cx.notify();
    }

    fn build_client(
        &mut self,
        session: Session,
        epoch: u64,
        changed: mpsc::UnboundedSender<()>,
    ) -> SyncClient {
        let items = parse_queue(&lock(&self.sync_queue).items);
        let mirror = self.sync_queue.clone();
        let client = SyncClient::new(self.server_base.clone(), session)
            .with_queue(items)
            .with_persist(Box::new(move |items: &[QueuedReq]| {
                // Serialized before locking: the UI thread takes this lock
                // for every store snapshot.
                let values: Vec<Value> = items
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect();
                let mut q = lock(&mirror);
                // A client from an earlier session must not overwrite the
                // queue the current one restored.
                if q.epoch != epoch {
                    return;
                }
                q.items = values;
                q.rev += 1;
                drop(q);
                let _ = changed.unbounded_send(());
            }));
        match self.sync_sleep.clone() {
            Some(sleep) => client.with_sleep(sleep),
            None => client,
        }
    }

    /// Ends the sync session: cancels its tasks, drops the client and resets
    /// every resource, so nothing from this account reaches the next one.
    fn stop_sync(&mut self) {
        self.sync_epoch += 1;
        lock(&self.sync_queue).epoch = self.sync_epoch;
        // Before anything else: no request of this account may go out after
        // the boundary (a drain asleep in its backoff would wake up holding
        // the next account's token).
        self.rt_work.abort_all();
        self.lifecycle = None;
        self.sync = None;
        self.resources.reset();
        self.resources_out = false;
        self.resource_waiters.clear();
        self.drained_backlog.clear();
        self.resources_pulled = false;
        self.article_states_pulled = false;
        self.toggled_in_pull = None;
        self.sync_status = SyncStatus::default();
    }

    /// Drains the queue now (window activation).
    pub fn drain_now(&mut self) {
        let epoch = self.sync_epoch;
        self.send(epoch, Cmd::Drain);
    }

    fn send(&self, epoch: u64, cmd: Cmd) {
        if epoch != self.sync_epoch {
            return;
        }
        if let Some(lc) = &self.lifecycle {
            lc.send(cmd);
        }
    }

    /// Restarts the push debounce (every model change while signed in).
    pub(super) fn schedule_push(&mut self, cx: &mut Context<Self>) {
        let epoch = self.sync_epoch;
        let Some(lc) = self.lifecycle.as_mut() else {
            return;
        };
        lc.push_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PUSH_DEBOUNCE).await;
            let _ = this.update(cx, |m, _| m.send(epoch, Cmd::Push));
        }));
    }

    /// Writes everything that is pending, now. Called when the window closes
    /// and when the app quits (replaces `flushRegistry.js`):
    /// 1. the store write of Task 7, synchronously (unchanged semantics);
    /// 2. the sync push, skipping the debounce. It runs on tokio, and a write
    ///    that can't reach the server lands in the offline queue.
    ///
    /// The returned future resolves once the push is done and, if the push
    /// queued something, the queue is on disk. It depends only on tokio,
    /// so the quit hook can await it while GPUI blocks the main thread.
    /// When the window closes it is dropped, and the push finishes on its own.
    pub fn flush_now(&mut self, cx: &mut Context<Self>) -> impl Future<Output = ()> + 'static {
        let pushed = self.push_now(cx);
        self.flush_store();
        let rewrite = pushed.is_some().then(|| QueueRewrite {
            path: self.store_path.clone(),
            snap: self.model.to_snapshot(Vec::new()),
            mirror: self.sync_queue.clone(),
            rev: self.saved_queue_rev,
            generation: self.save_gen,
            written_gen: self.written_gen.clone(),
        });
        async move {
            if let Some(done) = pushed {
                let _ = done.await;
            }
            if let Some(rewrite) = rewrite {
                rewrite.run();
            }
        }
    }

    /// Starts a push right away. `None` when signed out, or when an op holds
    /// the resources (a push is queued behind it instead).
    fn push_now(&mut self, cx: &mut Context<Self>) -> Option<oneshot::Receiver<()>> {
        let epoch = self.sync_epoch;
        let lc = self.lifecycle.as_mut()?;
        lc.push_debounce = None;
        let job = match self.begin(epoch, Op::Push) {
            Begin::Started(job) => job,
            Begin::Wait(_) => {
                self.send(epoch, Cmd::Push);
                return None;
            }
            Begin::Stale => return None,
        };
        let (result_tx, result_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        // Detached: the quit hook waits on `done_rx`, not on a GPUI task.
        drop(self.rt_work.spawn(async move {
            let _ = result_tx.send(job.run().await);
            let _ = done_tx.send(());
        }));
        cx.spawn(async move |this, cx| {
            if let Ok((res, out)) = result_rx.await {
                let _ = this.update(cx, |m, cx| m.finish(epoch, res, out, cx));
            }
        })
        .detach();
        Some(done_rx)
    }

    /// Lends the resources to a new op, or says how to wait for them.
    fn begin(&mut self, epoch: u64, op: Op) -> Begin {
        let Some(client) = self.sync.clone().filter(|_| epoch == self.sync_epoch) else {
            return Begin::Stale;
        };
        if self.resources_out {
            let (tx, rx) = oneshot::channel();
            self.resource_waiters.push(tx);
            return Begin::Wait(rx);
        }
        self.resources_out = true;
        let pull_states = op == Op::Pull && !self.article_states_pulled;
        if pull_states {
            self.toggled_in_pull = Some(Default::default());
        }
        let m = &self.model;
        Begin::Started(Box::new(Job {
            op,
            client,
            res: std::mem::take(&mut self.resources),
            locals: Locals {
                profile: m.profile.clone(),
                categories: m.categories.clone(),
                sources: m.sources.clone(),
                prefs: m.prefs.clone(),
                archives: m.archives.clone(),
                articles: m.articles.clone(),
            },
            pull_states,
        }))
    }

    /// Takes the resources back from an op and applies what it learned.
    fn finish(&mut self, epoch: u64, res: SyncResources, mut out: Outcome, cx: &mut Context<Self>) {
        if epoch != self.sync_epoch {
            return;
        }
        self.resources = res;
        self.resources_out = false;
        for waiter in self.resource_waiters.drain(..) {
            let _ = waiter.send(());
        }
        self.note_synced(std::mem::take(&mut out.digests), cx);
        for (key, body) in std::mem::take(&mut self.drained_backlog) {
            self.route_drained(&key, &body, cx);
        }
        let auth_lost = self.apply(out, cx);
        let pulled = self.resources.all_pulled();
        if pulled != self.resources_pulled {
            self.resources_pulled = pulled;
            cx.notify();
        }
        if auth_lost {
            self.check_session(cx);
        }
    }

    /// Writes pulled or merged values into the model. Returns whether a step
    /// failed because the session is gone.
    fn apply(&mut self, out: Outcome, cx: &mut Context<Self>) -> bool {
        let mut auth_lost = false;
        let profile = value(out.profile, "profile", &mut auth_lost);
        let categories = value(out.categories, "categories", &mut auth_lost);
        let sources = value(out.sources, "sources", &mut auth_lost);
        let prefs = value(out.prefs, "prefs", &mut auth_lost);
        let archives = value(out.archives, "archives", &mut auth_lost);
        let states = match out.states {
            Some(Ok(states)) => Some(states),
            Some(Err(e)) => {
                auth_lost |= e == SyncError::NotSignedIn;
                log::warn!("[sync] article states pull failed: {e}");
                None
            }
            None => None,
        };
        // Only a pull of the states set it; its toggles are newer than the
        // pulled copy.
        let toggled = self.toggled_in_pull.take().unwrap_or_default();
        if states.is_some() {
            self.article_states_pulled = true;
        }
        let changed = profile.is_some()
            || categories.is_some()
            || sources.is_some()
            || prefs.is_some()
            || archives.is_some()
            || states.is_some();
        if changed {
            self.update_model(cx, |m| {
                if let Some(v) = profile {
                    m.profile = v;
                }
                if let Some(v) = categories {
                    m.categories = v;
                }
                // Merged with the live model: the op worked from a snapshot
                // taken when it started.
                if let Some(v) = sources {
                    m.sources = merge_sources(v, &m.sources);
                }
                if let Some(v) = prefs {
                    m.prefs = merge_prefs(v, &m.prefs);
                }
                if let Some(v) = archives {
                    m.archives = merge_archives(v, &m.archives);
                }
                // The server's rows and the seeded flags, except for the
                // articles toggled while the pull ran: their live state is
                // newer (and its PATCH is on the way).
                if let Some((server, seeded)) = states {
                    let mut next = server;
                    next.extend(seeded);
                    for id in &toggled {
                        if let Some(live) = m.article_states.get(id) {
                            next.insert(id.clone(), *live);
                        }
                    }
                    article_states::overlay(&mut m.articles, &next);
                    m.article_states = next;
                }
            });
        }
        auth_lost
    }

    fn finish_drain(&mut self, epoch: u64, report: DrainReport, cx: &mut Context<Self>) {
        if epoch != self.sync_epoch {
            return;
        }
        for (key, body) in report.succeeded {
            if self.resources_out {
                self.drained_backlog.push((key, body));
            } else {
                self.route_drained(&key, &body, cx);
            }
        }
        if report.remaining > 0 {
            // The pass may have stopped because the session ended.
            self.check_session(cx);
        }
    }

    fn route_drained(&mut self, key: &str, body: &Value, cx: &mut Context<Self>) {
        let m = &self.model;
        let local = LocalRefs {
            profile: &m.profile,
            categories: &m.categories,
            sources: &m.sources,
            prefs: &m.prefs,
            archives: &m.archives,
        };
        let digests = self.resources.on_drained(key, body, &local);
        self.note_synced(digests, cx);
    }

    /// Records what the resources last synced; a change is saved with the
    /// next store write (it must never land ahead of the store).
    fn note_synced(&mut self, digests: Digests, cx: &mut Context<Self>) {
        let Some(state) = self.last_synced.as_mut() else {
            return;
        };
        if state.digests.update(digests) {
            self.schedule_save(cx);
        }
    }

    /// Signed out: no digests are kept, and the next save removes the file.
    fn forget_last_synced(&mut self, cx: &mut Context<Self>) {
        if self.last_synced.take().is_some() {
            self.schedule_save(cx);
        }
    }

    fn on_status(&mut self, epoch: u64, status: SyncStatus, cx: &mut Context<Self>) {
        if epoch != self.sync_epoch {
            return;
        }
        let enqueued = status.pending > self.sync_status.pending;
        self.sync_status = status;
        if enqueued {
            self.send(epoch, Cmd::Drain);
        }
        cx.notify();
    }

    /// Moves to signed-out if the session ended underneath sync (a rejected
    /// refresh token signs the session out inside radar-core). The check runs
    /// off the UI thread: `Session` holds its mutex during keychain writes.
    pub(super) fn check_session(&mut self, cx: &mut Context<Self>) {
        let (Some(session), true) = (self.session.clone(), self.account.is_some()) else {
            return;
        };
        let epoch = self.sync_epoch;
        cx.spawn(async move |this, cx| {
            if bridge::on_blocking(move || session.is_signed_in()).await {
                return;
            }
            let _ = this.update(cx, |m, cx| {
                if m.sync_epoch == epoch && m.account.is_some() {
                    log::info!("[auth] session ended; now signed out");
                    m.stop_sync();
                    m.account = None;
                    m.forget_last_synced(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn pending(&self) -> usize {
        self.sync
            .as_ref()
            .map_or(0, |c| c.status().borrow().pending)
    }

    fn wants_pull(&self) -> bool {
        !self.resources.all_pulled() || !self.article_states_pulled
    }
}

/// Runs one op to completion, waiting for the resources if they are lent.
async fn run_op(this: &WeakEntity<AppModel>, cx: &mut AsyncApp, epoch: u64, op: Op) {
    loop {
        let Ok((begin, rt)) = this.update(cx, |m, _| (m.begin(epoch, op), m.rt_work.clone()))
        else {
            return;
        };
        match begin {
            Begin::Started(job) => {
                // `None`: aborted at an account boundary.
                if let Some((res, out)) = rt.spawn(job.run()).await {
                    let _ = this.update(cx, |m, cx| m.finish(epoch, res, out, cx));
                }
                return;
            }
            Begin::Wait(rx) => {
                let _ = rx.await;
            }
            Begin::Stale => return,
        }
    }
}

async fn run_drain(
    this: &WeakEntity<AppModel>,
    cx: &mut AsyncApp,
    epoch: u64,
    c: &Arc<SyncClient>,
) {
    let Ok(rt) = this.update(cx, |m, _| m.rt_work.clone()) else {
        return;
    };
    let client = c.clone();
    if let Some(report) = rt.spawn(async move { client.drain().await }).await {
        let _ = this.update(cx, |m, cx| m.finish_drain(epoch, report, cx));
    }
}

/// The session's worker: one op at a time, in this order on start:
/// 1. drain writes queued earlier (an offline edit from a previous run must
///    land before the pulls read the server, or the pull would replace it);
/// 2. pull everything;
/// 3. then serve commands: drains (and pull retries) before pushes.
async fn worker(
    this: WeakEntity<AppModel>,
    cx: &mut AsyncApp,
    epoch: u64,
    client: Arc<SyncClient>,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
) {
    if client.status().borrow().pending > 0 {
        run_drain(&this, cx, epoch, &client).await;
    }
    run_op(&this, cx, epoch, Op::Pull).await;
    while let Some(first) = rx.next().await {
        let mut wanted = Wanted::default();
        wanted.add(first);
        while let Ok(cmd) = rx.try_recv() {
            wanted.add(cmd);
        }
        if wanted.drain || wanted.tick {
            let Ok((pending, retry)) = this.update(cx, |m, _| (m.pending(), m.wants_pull())) else {
                return;
            };
            if wanted.drain || pending > 0 {
                run_drain(&this, cx, epoch, &client).await;
            }
            if retry {
                run_op(&this, cx, epoch, Op::Pull).await;
            }
        }
        if wanted.push {
            run_op(&this, cx, epoch, Op::Push).await;
        }
    }
}

/// Mirrors the client's status into the model; an enqueue (pending grew)
/// asks the worker to drain.
async fn status_loop(
    this: WeakEntity<AppModel>,
    cx: &mut AsyncApp,
    epoch: u64,
    mut rx: tokio::sync::watch::Receiver<SyncStatus>,
) {
    loop {
        let (back, changed) = bridge::on_rt(async move {
            let changed = rx.changed().await.is_ok();
            (rx, changed)
        })
        .await;
        rx = back;
        if !changed {
            return;
        }
        let status = rx.borrow_and_update().clone();
        if this
            .update(cx, |m, cx| m.on_status(epoch, status, cx))
            .is_err()
        {
            return;
        }
    }
}

/// The quit flush's second write: if the push queued something after the
/// store was written, write the store again with the new queue.
struct QueueRewrite {
    path: std::path::PathBuf,
    snap: radar_core::store::Snapshot,
    mirror: QueueMirror,
    rev: u64,
    /// The save generation `snap` belongs to (what the flush wrote).
    generation: u64,
    written_gen: Arc<Mutex<u64>>,
}

impl QueueRewrite {
    fn run(mut self) {
        let items = {
            let q = lock(&self.mirror);
            if q.rev == self.rev {
                return;
            }
            q.items.clone()
        };
        self.snap.sync_queue = items;
        let written = lock(&self.written_gen);
        // The app kept running during the push (the updater awaits it) and
        // a newer save landed: `snap` would overwrite it. That save, or the
        // one the queue change schedules, carries the queue.
        if *written > self.generation {
            return;
        }
        if let Err(e) = radar_core::store::save(&self.path, &self.snap) {
            log::error!(
                "[store] final queue save to {} failed: {e}",
                self.path.display()
            );
        }
    }
}

/// An error toast in every open window (logged only when none is open).
fn show_error(cx: &mut App, title: &'static str, message: String) {
    for window in cx.windows() {
        let note = Notification::error(message.clone()).title(title);
        let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
    }
}

#[cfg(test)]
#[path = "sync_lifecycle_tests.rs"]
mod sync_lifecycle_tests;
