// Persistent offline queue for mutating sync requests.
//
// Design
// ------
// Every mutating sync call (PUT/PATCH/DELETE against radar-server) routes
// through this module. When the network is available and the server answers
// a non-retryable status (2xx or a 4xx other than 408/429/5xx), the request
// resolves directly. On transient failure — network unreachable, 5xx, 408,
// 429 — the request body is appended to a disk-persisted queue and a
// synthetic `ok` response is returned to the caller so UI state treats the
// write as provisionally successful.
//
// The queue drains in the background whenever:
//   - something gets enqueued while online
//   - `navigator.onLine` flips to true
//   - `installSyncQueue` is called on app boot
//   - a visibility/`beforeunload` handler forces a flush (see flushOnUnload)
//
// Dedup semantics
// ---------------
// Each enqueue specifies a `groupKey`. When a new request is enqueued with
// an existing key, the older one is dropped — the latest full-list PUT
// always supersedes earlier ones (last-write-wins against our own queue).
// Article-state PATCHes suffix the key with the article id so per-article
// writes don't collide but repeated toggles on one article do dedup.

import { getItem, setItem } from './storage.js';

const QUEUE_KEY = 'sync-queue';
const MAX_TRIES = 8;
const MIN_BACKOFF_MS = 500;
const MAX_BACKOFF_MS = 60_000;

let baseUrl = '';
let tokenGetter = null;
let currentAccountId = null;
let queue = null;
let draining = false;
let installed = false;

// groupKey -> (serverBody, serverHeaders) => void
// Callers (useSyncedResource) register here so that when a queued mutation
// eventually lands successfully, the hook can pick up the fresh ETag and
// stop sending stale If-Match headers.
const drainSuccessListeners = new Map();

export function onDrainSuccess(groupKey, fn) {
  drainSuccessListeners.set(groupKey, fn);
  return () => {
    if (drainSuccessListeners.get(groupKey) === fn) {
      drainSuccessListeners.delete(groupKey);
    }
  };
}

// Observable queue status for UI consumers.
//   pending: items currently queued (across all accounts)
//   inflight: are we in the middle of a drain attempt?
//   lastError: message of the most recent failure (retryable or terminal)
//   lastSuccessAt: Date.now() of the last successful drained mutation
const statusListeners = new Set();
let status = {
  pending: 0,
  inflight: false,
  lastError: null,
  lastSuccessAt: null,
};

export function subscribeSyncStatus(fn) {
  statusListeners.add(fn);
  fn(status);
  return () => statusListeners.delete(fn);
}

function emitStatus(patch) {
  status = { ...status, ...patch };
  for (const fn of statusListeners) {
    try { fn(status); } catch (e) { console.error('status listener threw:', e); }
  }
}

async function loadQueue() {
  if (queue == null) {
    const raw = await getItem(QUEUE_KEY);
    queue = Array.isArray(raw) ? raw : [];
  }
  return queue;
}

async function saveQueue() {
  await setItem(QUEUE_KEY, queue ?? []);
}

function isOnline() {
  return typeof navigator === 'undefined' || navigator.onLine !== false;
}

function isRetryableStatus(status) {
  return status === 408 || status === 429 || (status >= 500 && status < 600);
}

// Caller sees an "it worked" Response-like so the UI doesn't bounce back
// into an error state. The body comes from the original request; that
// matches the shape every handler would have returned (minus server-stamped
// fields like updatedAt, which callers already handle as optional).
function syntheticAccepted(body) {
  return {
    ok: true,
    status: 202,
    headers: {
      get() { return null; },
    },
    async json() { return body ?? {}; },
    async text() { return JSON.stringify(body ?? {}); },
  };
}

export function installSyncQueue({ getToken, base, accountId = null }) {
  tokenGetter = getToken;
  baseUrl = (base || '').replace(/\/$/, '');
  currentAccountId = accountId;
  if (installed) {
    drain();
    return;
  }
  installed = true;
  loadQueue().then(() => drain());
  if (typeof window !== 'undefined') {
    window.addEventListener('online', drain);
  }
}

/**
 * Append a mutation to the queue and kick off a drain. `body` and `headers`
 * are stored as-is; the Authorization header is re-minted at drain time so
 * tokens don't expire in the queue.
 */
export async function enqueue({ path, method, body, headers = {}, groupKey }) {
  const q = await loadQueue();
  if (groupKey) {
    for (let i = q.length - 1; i >= 0; i--) {
      if (q[i].groupKey === groupKey) q.splice(i, 1);
    }
  }
  q.push({
    id: cryptoRandomId(),
    path, method, body, headers, groupKey,
    accountId: currentAccountId,
    tries: 0,
    queuedAt: Date.now(),
  });
  await saveQueue();
  emitStatus({ pending: q.length });
  drain();
}

async function drain() {
  if (draining || !tokenGetter || !baseUrl) return;
  draining = true;
  emitStatus({ inflight: true });
  try {
    const q = await loadQueue();
    emitStatus({ pending: q.length });
    // Find the first item belonging to the current account and drain from
    // there. Items tagged with a different account stay put; they'll drain
    // the next time that user signs back in on this device.
    let cursor = 0;
    while (cursor < q.length) {
      if (!isOnline()) break;
      const item = q[cursor];
      if (item.accountId && item.accountId !== currentAccountId) {
        cursor += 1;
        continue;
      }
      let outcome = 'retry';
      try {
        const token = await tokenGetter();
        if (!token) break; // not signed in; wait
        const resp = await fetch(`${baseUrl}${item.path}`, {
          method: item.method,
          headers: {
            'Content-Type': 'application/json',
            ...item.headers,
            Authorization: `Bearer ${token}`,
          },
          body: item.body != null ? JSON.stringify(item.body) : undefined,
        });
        if (isRetryableStatus(resp.status)) {
          outcome = 'retry';
        } else {
          // 2xx: succeeded. 4xx (other than retryable): we can't recover
          // automatically — drop the item so the queue isn't jammed forever.
          // 412 Precondition Failed is a conflict that the client's pull/push
          // cycle already knows how to handle on next edit; drop here too.
          if (!resp.ok) {
            console.warn(`sync queue: dropping ${item.method} ${item.path} (${resp.status})`);
            emitStatus({ lastError: `${resp.status} on ${item.path}` });
          } else {
            emitStatus({ lastSuccessAt: Date.now(), lastError: null });
          }
          if (resp.ok && item.groupKey) {
            const listener = drainSuccessListeners.get(item.groupKey);
            if (listener) {
              try {
                const cloned = resp.clone();
                const body = await cloned.json().catch(() => null);
                listener(body, resp.headers);
              } catch (e) {
                console.error('drain listener threw:', e);
              }
            }
          }
          outcome = 'done';
        }
      } catch (err) {
        // Network unreachable, DNS failure, CORS preflight fail, etc.
        outcome = 'retry';
      }

      if (outcome === 'done') {
        q.splice(cursor, 1);
        await saveQueue();
        emitStatus({ pending: q.length });
        continue;
      }

      item.tries += 1;
      if (item.tries > MAX_TRIES) {
        console.error(
          `sync queue: giving up on ${item.method} ${item.path} after ${MAX_TRIES} tries`,
        );
        emitStatus({ lastError: `gave up on ${item.path} after ${MAX_TRIES} tries` });
        q.splice(cursor, 1);
        await saveQueue();
        emitStatus({ pending: q.length });
        continue;
      }
      await saveQueue();
      const backoff = Math.min(
        MAX_BACKOFF_MS,
        MIN_BACKOFF_MS * 2 ** Math.min(item.tries, 6),
      );
      await sleep(backoff);
    }
  } finally {
    draining = false;
    emitStatus({ inflight: false, pending: queue?.length ?? 0 });
  }
}

function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

function cryptoRandomId() {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/**
 * Make a mutating sync request. On transient failure it enqueues and
 * returns a synthetic 202. On non-retryable errors (4xx that aren't
 * 408/429) the real Response is returned so the caller can handle it
 * (e.g. 412 Precondition Failed).
 */
export async function queuedMutation({ path, method, body, headers = {}, groupKey }) {
  if (!tokenGetter || !baseUrl) {
    throw new Error('syncQueue not installed');
  }
  try {
    if (!isOnline()) throw new Error('offline');
    const token = await tokenGetter();
    if (!token) throw new Error('not signed in');
    const resp = await fetch(`${baseUrl}${path}`, {
      method,
      headers: {
        'Content-Type': 'application/json',
        ...headers,
        Authorization: `Bearer ${token}`,
      },
      body: body != null ? JSON.stringify(body) : undefined,
    });
    if (isRetryableStatus(resp.status)) {
      await enqueue({ path, method, body, headers, groupKey });
      return syntheticAccepted(body);
    }
    return resp;
  } catch (err) {
    await enqueue({ path, method, body, headers, groupKey });
    return syntheticAccepted(body);
  }
}
