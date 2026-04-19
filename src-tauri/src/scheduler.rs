//! In-process daily briefing scheduler.
//!
//! A `tokio::spawn`ed loop that polls the Tauri store every 30s, decides
//! whether today's scheduled slot has arrived and we haven't fired yet, and
//! if so runs the ingest pipeline and surfaces the result via:
//!   - a persisted update to `articles` + `archives` in the store
//!   - a `briefing://completed` event so an open webview refreshes its UI
//!   - a native notification banner
//!
//! The scheduler is the authoritative daily timer — the old JS setTimeout
//! path in `App.jsx` is removed. One source of truth, no drift.
//!
//! **Scope note**: this still only fires while the Radar process is alive.
//! "Fires after the user hits Quit" requires OS-level scheduling (launchd
//! plist with StartCalendarInterval on macOS, etc.) that relaunches Radar
//! at the appointed time. Once Radar is up, this scheduler handles the
//! actual work — so the OS plist is a clean follow-on, not a rewrite.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::{Store, StoreExt};

use crate::ingest::{self, CategoryInput, SourceInput};

pub const COMPLETED_EVENT: &str = "briefing://completed";

const STORE_FILE: &str = "radar.json";
const TICK_INTERVAL_SECS: u64 = 30;
/// Small delay before the first tick so the webview has time to mount and
/// register its `briefing://completed` listener — scheduled fires right at
/// launch would otherwise update the store without refreshing the UI.
const WARMUP_SECS: u64 = 5;
/// Matches the frontend's `ARCHIVE_CAP` in `App.jsx`.
const ARCHIVE_CAP: usize = 90;

pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(WARMUP_SECS)).await;
        loop {
            if let Err(e) = tick(&app).await {
                eprintln!("[scheduler] tick error: {e}");
            }
            tokio::time::sleep(Duration::from_secs(TICK_INTERVAL_SECS)).await;
        }
    });
}

async fn tick<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("store open: {e}"))?;

    let prefs = read_prefs(&store);
    if !prefs.schedule_enabled {
        return Ok(());
    }

    let Some((hour, minute)) = parse_time(&prefs.schedule_time) else {
        return Err(format!("bad scheduleTime: {:?}", prefs.schedule_time));
    };

    let Some(today_slot) = today_slot(hour, minute) else {
        return Err("couldn't compute today's slot".to_string());
    };

    let now = Local::now();
    if now < today_slot {
        return Ok(());
    }

    if last_run_at_or_after(&store, &today_slot) {
        return Ok(());
    }

    let sources = read_sources(&store);
    if !sources
        .iter()
        .any(|s| s.feed_url.as_deref().is_some_and(|u| !u.is_empty()))
    {
        // No usable sources — don't fire (and don't keep retrying; next
        // tick will re-evaluate).
        return Ok(());
    }

    let categories = read_categories(&store);
    let lens = read_lens(&store);

    fire(app, &store, sources, categories, lens).await
}

async fn fire<R: Runtime>(
    app: &AppHandle<R>,
    store: &Arc<Store<R>>,
    sources: Vec<SourceInput>,
    categories: Vec<CategoryInput>,
    lens: Option<String>,
) -> Result<(), String> {
    let prev_ids = read_article_ids(store);

    let response = ingest::run_briefing(sources, categories, lens).await?;

    // Persist so the next webview mount sees fresh data, and so the next
    // tick's `last_run_at_or_after` check sees we fired.
    persist(store, &response)?;

    // Wake the UI if it's listening. Harmless if no listener is attached.
    app.emit(COMPLETED_EVENT, &response)
        .map_err(|e| format!("emit: {e}"))?;

    let new_count = response
        .articles
        .iter()
        .filter(|a| !prev_ids.contains(&a.id))
        .count();
    if new_count > 0 {
        let total = response.articles.len();
        let sources: HashSet<&str> = response.articles.iter().map(|a| a.source.as_str()).collect();
        let body = format!(
            "{new_count} new · {total} total across {} source{}",
            sources.len(),
            if sources.len() == 1 { "" } else { "s" },
        );
        if let Err(e) = app
            .notification()
            .builder()
            .title("Radar briefing ready")
            .body(body)
            .show()
        {
            eprintln!("[scheduler] notification failed: {e}");
        }
    }

    Ok(())
}

fn persist<R: Runtime>(
    store: &Arc<Store<R>>,
    response: &ingest::BriefingResponse,
) -> Result<(), String> {
    let articles_json =
        serde_json::to_value(&response.articles).map_err(|e| format!("encode articles: {e}"))?;
    store.set("articles", articles_json);

    let errors_json = serde_json::to_value(&response.errors).unwrap_or(Value::Array(vec![]));
    let articles_for_archive = serde_json::to_value(&response.articles)
        .map_err(|e| format!("encode archive articles: {e}"))?;
    let snapshot = json!({
        "id": Utc::now().timestamp_millis(),
        "runAt": Utc::now().to_rfc3339(),
        "articles": articles_for_archive,
        "errors": errors_json,
    });

    let mut archives: Vec<Value> = store
        .get("archives")
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default();
    archives.insert(0, snapshot);
    archives.truncate(ARCHIVE_CAP);
    store.set("archives", Value::Array(archives));

    store.save().map_err(|e| format!("store save: {e}"))
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    #[serde(default)]
    schedule_enabled: bool,
    #[serde(default = "default_schedule_time")]
    schedule_time: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            schedule_enabled: false,
            schedule_time: default_schedule_time(),
        }
    }
}

fn default_schedule_time() -> String {
    "08:00".to_string()
}

fn read_prefs<R: Runtime>(store: &Arc<Store<R>>) -> Prefs {
    store
        .get("prefs")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn read_sources<R: Runtime>(store: &Arc<Store<R>>) -> Vec<SourceInput> {
    store
        .get("sources")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn read_categories<R: Runtime>(store: &Arc<Store<R>>) -> Vec<CategoryInput> {
    store
        .get("categories")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn read_lens<R: Runtime>(store: &Arc<Store<R>>) -> Option<String> {
    let profile = store.get("profile")?;
    let lens = profile.get("lens")?.as_str()?.trim();
    if lens.is_empty() {
        None
    } else {
        Some(lens.to_string())
    }
}

fn read_article_ids<R: Runtime>(store: &Arc<Store<R>>) -> HashSet<u64> {
    store
        .get("articles")
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_u64))
        .collect()
}

fn last_run_at_or_after<R: Runtime>(store: &Arc<Store<R>>, slot: &DateTime<Local>) -> bool {
    let Some(archives) = store.get("archives") else {
        return false;
    };
    let Value::Array(arr) = archives else {
        return false;
    };
    let Some(first) = arr.first() else {
        return false;
    };
    let Some(run_at) = first.get("runAt").and_then(Value::as_str) else {
        return false;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(run_at) else {
        return false;
    };
    parsed.with_timezone(&Local) >= *slot
}

fn parse_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

fn today_slot(hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let date = Local::now().date_naive();
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    let naive = date.and_time(time);
    Local.from_local_datetime(&naive).single()
}
