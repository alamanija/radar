// Native-banner notification when a briefing completes with new articles.
// No-op under plain Vite (`isTauri()` false); on first use under Tauri it
// requests OS notification permission once and caches the result.
//
// Suppressed when the Radar window is focused — if the user is already
// looking, re-rendering the briefing is the notification. The intended case
// is the daily-scheduled run firing while the window is minimised / hidden
// to tray.

import { isTauri } from '@tauri-apps/api/core';

let permissionGranted = null;

async function ensurePermission() {
  if (!isTauri()) return false;
  if (permissionGranted !== null) return permissionGranted;
  const { isPermissionGranted, requestPermission } = await import(
    '@tauri-apps/plugin-notification'
  );
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      const result = await requestPermission();
      granted = result === 'granted';
    }
    permissionGranted = granted;
  } catch (e) {
    console.warn('[notify] permission check failed:', e);
    permissionGranted = false;
  }
  return permissionGranted;
}

export async function notifyBriefingComplete({ newCount, totalCount, sourceCount }) {
  if (!isTauri()) return;
  if (newCount <= 0) return;
  if (document.visibilityState === 'visible' && document.hasFocus()) return;
  if (!(await ensurePermission())) return;

  try {
    const { sendNotification } = await import('@tauri-apps/plugin-notification');
    const body = `${newCount} new · ${totalCount} total across ${sourceCount} source${sourceCount === 1 ? '' : 's'}`;
    sendNotification({ title: 'Radar briefing ready', body });
  } catch (e) {
    console.warn('[notify] send failed:', e);
  }
}
