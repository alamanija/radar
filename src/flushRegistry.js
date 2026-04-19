// Global "flush now" registry. `useSyncedResource` registers a callback
// that enqueues its current pending debounce synchronously; listeners fire
// on the three events a user-visible "I'm leaving" would surface through:
//
//   - `beforeunload` — navigation away in a plain web context
//   - `pagehide`     — Safari's unload substitute; also fires on bfcache
//   - `visibilitychange -> hidden` — the most reliable desktop-close signal,
//     including Tauri where the window close path runs through the webview
//
// Handlers are registered once per app load; hooks add themselves via
// `registerFlusher` and remove on unmount.

const flushers = new Set();
let attached = false;

function attach() {
  if (attached || typeof window === 'undefined') return;
  attached = true;
  const run = () => {
    for (const f of flushers) {
      try { f(); } catch (e) { console.error('flusher threw:', e); }
    }
  };
  window.addEventListener('beforeunload', run);
  window.addEventListener('pagehide', run);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') run();
  });
}

export function registerFlusher(fn) {
  attach();
  flushers.add(fn);
  return () => flushers.delete(fn);
}
