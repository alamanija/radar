//! Briefings that start on their own: the daily slot, run-at-launch, and
//! the OS wake entry that relaunches Radar for the slot. Ports
//! `src-tauri/src/scheduler.rs` and the run-at-launch and schedule-wake
//! effects of `App.jsx`.
//!
//! Every briefing goes through [`AppModel::run_briefing`], which persists
//! the result itself, so the Tauri scheduler's separate store-writing path
//! is gone. All of this runs with no window open.

use crate::app_model::AppModel;
use crate::bridge;
use chrono::{DateTime, Local, TimeZone as _};
use gpui_kit::{App, AppContext as _, Context, Task};
use radar_core::model::RadarModel;
use radar_core::schedule;
use radar_core::types::Prefs;
use std::sync::Arc;
use std::time::Duration;

/// Delay before the first tick.
pub const WARMUP: Duration = Duration::from_secs(5);
/// How often the daily slot is checked.
pub const TICK: Duration = Duration::from_secs(30);

/// The blocking OS wake sync, `(enabled, hour, minute)`:
/// `schedule_wake::sync_schedule_wake` in the app.
pub type WakeFn = Arc<dyn Fn(bool, Option<u32>, Option<u32>) -> Result<(), String> + Send + Sync>;

/// Scheduler bookkeeping, kept on the model.
#[derive(Default)]
pub struct State {
    /// Run-at-launch was decided (fired or skipped) in this process.
    launch_decided: bool,
    /// When the loop last started a briefing. It counts as a run for the
    /// slot, so a briefing that archives nothing (every feed failed) isn't
    /// restarted on every tick.
    fired_at: Option<DateTime<Local>>,
    /// The schedule prefs last handed to the wake syncer.
    wake_synced: Option<(bool, String)>,
    /// The newest wake sync; the next one waits for it, so they apply in
    /// order.
    wake_task: Option<Task<()>>,
}

impl State {
    /// Forgets what the wake syncer was told, so the next
    /// [`sync_wake`](Self::sync_wake) passes the prefs on again.
    pub fn forget_wake(&mut self) {
        self.wake_synced = None;
    }

    /// Hands changed schedule prefs to `wake` on the blocking pool, in
    /// order (`App.jsx:431-441`). An unparsable time removes the entry.
    pub fn sync_wake(&mut self, prefs: &Prefs, wake: &WakeFn, cx: &mut App) {
        let key = (prefs.schedule_enabled, prefs.schedule_time.clone());
        if self.wake_synced.as_ref() == Some(&key) {
            return;
        }
        self.wake_synced = Some(key);
        let hm = schedule::parse_time(&prefs.schedule_time);
        let enabled = prefs.schedule_enabled && hm.is_some();
        let (hour, minute) = (hm.map(|t| t.0), hm.map(|t| t.1));
        let wake = wake.clone();
        let previous = self.wake_task.take();
        self.wake_task = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            match bridge::on_blocking(move || wake(enabled, hour, minute)).await {
                Ok(()) => log::info!("[schedule-wake] synced (enabled: {enabled})"),
                Err(e) => log::warn!("[schedule-wake] sync failed: {e}"),
            }
        }));
    }
}

/// Starts the daily-slot loop: after [`WARMUP`], a check every [`TICK`].
/// Call once at startup.
pub fn spawn(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(WARMUP).await;
        log::info!("[scheduler] loop started");
        loop {
            cx.update(|cx| AppModel::global(cx).update(cx, tick));
            cx.background_executor().timer(TICK).await;
        }
    })
    .detach();
}

/// Starts the briefing when today's slot has passed and nothing ran at or
/// after it. Waits while a briefing runs or no source can be fetched (a
/// run then would only clear the briefing); the next tick looks again.
fn tick(m: &mut AppModel, cx: &mut Context<AppModel>) {
    let now = m.now();
    let archived = m
        .model
        .last_run_at_ms()
        .and_then(|ms| Local.timestamp_millis_opt(ms).single());
    let last_run = archived.max(m.schedule.fired_at);
    if m.model.scanning
        || !schedule::should_fire(&m.model.prefs, last_run, now)
        || !has_usable_source(&m.model)
    {
        return;
    }
    log::info!(
        "[scheduler] firing briefing for {}",
        m.model.prefs.schedule_time
    );
    m.schedule.fired_at = Some(now);
    m.run_briefing(cx);
}

/// Run-at-launch (`App.jsx` `launchFired`): decided once per process,
/// once the model is ready. It fires when the pref is on and a source can
/// be fetched.
pub fn run_at_launch(cx: &mut App) {
    AppModel::global(cx).update(cx, |m, cx| {
        if !m.ready || m.schedule.launch_decided {
            return;
        }
        m.schedule.launch_decided = true;
        if m.model.prefs.run_at_launch && has_usable_source(&m.model) {
            log::info!("[briefing] run at launch");
            m.run_briefing(cx);
        }
    });
}

/// An enabled source with a feed URL: one a briefing fetches (the filter
/// of `RadarModel::briefing_inputs`).
fn has_usable_source(m: &RadarModel) -> bool {
    m.sources
        .iter()
        .any(|s| s.enabled && s.feed_url.as_deref().is_some_and(|u| !u.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{at, faked_model, settle, wait_until, Fakes};
    use crate::workspace::shell_tests::article;
    use gpui_kit::{Entity, TestAppContext};
    use radar_core::types::Theme;

    fn idle(model: &Entity<AppModel>, cx: &mut TestAppContext) {
        wait_until(cx, |cx| !model.read_with(cx, |m, _| m.model.scanning));
    }

    fn schedule_at(model: &Entity<AppModel>, cx: &mut TestAppContext, time: &str) {
        model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                r.prefs.schedule_enabled = true;
                r.prefs.schedule_time = time.into();
            })
        });
    }

    fn tick_after(cx: &mut TestAppContext, d: Duration) {
        cx.executor().advance_clock(d);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn scheduler_fires_when_due(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let one = vec![article(1, "tools", false)];
        let model = faked_model(cx, &dir, &fakes, one, at(9, 0));
        schedule_at(&model, cx, "07:30");
        cx.update(spawn);

        tick_after(cx, WARMUP - Duration::from_secs(1));
        assert_eq!(fakes.runs(), 0, "nothing fires during the warmup");
        tick_after(cx, Duration::from_secs(1));
        assert_eq!(fakes.runs(), 1, "09:00 is past the 07:30 slot");
        idle(&model, cx);
        let last_run = model.read_with(cx, |m, _| m.model.last_run_at_ms());
        assert_eq!(last_run, Some(at(9, 0).timestamp_millis()));

        tick_after(cx, TICK);
        assert_eq!(fakes.runs(), 1, "the run is after the slot: not again");
    }

    /// Every feed failed: nothing is archived, so `last_run_at` stays before
    /// the slot. The loop counts its own attempt instead of retrying the
    /// whole briefing every 30 s.
    #[gpui_kit::test]
    fn scheduler_fires_a_slot_once_even_when_nothing_is_archived(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = faked_model(cx, &dir, &fakes, Vec::new(), at(9, 0));
        schedule_at(&model, cx, "07:30");
        cx.update(spawn);

        tick_after(cx, WARMUP);
        assert_eq!(fakes.runs(), 1);
        idle(&model, cx);
        assert!(model.read_with(cx, |m, _| m.model.archives.is_empty()));
        for _ in 0..3 {
            tick_after(cx, TICK);
        }
        assert_eq!(fakes.runs(), 1);
    }

    /// Like the Tauri scheduler: no fetchable source, no run (it would only
    /// clear the briefing); the next tick looks again.
    #[gpui_kit::test]
    fn scheduler_waits_for_a_usable_source(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let one = vec![article(1, "tools", false)];
        let model = faked_model(cx, &dir, &fakes, one, at(9, 0));
        let id = model.read_with(cx, |m, _| m.model.sources[0].id);
        model.update(cx, |m, cx| m.update_model(cx, |r| r.toggle_source(id)));
        schedule_at(&model, cx, "07:30");
        cx.update(spawn);

        tick_after(cx, WARMUP);
        assert_eq!(fakes.runs(), 0, "the only source is disabled");
        model.update(cx, |m, cx| m.update_model(cx, |r| r.toggle_source(id)));
        tick_after(cx, TICK);
        assert_eq!(fakes.runs(), 1);
        idle(&model, cx);
    }

    #[gpui_kit::test]
    fn run_at_launch_fires_once_and_only_with_usable_source(cx: &mut TestAppContext) {
        let launch = |cx: &mut TestAppContext, on: bool, usable: bool| {
            let dir = tempfile::tempdir().unwrap();
            let fakes = Fakes::default();
            let one = vec![article(1, "tools", false)];
            let model = faked_model(cx, &dir, &fakes, one, at(9, 0));
            model.update(cx, |m, cx| {
                m.update_model(cx, |r| {
                    r.prefs.run_at_launch = on;
                    if !usable {
                        r.sources[0].feed_url = None;
                    }
                })
            });
            cx.update(run_at_launch);
            (dir, fakes, model)
        };

        let (_dir, fakes, model) = launch(cx, true, true);
        assert_eq!(fakes.runs(), 1);
        idle(&model, cx);
        cx.update(run_at_launch);
        assert_eq!(fakes.runs(), 1, "once per launch");

        let (_dir, fakes, model) = launch(cx, true, false);
        assert_eq!(fakes.runs(), 0, "no source with a feed URL");
        // Decided at launch: a source added later doesn't trigger it.
        model.update(cx, |m, cx| {
            m.update_model(cx, |r| {
                r.sources[0].feed_url = Some("https://feed.test/rss".into())
            })
        });
        cx.update(run_at_launch);
        assert_eq!(fakes.runs(), 0);

        let (_dir, fakes, _model) = launch(cx, false, true);
        assert_eq!(fakes.runs(), 0, "the pref is off");
    }

    #[gpui_kit::test]
    fn schedule_pref_change_calls_wake_sync_after_ready_only(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let fakes = Fakes::default();
        let model = cx.update(|cx| AppModel::init(dir.path().join("radar.json"), None, cx));
        let edit = |cx: &mut TestAppContext, f: fn(&mut Prefs)| {
            model.update(cx, |m, cx| m.update_model(cx, |r| f(&mut r.prefs)));
        };

        model.update(cx, |m, cx| {
            m.ready = false;
            m.set_hooks(fakes.hooks(Vec::new(), at(9, 0)), cx);
        });
        edit(cx, |p| {
            p.schedule_enabled = true;
            p.schedule_time = "06:45".into();
        });
        settle(cx);
        assert!(fakes.wakes().is_empty(), "nothing before ready");

        model.update(cx, |m, _| m.ready = true);
        edit(cx, |p| p.schedule_time = "08:05".into());
        wait_until(cx, |_| !fakes.wakes().is_empty());
        assert_eq!(fakes.wakes(), vec![(true, Some(8), Some(5))]);

        edit(cx, |p| p.theme = Theme::Dark);
        edit(cx, |p| p.schedule_enabled = false);
        wait_until(cx, |_| fakes.wakes().len() == 2);
        settle(cx);
        assert_eq!(
            fakes.wakes(),
            vec![(true, Some(8), Some(5)), (false, Some(8), Some(5))],
            "only schedule changes sync, in order"
        );

        // Hooks installed on a ready model sync once with the hydrated prefs
        // (the JS effect's first run).
        let fresh = Fakes::default();
        model.update(cx, |m, cx| {
            m.set_hooks(fresh.hooks(Vec::new(), at(9, 0)), cx)
        });
        wait_until(cx, |_| !fresh.wakes().is_empty());
        assert_eq!(fresh.wakes(), vec![(false, Some(8), Some(5))]);
    }
}
