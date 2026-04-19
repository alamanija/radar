import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';

// Wraps @tauri-apps/plugin-updater. The plugin only exists under Tauri, so
// this hook returns a "not supported" status in plain Vite dev mode instead
// of throwing at import-time — that keeps `npm run dev` working for frontend
// iteration without a Tauri window.
//
// State machine:
//   idle        — nothing in flight
//   checking    — calling `check()`
//   available   — a newer version exists; user can choose to download
//   downloading — bytes coming in; `progress` is 0..1 or null if unknown
//   ready       — downloaded + verified; restart will apply it
//   uptodate    — explicitly up to date (only set by a user-initiated check)
//   error       — last operation failed; see `error`
//
// `autoCheckOnMount` runs a single silent check on first render. If a new
// version exists it quietly moves to `available`; the Settings card picks it
// up and lets the user decide when to download.

export function useUpdater({ autoCheckOnMount = true } = {}) {
  const [state, setState] = useState('idle');
  const [version, setVersion] = useState(null);
  const [notes, setNotes] = useState('');
  const [progress, setProgress] = useState(null);
  const [error, setError] = useState(null);
  const updateRef = useRef(null);
  const supported = isTauri();

  const check = useCallback(async () => {
    if (!supported) {
      setState('uptodate');
      return { available: false };
    }
    setState('checking');
    setError(null);
    try {
      const { check: checkFn } = await import('@tauri-apps/plugin-updater');
      const update = await checkFn();
      if (update) {
        updateRef.current = update;
        setVersion(update.version ?? null);
        setNotes(update.body ?? '');
        setState('available');
        return { available: true, version: update.version };
      }
      updateRef.current = null;
      setVersion(null);
      setNotes('');
      setState('uptodate');
      return { available: false };
    } catch (e) {
      console.error('updater check failed:', e);
      setError(String(e?.message ?? e));
      setState('error');
      return { available: false, error: e };
    }
  }, [supported]);

  const downloadAndInstall = useCallback(async () => {
    if (!supported) return;
    const update = updateRef.current;
    if (!update) {
      // User clicked "download" before check settled — treat as a re-check.
      await check();
      return;
    }
    setState('downloading');
    setError(null);
    try {
      let contentLength = 0;
      let downloaded = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            contentLength = event.data?.contentLength ?? 0;
            downloaded = 0;
            setProgress(contentLength > 0 ? 0 : null);
            break;
          case 'Progress':
            downloaded += event.data?.chunkLength ?? 0;
            if (contentLength > 0) {
              setProgress(Math.min(1, downloaded / contentLength));
            }
            break;
          case 'Finished':
            setProgress(1);
            break;
          default:
        }
      });
      setState('ready');
    } catch (e) {
      console.error('updater download failed:', e);
      setError(String(e?.message ?? e));
      setState('error');
    }
  }, [supported, check]);

  const restartToApply = useCallback(async () => {
    if (!supported) return;
    try {
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      console.error('restart failed:', e);
      setError(String(e?.message ?? e));
      setState('error');
    }
  }, [supported]);

  // One silent check on mount. Intentionally fire-and-forget: if the user
  // isn't online or the endpoint is down, we just stay `idle`.
  useEffect(() => {
    if (!autoCheckOnMount || !supported) return;
    check().catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [supported]);

  return {
    state, version, notes, progress, error, supported,
    check, downloadAndInstall, restartToApply,
  };
}
