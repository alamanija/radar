// Frontend calls radar-server directly, attaching the Clerk session JWT as
// a bearer token. `getToken` comes from Clerk's `useAuth()` hook.
//
// Mutations (PUT/PATCH/DELETE) route through `syncQueue.queuedMutation` so
// a transient failure (network drop, 5xx, 429) ends up in the persistent
// retry queue instead of being lost; GETs go through raw fetch.

import { queuedMutation } from './syncQueue.js';

const BASE = (import.meta.env.VITE_RADAR_SERVER_URL || 'http://127.0.0.1:8787')
  .replace(/\/$/, '');

export function makeSyncFetch(getToken) {
  return async (path, { method = 'GET', body, headers = {}, groupKey } = {}) => {
    if (method !== 'GET') {
      return queuedMutation({ path, method, body, headers, groupKey });
    }
    const token = await getToken();
    if (!token) throw new Error('not signed in');
    return fetch(`${BASE}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        ...headers,
      },
    });
  };
}

export { BASE as SYNC_BASE };
