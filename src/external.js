// Open an external URL in the system default browser.
// Under Tauri, uses the opener plugin (the webview blocks window.open to external URLs).
// Under plain Vite dev, falls back to window.open.

import { isTauri } from '@tauri-apps/api/core';

export async function openExternal(url) {
  if (!url) return;
  if (isTauri()) {
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(url);
      return;
    } catch (e) {
      console.error('openExternal (tauri) failed:', e);
    }
  }
  try { window.open(url, '_blank', 'noopener,noreferrer'); }
  catch (e) { console.error('openExternal (window) failed:', e); }
}
