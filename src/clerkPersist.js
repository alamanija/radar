// Persist Clerk's dev-browser JWT across app restarts under Tauri.
//
// Background: Clerk's dev instance identifies each browser with a long-lived
// `__clerk_db_jwt` (~2 months on Clerk's dev issuer). In normal browsers that
// JWT rides a first-party cookie on `*.clerk.accounts.dev` plus a localStorage
// copy. Under `tauri://localhost`, `*.clerk.accounts.dev` is third-party and
// WebKit drops the cookie across process restarts — so on relaunch Clerk
// bootstraps a brand-new anonymous client and the user is "signed out" even
// though the server-side session is still valid.
//
// Fix: stash the JWT in the OS keychain, and on boot inject it back into the
// page URL as `?__clerk_db_jwt=...` before Clerk initialises. Clerk's
// `extractDevBrowserJWTFromURL` picks it up, removes it from the URL, and
// resumes the existing client → the active session rehydrates.
//
// A fetch-wrapper watches every request for a fresh `__clerk_db_jwt` and
// persists it whenever Clerk rotates. No-op outside Tauri (cookies work
// normally in a real browser).

import { invoke, isTauri } from '@tauri-apps/api/core';
import { logger } from './log.js';

const JWT_PARAM = '__clerk_db_jwt';

let lastPersisted = null;

export async function restoreDevBrowserJwt() {
  if (!isTauri()) return;
  try {
    const jwt = await invoke('get_clerk_db_jwt');
    if (!jwt) return;
    lastPersisted = jwt;
    const url = new URL(window.location.href);
    if (url.searchParams.get(JWT_PARAM) === jwt) return;
    url.searchParams.set(JWT_PARAM, jwt);
    window.history.replaceState(null, '', url);
  } catch (e) {
    logger.warn(`[clerkPersist] restore failed: ${e}`);
  }
}

export function installDevBrowserJwtWatcher() {
  if (!isTauri()) return;
  const origFetch = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    try {
      const href = typeof input === 'string'
        ? input
        : input instanceof URL ? input.href : input?.url;
      if (href) {
        const u = new URL(href, window.location.href);
        const jwt = u.searchParams.get(JWT_PARAM);
        if (jwt && jwt !== lastPersisted) {
          lastPersisted = jwt;
          invoke('set_clerk_db_jwt', { jwt }).catch((e) => {
            logger.warn(`[clerkPersist] persist failed: ${e}`);
          });
        }
      }
    } catch {
      // URL parsing can throw on opaque inputs (e.g. Request objects built
      // from blobs) — nothing to persist in those cases.
    }
    return origFetch(input, init);
  };
}
