//! The global application model: one `Entity<AppModel>` owns all mutable
//! state (port of the state that lived in `App.jsx`) and persists it.
//!
//! # Pattern for later tasks
//! - Get the entity anywhere with [`AppModel::global`]; views `cx.observe`
//!   it to re-render.
//! - Every change to [`RadarModel`] goes through [`AppModel::update_model`]
//!   (views and click handlers use [`AppModel::mutate`], which calls it on
//!   the global entity). It notifies observers, re-applies theme tokens when the visual prefs
//!   changed, and (re)schedules one debounced save ([`SAVE_DEBOUNCE`],
//!   matching plugin-store's 100 ms autoSave). Never mutate `model` directly.
//! - Hydration ([`AppModel::new`]) only reads the store; `ready` flips true
//!   once it is done, and saves are gated on it.
//! - Sign-in, sign-out and the live sync loop live in `sync_lifecycle.rs`
//!   (a child module, so it shares this struct's private state). Every
//!   [`update_model`](AppModel::update_model) also (re)starts the 800 ms
//!   sync push debounce while signed in.
//! - Everything outside the process (briefing pipeline, clock, system
//!   notification, tray, OS wake entry, login item) goes through
//!   [`Hooks`]. The model starts with [`Hooks::inert`]; `main.rs` installs
//!   the real ones with [`AppModel::set_hooks`]. Tests install fakes.

use crate::platform::{autostart, tray};
use crate::scheduler::{self, WakeFn};
use crate::{bridge, theme};
use chrono::{DateTime, Local};
use futures::future::BoxFuture;
use gpui_kit::{App, AppContext, Context, Entity, Global, Task};
use radar_core::auth::Session;
use radar_core::ingest::{self, CategoryInput, SourceInput};
use radar_core::keychain::KeyStore;
use radar_core::model::{ArticleState, BriefingOutcome, RadarModel};
use radar_core::store;
use radar_core::sync::queue::SleepFn;
use radar_core::sync::{article_states, SyncClient, SyncStatus};
use radar_core::types::{Account, BriefingResponse};
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "sync_lifecycle.rs"]
mod sync_lifecycle;

#[cfg(test)]
pub use sync_lifecycle::PUSH_DEBOUNCE;
use sync_lifecycle::{Lifecycle, PersistedQueue, QueueMirror, RuntimeWork};
pub use sync_lifecycle::{SignInFn, SyncResources};

/// Delay between the last mutation and the write to disk.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);
/// First retry delay after a failed write; doubles up to [`SAVE_RETRY_MAX`].
pub const SAVE_RETRY_MIN: Duration = Duration::from_secs(1);
pub const SAVE_RETRY_MAX: Duration = Duration::from_secs(60);

/// Runs a briefing: `(sources, categories, lens)` to the response. The
/// returned future runs on the tokio runtime.
pub type BriefingFn = Arc<
    dyn Fn(
            Vec<SourceInput>,
            Vec<CategoryInput>,
            Option<String>,
        ) -> BoxFuture<'static, Result<BriefingResponse, String>>
        + Send
        + Sync,
>;

/// Posts the "briefing ready" notification for a finished run.
pub type NotifyFn = Rc<dyn Fn(&mut App, BriefingOutcome)>;

/// What the model reaches outside the process through, injected so tests
/// never touch the OS or the network.
#[derive(Clone)]
pub struct Hooks {
    /// The ingest pipeline ([`ingest_runner`] in the app).
    pub briefing: BriefingFn,
    /// Local wall clock: the scheduler's "now" and the stamps a finished
    /// briefing writes.
    pub clock: Rc<dyn Fn() -> DateTime<Local>>,
    /// "Radar briefing ready" (`notify::briefing_ready`).
    pub notify: NotifyFn,
    /// Shows `(title, tooltip)` on the tray icon (`tray::set_status`).
    pub tray: Rc<dyn Fn(Option<String>, String)>,
    /// Syncs the OS wake entry (`schedule_wake::sync_schedule_wake`);
    /// `None` leaves it alone.
    pub wake: Option<WakeFn>,
    /// The login item (`platform::autostart`).
    pub autostart: autostart::Backend,
}

impl Hooks {
    /// Nothing leaves the process: briefings come back empty, and the
    /// notification, tray and wake entry are skipped. The login item reads
    /// as off. The clock is the real one.
    pub fn inert() -> Self {
        Self {
            briefing: Arc::new(|_, _, _| {
                Box::pin(async {
                    Ok(BriefingResponse {
                        articles: Vec::new(),
                        errors: Vec::new(),
                        fetched_at: chrono::Utc::now(),
                    })
                })
            }),
            clock: Rc::new(Local::now),
            notify: Rc::new(|_, _| {}),
            tray: Rc::new(|_, _| {}),
            wake: None,
            autostart: autostart::Backend::off(),
        }
    }
}

/// The real pipeline, `ingest::run_briefing`: it fetches every feed and,
/// with categories set, reads the API key from the OS keychain.
pub fn ingest_runner() -> BriefingFn {
    Arc::new(|sources, categories, lens| Box::pin(ingest::run_briefing(sources, categories, lens)))
}

pub struct AppModel {
    pub model: RadarModel,
    /// `None` when Clerk isn't configured (`AuthConfig::from_env()` is
    /// `None`); the app then runs fully local.
    pub session: Option<Session>,
    pub account: Option<Account>,
    /// The signed-in sync client (`None` when signed out). `Arc` so async
    /// work can hold it past a borrow.
    pub sync: Option<Arc<SyncClient>>,
    /// Latest queue status, for the sync indicator.
    pub sync_status: SyncStatus,
    /// True once hydration finished; saves are skipped before that.
    pub ready: bool,
    /// True once all five synced resources (profile, categories, sources,
    /// archives, prefs) have pulled for the signed-in account; cleared on
    /// sign-out and account switch.
    pub resources_pulled: bool,
    pub store_path: PathBuf,
    /// Sync bookkeeping (pulled / server key / etag) of the five resources.
    pub resources: SyncResources,
    /// Article states pulled (and overlaid) for the signed-in account.
    pub article_states_pulled: bool,
    /// A browser sign-in is running.
    pub signing_in: bool,
    /// Where the Anthropic API key lives: the OS keychain (an in-memory
    /// store under `cfg(test)`).
    pub key_store: Arc<dyn KeyStore>,
    /// radar-server base URL (`radar_core::sync::server_url()`).
    pub server_base: String,
    /// The browser sign-in (`Session::sign_in`); tests inject failures.
    pub sign_in_impl: SignInFn,
    /// OS and pipeline integrations ([`Hooks::inert`] until `main.rs`
    /// installs the real ones).
    pub hooks: Hooks,
    /// The tray title and tooltip last pushed.
    tray_shown: Option<tray::Status>,
    /// Daily-slot, run-at-launch and wake-sync bookkeeping.
    pub(crate) schedule: scheduler::State,
    /// Backoff sleep handed to the sync client; tests use an instant one.
    pub sync_sleep: Option<SleepFn>,
    /// Persisted offline sync queue (the store's `sync-queue` key, opaque JS
    /// shape), shared with the sync client's persist callback.
    sync_queue: QueueMirror,
    /// `sync_queue` revision included in the newest snapshot handed to a
    /// writer.
    saved_queue_rev: u64,
    /// Tasks and channel of the running sync session.
    lifecycle: Option<Lifecycle>,
    /// Bumped on every sign-in, sign-out and account switch; results of
    /// work started under an older epoch are dropped.
    sync_epoch: u64,
    /// `resources` are lent to a running pull or push.
    resources_out: bool,
    /// Ops waiting for `resources` to come back.
    resource_waiters: Vec<futures::channel::oneshot::Sender<()>>,
    /// Drained queue bodies that arrived while `resources` were lent out.
    drained_backlog: Vec<(String, Value)>,
    /// The keychain delete of the last sign-out (a new sign-in waits on it).
    sign_out_task: Option<Task<()>>,
    /// Tokio work of the signed-in session, aborted at account boundaries.
    rt_work: RuntimeWork,
    /// The pending debounced save; replacing it cancels the previous one.
    save_task: Option<Task<()>>,
    /// Unsaved changes exist (cleared when a snapshot is taken for writing).
    dirty: bool,
    /// Monotonic id of the newest snapshot handed to a writer.
    save_gen: u64,
    /// Serializes writers and records the newest generation written, so an
    /// older snapshot can never land after a newer one.
    written_gen: Arc<Mutex<u64>>,
}

struct GlobalAppModel(Entity<AppModel>);

impl Global for GlobalAppModel {}

impl AppModel {
    /// Hydrates from `store_path`. Reads only; never writes the store.
    pub fn new(store_path: PathBuf, session: Option<Session>, _cx: &mut Context<Self>) -> Self {
        let snap = store::load(&store_path);
        let sync_queue = Arc::new(Mutex::new(PersistedQueue::new(snap.sync_queue.clone())));
        let model = RadarModel::from_snapshot(snap);
        // Startup only: nothing else uses the session yet, so this can't
        // wait on a keychain write.
        let account = session.as_ref().and_then(Session::account);
        Self {
            model,
            session,
            account,
            sync: None,
            sync_status: SyncStatus::default(),
            ready: true,
            resources_pulled: false,
            store_path,
            resources: SyncResources::default(),
            article_states_pulled: false,
            signing_in: false,
            key_store: sync_lifecycle::default_key_store(),
            server_base: radar_core::sync::server_url(),
            sign_in_impl: sync_lifecycle::browser_sign_in(),
            hooks: Hooks::inert(),
            tray_shown: None,
            schedule: scheduler::State::default(),
            sync_sleep: None,
            sync_queue,
            saved_queue_rev: 0,
            lifecycle: None,
            sync_epoch: 0,
            resources_out: false,
            resource_waiters: Vec::new(),
            drained_backlog: Vec::new(),
            sign_out_task: None,
            rt_work: RuntimeWork::default(),
            save_task: None,
            dirty: false,
            save_gen: 0,
            written_gen: Arc::new(Mutex::new(0)),
        }
    }

    /// Creates the global entity, applies the persisted theme, and flushes
    /// pending writes (store and sync) on quit. Call once at startup.
    pub fn init(store_path: PathBuf, session: Option<Session>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| {
            cx.on_app_quit(|this: &mut Self, cx| this.flush_now(cx))
                .detach();
            Self::new(store_path, session, cx)
        });
        let prefs = entity.read(cx).model.prefs.clone();
        theme::apply(cx, &prefs);
        cx.set_global(GlobalAppModel(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalAppModel>().0.clone()
    }

    /// [`update_model`](Self::update_model) on the global entity; the usual
    /// call from views and click handlers.
    pub fn mutate<R>(cx: &mut App, f: impl FnOnce(&mut RadarModel) -> R) -> R {
        Self::global(cx).update(cx, |m, cx| m.update_model(cx, f))
    }

    /// The one write path for [`RadarModel`].
    pub fn update_model<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut RadarModel) -> R,
    ) -> R {
        let visual = |p: &radar_core::types::Prefs| (p.theme, p.accent, p.density);
        let before = visual(&self.model.prefs);
        let out = f(&mut self.model);
        if visual(&self.model.prefs) != before {
            let prefs = self.model.prefs.clone();
            theme::apply(cx, &prefs);
        }
        cx.notify();
        self.schedule_save(cx);
        self.schedule_push(cx);
        self.sync_platform(cx);
        out
    }

    /// Installs `hooks` (`main.rs`: the real integrations), then brings the
    /// tray and the OS wake entry in step with the model, as the JS effects
    /// did once `ready` was set.
    pub fn set_hooks(&mut self, hooks: Hooks, cx: &mut Context<Self>) {
        self.hooks = hooks;
        self.tray_shown = None;
        self.schedule.forget_wake();
        self.sync_platform(cx);
    }

    /// Local wall-clock time from the injected clock.
    pub fn now(&self) -> DateTime<Local> {
        (self.hooks.clock)()
    }

    /// Once ready, after every change: the tray shows the current status
    /// (pushed only when it changed: toggles, briefings, archive pulls),
    /// and the OS wake entry follows the schedule prefs.
    fn sync_platform(&mut self, cx: &mut Context<Self>) {
        if !self.ready {
            return;
        }
        let status = tray::status(&self.model, self.now().timestamp_millis());
        if self.tray_shown.as_ref() != Some(&status) {
            (self.hooks.tray)(status.0.clone(), status.1.clone());
            self.tray_shown = Some(status);
        }
        if let Some(wake) = &self.hooks.wake {
            self.schedule.sync_wake(&self.model.prefs, wake, cx);
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.ready {
            return;
        }
        self.dirty = true;
        // Dropping the previous task cancels its pending timer (or its
        // not-yet-started write; `flush_store` and the newer task cover that,
        // because a taken-but-unwritten generation keeps `save_gen` ahead of
        // `written_gen`, and `dirty` is true again).
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let mut delay = SAVE_DEBOUNCE;
            loop {
                cx.background_executor().timer(delay).await;
                let Ok(Some(job)) = this.update(cx, |this, _| this.take_save_job()) else {
                    return;
                };
                let path = job.path.clone();
                match cx.background_spawn(async move { job.run() }).await {
                    Ok(()) => return,
                    Err(e) => {
                        delay = next_retry(delay);
                        log::error!(
                            "[store] save to {} failed: {e}; retrying in {delay:?}",
                            path.display()
                        );
                        if this.update(cx, |this, _| this.dirty = true).is_err() {
                            return;
                        }
                    }
                }
            }
        }));
    }

    fn take_save_job(&mut self) -> Option<SaveJob> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        self.save_gen += 1;
        let queue = {
            let q = sync_lifecycle::lock(&self.sync_queue);
            self.saved_queue_rev = q.rev;
            q.items.clone()
        };
        Some(SaveJob {
            path: self.store_path.clone(),
            snap: self.model.to_snapshot(queue),
            generation: self.save_gen,
            written_gen: self.written_gen.clone(),
        })
    }

    /// Runs one briefing. The single path for the header button, the tray,
    /// run-at-launch and the scheduler; it works with no window open, and
    /// does nothing while a briefing is already running.
    /// 1. `begin_briefing`.
    /// 2. The injected runner (`ingest::run_briefing`) on tokio.
    /// 3. `finish_briefing` (or `fail_briefing`), stamped by the injected
    ///    clock. As a model change it is saved and refreshes the tray.
    /// 4. The "briefing ready" notification, when the run brought new
    ///    articles and no Radar window has focus.
    pub fn run_briefing(&mut self, cx: &mut Context<Self>) {
        if self.model.scanning {
            return;
        }
        let (sources, categories, lens, attempted) = self.model.briefing_inputs();
        log::info!("[briefing] started ({} sources)", sources.len());
        self.update_model(cx, RadarModel::begin_briefing);
        let run = (self.hooks.briefing)(sources, categories, lens);
        cx.spawn(async move |this, cx| {
            let result = bridge::on_rt(run).await;
            let _ = this.update(cx, |this, cx| {
                let now_ms = this.now().timestamp_millis();
                let outcome = this.update_model(cx, |m| match result {
                    Ok(resp) => {
                        let out = m.finish_briefing(resp, &attempted, now_ms);
                        log::info!(
                            "[briefing] done: {} new / {} total, {} errors",
                            out.new_count,
                            out.total,
                            m.briefing_errors.len()
                        );
                        Some(out)
                    }
                    Err(e) => {
                        log::error!("[briefing] failed: {e}");
                        m.fail_briefing(&e);
                        None
                    }
                });
                let fresh = outcome.filter(|o| o.new_count > 0);
                if let Some(out) = fresh.filter(|_| crate::active_radar_window(cx).is_none()) {
                    (this.hooks.notify)(cx, out);
                }
            });
        })
        .detach();
    }

    /// Fully hydrated: local disk loaded and, when signed in, every synced
    /// resource pulled (`hydrated` in `App.jsx`). Gates first-run UI so the
    /// tutorial doesn't flash between local load and the server data.
    pub fn hydrated(&self) -> bool {
        self.ready && (self.account.is_none() || self.resources_pulled)
    }

    /// Flips the read flag of an article and PATCHes it when signed in.
    pub fn toggle_read(&mut self, cx: &mut Context<Self>, id: u64) {
        let out = self.update_model(cx, |m| m.toggle_read(id));
        self.patch_article_state(cx, out);
    }

    /// Flips the bookmark flag of an article and PATCHes it when signed in.
    pub fn toggle_bookmark(&mut self, cx: &mut Context<Self>, id: u64) {
        let out = self.update_model(cx, |m| m.toggle_bookmark(id));
        self.patch_article_state(cx, out);
    }

    /// One toggle, one PATCH. The request runs on the tokio runtime (never
    /// the UI thread) and is aborted at an account boundary;
    /// `SyncClient::mutate` queues it on a transient failure. Without a sync
    /// client (signed out) nothing leaves the app.
    fn patch_article_state(&self, cx: &mut Context<Self>, out: Option<(String, ArticleState)>) {
        let (Some(client), Some((id, state))) = (self.sync.clone(), out) else {
            return;
        };
        let res = self
            .rt_work
            .spawn(async move { article_states::patch(&client, &id, state).await });
        cx.spawn(async move |this, cx| match res.await {
            None | Some(Ok(_)) => {}
            Some(Err(radar_core::sync::SyncError::NotSignedIn)) => {
                let _ = this.update(cx, |m, cx| m.check_session(cx));
            }
            Some(Err(e)) => log::warn!("[sync] article state patch failed: {e}"),
        })
        .detach();
    }

    /// Writes everything not yet on disk, synchronously (Task 7's quit
    /// flush, also run by [`flush_now`](Self::flush_now)).
    ///
    /// It runs inside the quit callback, so by the time the quit future is
    /// returned there is nothing left pending: GPUI only polls quit futures
    /// for `SHUTDOWN_TIMEOUT` (200 ms), too short to trust with a large
    /// archive write.
    fn flush_store(&mut self) {
        // Cancels a pending timer or a write that hasn't started yet.
        self.save_task = None;
        let written_gen = self.written_gen.clone();
        // Waits for a write already running on the background executor.
        let mut written = lock(&written_gen);
        if self.save_gen > *written {
            // A taken snapshot never landed (cancelled or failed).
            self.dirty = true;
        }
        if let Some(job) = self.take_save_job() {
            if let Err(e) = job.run_locked(&mut written) {
                log::error!("[store] final save to {} failed: {e}", job.path.display());
            }
        }
    }
}

fn lock(m: &Mutex<u64>) -> std::sync::MutexGuard<'_, u64> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn next_retry(prev: Duration) -> Duration {
    if prev < SAVE_RETRY_MIN {
        SAVE_RETRY_MIN
    } else {
        (prev * 2).min(SAVE_RETRY_MAX)
    }
}

struct SaveJob {
    path: PathBuf,
    snap: store::Snapshot,
    generation: u64,
    written_gen: Arc<Mutex<u64>>,
}

impl SaveJob {
    /// Writes unless a newer generation already landed.
    fn run(&self) -> std::io::Result<()> {
        let written_gen = self.written_gen.clone();
        let mut written = lock(&written_gen);
        self.run_locked(&mut written)
    }

    fn run_locked(&self, written: &mut u64) -> std::io::Result<()> {
        if self.generation <= *written {
            return Ok(());
        }
        store::save(&self.path, &self.snap)?;
        *written = self.generation;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{at, faked_model, wait_until, Fakes};
    use crate::workspace::shell_tests::article;
    use gpui_kit::TestAppContext;
    use std::path::Path;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../radar-core/tests/fixtures/tauri_radar.json")
    }

    fn add_one(model: &Entity<AppModel>, cx: &mut TestAppContext) {
        model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                r.add_source(
                    "Feed".into(),
                    "packaging".into(),
                    Some("https://x.test/feed".into()),
                )
            });
        });
    }

    fn sources_on_disk(p: &Path) -> usize {
        store::load(p).sources.len()
    }

    #[gpui_kit::test]
    fn mutation_persists_after_debounce(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));

        add_one(&model, cx);
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        assert!(!path.exists(), "saved before the debounce elapsed");

        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(sources_on_disk(&path), 1);
    }

    #[gpui_kit::test]
    fn rapid_mutations_coalesce_into_one_later_save(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));

        add_one(&model, cx);
        cx.executor().advance_clock(Duration::from_millis(80));
        cx.run_until_parked();
        add_one(&model, cx);
        // 160 ms after the first edit but only 80 ms after the second.
        cx.executor().advance_clock(Duration::from_millis(80));
        cx.run_until_parked();
        assert!(!path.exists(), "debounce must restart on every mutation");

        cx.executor().advance_clock(Duration::from_millis(30));
        cx.run_until_parked();
        assert_eq!(sources_on_disk(&path), 2);
    }

    #[gpui_kit::test]
    fn quit_flushes_pending_save(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));
        add_one(&model, cx);
        assert!(!path.exists());
        cx.quit();
        assert_eq!(sources_on_disk(&path), 1);
    }

    /// The debounced task took the snapshot, then got cancelled before its
    /// background write ran. GPUI's test scheduler can't stop between those
    /// two steps (`advance_clock` drains every runnable task once the timer
    /// fires), so the test takes and drops the job at the same seam.
    #[gpui_kit::test]
    fn quit_flush_writes_job_taken_but_cancelled(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));
        add_one(&model, cx);
        model.update(cx, |m, _| {
            let job = m.take_save_job();
            assert!(job.is_some());
            m.save_task = None; // cancelled: the job never runs
        });
        assert!(!path.exists());
        cx.quit();
        assert_eq!(sources_on_disk(&path), 1);
    }

    #[gpui_kit::test]
    fn failed_save_is_retried(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));
        // A directory where the file belongs makes store::save fail.
        std::fs::create_dir(&path).unwrap();

        add_one(&model, cx);
        cx.executor().advance_clock(SAVE_DEBOUNCE);
        cx.run_until_parked();
        assert!(path.is_dir(), "first attempt must have failed");

        std::fs::remove_dir(&path).unwrap();
        cx.executor().advance_clock(SAVE_RETRY_MIN);
        cx.run_until_parked();
        assert_eq!(sources_on_disk(&path), 1);
    }

    #[gpui_kit::test]
    fn quit_after_failed_save_writes_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));
        std::fs::create_dir(&path).unwrap();
        add_one(&model, cx);
        cx.executor().advance_clock(SAVE_DEBOUNCE);
        cx.run_until_parked();
        assert!(path.is_dir());
        std::fs::remove_dir(&path).unwrap();
        cx.quit(); // before the retry timer fires
        assert_eq!(sources_on_disk(&path), 1);
    }

    #[gpui_kit::test]
    fn hydrate_does_not_overwrite_store(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        std::fs::copy(fixture(), &path).unwrap();
        let before = std::fs::read(&path).unwrap();

        let model = cx.update(|cx| AppModel::init(path.clone(), None, cx));
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();

        assert!(model.read_with(cx, |m, _| m.ready));
        assert_eq!(model.read_with(cx, |m, _| m.model.sources.len()), 2);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[gpui_kit::test]
    fn hydrate_applies_persisted_theme_tokens(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        std::fs::copy(fixture(), &path).unwrap();
        cx.update(|cx| AppModel::init(path, None, cx));
        // Fixture: dark / Olive / compact.
        let want = theme::tokens(
            radar_core::types::Theme::Dark,
            radar_core::types::Accent::Olive,
            radar_core::types::Density::Compact,
        );
        cx.read(|cx| assert_eq!(*theme::t(cx), want));
    }

    #[gpui_kit::test]
    fn pref_change_reapplies_tokens(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = cx.update(|cx| AppModel::init(dir.path().join("radar.json"), None, cx));
        model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                r.prefs.density = radar_core::types::Density::Compact
            })
        });
        cx.read(|cx| assert_eq!(theme::t(cx).row_h, gpui_kit::px(36.)));
    }

    fn two_articles() -> Vec<radar_core::types::Article> {
        vec![article(1, "tools", false), article(2, "tools", false)]
    }

    fn run_to_end(model: &Entity<AppModel>, cx: &mut TestAppContext) {
        model.update(cx, |m, cx| m.run_briefing(cx));
        wait_until(cx, |cx| !model.read_with(cx, |m, _| m.model.scanning));
    }

    /// A window that isn't the main window, for focus tests.
    struct Blank;

    impl gpui_kit::Render for Blank {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }

    /// Review Focus 2: the window is closed while a briefing runs (or the
    /// scheduler fires). The model still updates, persists and notifies.
    #[gpui_kit::test]
    fn briefing_completes_with_no_window(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = faked_model(cx, &dir, &fakes, two_articles(), at(9, 0));
        assert!(cx.windows().is_empty());

        run_to_end(&model, cx);
        model.read_with(cx, |m, _| {
            assert_eq!(m.model.articles.len(), 2);
            assert_eq!(m.model.archives.len(), 1);
            assert_eq!(m.model.last_run_at_ms(), Some(at(9, 0).timestamp_millis()));
        });
        cx.executor().advance_clock(SAVE_DEBOUNCE);
        cx.run_until_parked();
        let saved = store::load(&dir.path().join("radar.json"));
        assert_eq!(saved.articles.len(), 2);
        assert_eq!(saved.archives.len(), 1);
        assert_eq!(fakes.runs(), 1);
        assert_eq!(
            *fakes.notified.borrow(),
            vec![BriefingOutcome {
                new_count: 2,
                total: 2,
                source_count: 1
            }]
        );
    }

    /// Review Focus 2, the other half: the main window is open (and
    /// focused) when the run starts and the user closes it before the
    /// runner answers. The result still lands, persists and notifies, and
    /// a reopened window shows it.
    #[gpui_kit::test]
    fn briefing_completes_after_its_window_closes(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        cx.update(gpui_kit::init);
        let model = faked_model(cx, &dir, &fakes, Vec::new(), at(9, 0));
        cx.update(crate::workspace::init);
        // The runner answers only once the test releases it.
        let (release, gate) = async_channel::bounded::<()>(1);
        let resp = BriefingResponse {
            articles: two_articles(),
            errors: Vec::new(),
            fetched_at: chrono::Utc::now(),
        };
        model.update(cx, |m, _| {
            m.hooks.briefing = Arc::new(move |_, _, _| {
                let (gate, resp) = (gate.clone(), resp.clone());
                Box::pin(async move {
                    let _ = gate.recv().await;
                    Ok(resp)
                })
            })
        });
        cx.update(crate::open_main_window);
        let window = cx.windows()[0];
        cx.update_window(window, |_, w, _| w.activate_window())
            .unwrap();
        cx.run_until_parked();

        model.update(cx, |m, cx| m.run_briefing(cx));
        assert!(model.read_with(cx, |m, _| m.model.scanning));
        // The user closes it: the app's should-close handler runs, then the
        // platform removes the window.
        assert!(gpui_kit::VisualTestContext::from_window(window, cx).simulate_close());
        cx.update_window(window, |_, w, _| w.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert!(cx.windows().is_empty());

        release.try_send(()).unwrap();
        wait_until(cx, |cx| !model.read_with(cx, |m, _| m.model.scanning));
        assert_eq!(model.read_with(cx, |m, _| m.model.articles.len()), 2);
        cx.executor().advance_clock(SAVE_DEBOUNCE);
        cx.run_until_parked();
        assert_eq!(
            store::load(&dir.path().join("radar.json")).articles.len(),
            2
        );
        assert_eq!(fakes.notified.borrow().len(), 1, "no window has focus now");

        cx.update(crate::open_main_window);
        let reopened = cx.windows()[0];
        cx.update_window(reopened, |_, w, cx| {
            w.render_frame(cx);
            assert!(w.try_find("card-1").is_some() && w.try_find("card-2").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn run_briefing_ignored_while_scanning(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = faked_model(cx, &dir, &fakes, two_articles(), at(9, 0));
        model.update(cx, |m, cx| {
            m.run_briefing(cx);
            m.run_briefing(cx);
        });
        assert_eq!(fakes.runs(), 1);
        wait_until(cx, |cx| !model.read_with(cx, |m, _| m.model.scanning));
        assert_eq!(model.read_with(cx, |m, _| m.model.archives.len()), 1);

        run_to_end(&model, cx);
        assert_eq!(fakes.runs(), 2, "a finished run doesn't block the next");
    }

    #[gpui_kit::test]
    fn notification_suppressed_when_window_active(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = faked_model(cx, &dir, &fakes, two_articles(), at(9, 0));
        let window: gpui_kit::AnyWindowHandle = cx
            .update(|cx| cx.open_window(Default::default(), |_, cx| cx.new(|_| Blank)))
            .unwrap()
            .into();

        run_to_end(&model, cx);
        assert_eq!(
            fakes.notified.borrow().len(),
            1,
            "an open window without focus still gets the banner"
        );

        cx.update_window(window, |_, w, _| w.activate_window())
            .unwrap();
        cx.run_until_parked();
        // Fresh ids again, so the run counts as new.
        model.update(cx, |m, cx| m.update_model(cx, |r| r.articles.clear()));
        run_to_end(&model, cx);
        assert_eq!(model.read_with(cx, |m, _| m.model.archives.len()), 2);
        assert_eq!(
            fakes.notified.borrow().len(),
            1,
            "the focused window shows the briefing itself"
        );
    }

    #[gpui_kit::test]
    fn tray_status_updates_on_toggle_read(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = cx.update(|cx| AppModel::init(dir.path().join("radar.json"), None, cx));
        model.update(cx, |m, cx| {
            m.set_hooks(fakes.hooks(Vec::new(), at(9, 0)), cx)
        });
        assert_eq!(
            fakes.tray_now(),
            Some((None, "Radar — all caught up · last run never".into())),
            "installing the hooks shows the current state"
        );

        model.update(cx, |m, cx| {
            m.update_model(cx, |r| r.articles = two_articles())
        });
        assert_eq!(fakes.tray_now().unwrap().0.as_deref(), Some("2"));
        model.update(cx, |m, cx| m.toggle_read(cx, 1));
        let (title, tooltip) = fakes.tray_now().unwrap();
        assert_eq!(title.as_deref(), Some("1"));
        assert!(
            tooltip.starts_with("Radar — 1 unread · last run never\n\nFeed — Article 1"),
            "{tooltip}"
        );

        let pushes = fakes.tray.borrow().len();
        model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                r.prefs.density = radar_core::types::Density::Compact
            })
        });
        assert_eq!(fakes.tray.borrow().len(), pushes, "same status, no push");
    }
}
