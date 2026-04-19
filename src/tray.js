// Menu-bar tray bridge. No-ops under plain Vite (`isTauri()` false).
//
// Two responsibilities:
//   - Push "unread · 3h ago" into the tray title whenever the briefing changes.
//   - Listen for the `tray://run-briefing` event and trigger a briefing run.

import { invoke, isTauri } from '@tauri-apps/api/core';
import { relativeTime } from './time.js';

const RUN_EVENT = 'tray://run-briefing';

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
    console.warn('[tray] set_tray_status failed:', e);
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
