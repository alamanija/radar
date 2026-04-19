// Menu-bar tray bridge. No-ops under plain Vite (`isTauri()` false).
//
// The tray itself is a bare icon with a three-item right-click menu (see
// `src-tauri/src/tray.rs`). Everything status-like — unread count, last
// run, top article headlines — lives in the hover tooltip that we build
// here and push on every briefing change. Keeping the menu static avoids
// the churn of rebuilding native menu items every time state ticks.

import { invoke, isTauri } from '@tauri-apps/api/core';
import { logger } from './log.js';
import { relativeTime } from './time.js';

const RUN_EVENT = 'tray://run-briefing';
const TOOLTIP_ARTICLE_COUNT = 3;
const TOOLTIP_LINE_MAX = 72;

function truncate(s, max) {
  if (!s) return '';
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

function buildTooltip({ unread, lastRunAt, articles }) {
  const lastRun = lastRunAt ? relativeTime(lastRunAt) : 'never';
  const header = unread > 0
    ? `Radar — ${unread} unread · last run ${lastRun}`
    : `Radar — all caught up · last run ${lastRun}`;

  const lines = (articles ?? [])
    .slice(0, TOOLTIP_ARTICLE_COUNT)
    .filter((a) => a && a.title)
    .map((a) => truncate(a.source ? `${a.source} — ${a.title}` : a.title, TOOLTIP_LINE_MAX));

  return lines.length > 0 ? `${header}\n\n${lines.join('\n')}` : header;
}

/// Push a fresh tooltip (and compact menu-bar title) to the tray icon.
/// Pass the full article list — this function picks the top N itself.
export async function pushTrayStatus({ unread, lastRunAt, articles }) {
  if (!isTauri()) return;
  try {
    const safeUnread = Math.max(0, unread | 0);
    await invoke('set_tray_status', {
      title: safeUnread > 0 ? String(safeUnread) : null,
      tooltip: buildTooltip({ unread: safeUnread, lastRunAt, articles }),
    });
  } catch (e) {
    // Tray may not exist on first render if setup() hasn't completed yet.
    // The next status push will succeed.
    logger.warn(`[tray] set_tray_status failed: ${e}`);
  }
}

/// Subscribe to the tray's "Run briefing now" menu item. Returns an unsubscribe.
export async function onTrayRunBriefing(handler) {
  if (!isTauri()) return () => {};
  // Dynamic import so the event module only loads under Tauri.
  const { listen } = await import('@tauri-apps/api/event');
  const unlisten = await listen(RUN_EVENT, () => handler());
  return unlisten;
}
