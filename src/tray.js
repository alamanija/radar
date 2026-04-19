// Menu-bar tray bridge. No-ops under plain Vite (`isTauri()` false).
//
// Three responsibilities:
//   - Push "unread · 3h ago" into the tray title whenever the briefing changes.
//   - Push the latest N article titles into the tray menu so the user can
//     click straight from the menu bar to an article in their browser.
//   - Listen for the `tray://run-briefing` event and trigger a briefing run.

import { invoke, isTauri } from '@tauri-apps/api/core';
import { logger } from './log.js';
import { relativeTime } from './time.js';

const RUN_EVENT = 'tray://run-briefing';
const TRAY_ARTICLE_LIMIT = 5;

export async function pushTrayStatus({ unread, lastRunAt }) {
  if (!isTauri()) return;
  try {
    await invoke('set_tray_status', {
      unread: Math.max(0, unread | 0),
      lastRunLabel: lastRunAt ? relativeTime(lastRunAt) : null,
    });
  } catch (e) {
    // Tray may not exist on first render if setup() hasn't completed yet.
    // The next status push will succeed.
    logger.warn(`[tray] set_tray_status failed: ${e}`);
  }
}

export async function pushTrayArticles(articles) {
  if (!isTauri()) return;
  try {
    const payload = (articles ?? [])
      .slice(0, TRAY_ARTICLE_LIMIT)
      .filter((a) => a && a.title && a.url)
      .map((a) => ({ title: a.title, url: a.url }));
    await invoke('set_tray_articles', { articles: payload });
  } catch (e) {
    logger.warn(`[tray] set_tray_articles failed: ${e}`);
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
