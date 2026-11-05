//! The global application model: one `Entity<AppModel>` owns all mutable
//! state (port of the state that lived in `App.jsx`) and persists it.
//!
//! # Pattern for later tasks
//! - Get the entity anywhere with [`AppModel::global`]; views `cx.observe`
//!   it to re-render.
//! - Every change to [`RadarModel`] goes through [`AppModel::update_model`].
//!   It notifies observers, re-applies theme tokens when the visual prefs
//!   changed, and (re)schedules one debounced save ([`SAVE_DEBOUNCE`],
//!   matching plugin-store's 100 ms autoSave). Never mutate `model` directly.
//! - Hydration ([`AppModel::new`]) only reads the store; `ready` flips true
//!   once it is done, and saves are gated on it.

use crate::theme;
use gpui_kit::{App, AppContext, Context, Entity, Global, Task};
use radar_core::auth::Session;
use radar_core::model::RadarModel;
use radar_core::store;
use radar_core::sync::{SyncClient, SyncStatus};
use radar_core::types::Account;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Delay between the last mutation and the write to disk.
#[allow(dead_code)] // only reached via update_model until Task 8
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);
/// First retry delay after a failed write; doubles up to [`SAVE_RETRY_MAX`].
#[allow(dead_code)]
pub const SAVE_RETRY_MIN: Duration = Duration::from_secs(1);
#[allow(dead_code)]
pub const SAVE_RETRY_MAX: Duration = Duration::from_secs(60);

// session/account/sync/sync_status are read from Task 8 (sidebar footer)
// and Task 12 (sync wiring) on.
#[allow(dead_code)]
pub struct AppModel {
    pub model: RadarModel,
    /// `None` when Clerk isn't configured (`AuthConfig::from_env()` is
    /// `None`); the app then runs fully local.
    pub session: Option<Session>,
    pub account: Option<Account>,
    /// Wired in Task 12. `Arc` so async work can hold it past a borrow.
    pub sync: Option<Arc<SyncClient>>,
    pub sync_status: SyncStatus,
    /// True once hydration finished; saves are skipped before that.
    pub ready: bool,
    pub store_path: PathBuf,
    /// Persisted offline sync queue (opaque JS shape). Passed through
    /// unchanged until sync is wired (Task 12).
    pub sync_queue: Vec<Value>,
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
        let sync_queue = snap.sync_queue.clone();
        let model = RadarModel::from_snapshot(snap);
        let account = session.as_ref().and_then(Session::account);
        Self {
            model,
            session,
            account,
            sync: None,
            sync_status: SyncStatus::default(),
            ready: true,
            store_path,
            sync_queue,
            save_task: None,
            dirty: false,
            save_gen: 0,
            written_gen: Arc::new(Mutex::new(0)),
        }
    }

    /// Creates the global entity, applies the persisted theme, and flushes
    /// pending writes on quit. Call once at startup.
    pub fn init(store_path: PathBuf, session: Option<Session>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| {
            cx.on_app_quit(|this: &mut Self, _cx| {
                this.flush_now();
                async {}
            })
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

    /// The one write path for [`RadarModel`].
    #[allow(dead_code)] // first non-test caller arrives in Task 8
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
        out
    }

    #[allow(dead_code)]
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.ready {
            return;
        }
        self.dirty = true;
        // Dropping the previous task cancels its pending timer (or its
        // not-yet-started write; `flush_now` and the newer task cover that,
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
        Some(SaveJob {
            path: self.store_path.clone(),
            snap: self.model.to_snapshot(self.sync_queue.clone()),
            generation: self.save_gen,
            written_gen: self.written_gen.clone(),
        })
    }

    /// Writes everything not yet on disk, synchronously (app quit).
    ///
    /// It runs inside the quit callback, so by the time the quit future is
    /// returned there is nothing left pending: GPUI only polls quit futures
    /// for `SHUTDOWN_TIMEOUT` (200 ms), too short to trust with a large
    /// archive write.
    fn flush_now(&mut self) {
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

#[allow(dead_code)]
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
    #[allow(dead_code)]
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
}
