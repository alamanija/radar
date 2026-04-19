import { useEffect, useRef, useState } from 'react';
import { onDrainSuccess } from '../syncQueue.js';
import { registerFlusher } from '../flushRegistry.js';

/**
 * Sync a client-side resource with a `/sync/<endpoint>` server route. Pull
 * on sign-in, debounced push on change, ETag-based conflict detection, and
 * a retry queue underneath for transient failures.
 *
 * The hook is the only component that knows the HTTP shape. Callers provide
 * transformation functions and an endpoint — everything else (auth header,
 * If-Match, queuing, flush-on-hide) is handled here.
 *
 * Guards (unchanged from prior version):
 *   - `pulled` gates pushes until the first pull finishes so defaults never
 *     clobber the server.
 *   - `serverKeyRef` holds the last-known server shape so no-op echoes are
 *     dropped.
 *
 * Conflict handling:
 *   - Each push sends `If-Match` with the last-known `updatedAt`.
 *   - On 412 Precondition Failed, we refetch, let `fromRemote` merge, and
 *     update the etag. We do NOT re-push automatically — the merged state
 *     is now the truth and the next local edit will push naturally.
 *
 * @param {object}   o
 * @param {*}        o.value         current local value
 * @param {function} o.setValue      setter
 * @param {object|null} o.account    signed-in user, or null
 * @param {boolean}  o.ready         true once the local store has hydrated
 * @param {string}   o.label         short label used in log messages
 * @param {function} o.syncFetch     the token-authenticated fetch wrapper
 * @param {string}   o.endpoint      e.g. '/sync/categories'
 * @param {function} o.buildPushBody (value) => request body for PUT
 * @param {function} o.fromRemote    (serverSnapshot, prev) => localValue
 * @param {function} o.stableKey     (localValue) => string used for dedup
 * @param {function} o.isEmpty       (localValue) => bool, skips seed-push
 * @param {number}   [o.debounceMs=800]
 */
export function useSyncedResource({
  value, setValue, account, ready, label,
  syncFetch, endpoint,
  buildPushBody,
  fromRemote = (r) => r,
  stableKey, isEmpty,
  debounceMs = 800,
}) {
  const [pulled, setPulled] = useState(false);
  const serverKeyRef = useRef(null);
  const etagRef = useRef(null);
  const groupKey = `PUT:${endpoint}`;

  // Keep latest props reachable from the debounce timer / flush handler
  // without forcing re-registration every render.
  const latest = useRef({
    value, syncFetch, buildPushBody, stableKey, endpoint,
  });
  latest.current = { value, syncFetch, buildPushBody, stableKey, endpoint };

  // Pull on sign-in / ready transition.
  useEffect(() => {
    serverKeyRef.current = null;
    etagRef.current = null;
    setPulled(false);
    if (!ready || !account) return;

    let cancelled = false;
    (async () => {
      try {
        const remote = await pull({ syncFetch, endpoint });
        if (cancelled) return;
        if (remote) {
          etagRef.current = remote.updatedAt ?? null;
          const next = fromRemote(remote, value);
          serverKeyRef.current = stableKey(next);
          setValue(() => next);
        } else if (!isEmpty(value)) {
          const pushed = await push({
            syncFetch, endpoint, groupKey,
            body: buildPushBody(value),
            etag: null,
          });
          if (cancelled) return;
          if (pushed?.snapshot) {
            etagRef.current = pushed.snapshot.updatedAt ?? etagRef.current;
            serverKeyRef.current = stableKey(fromRemote(pushed.snapshot, value));
          } else {
            // Queued — we don't know the server etag yet. Mark the current
            // value as "pushed" so the push effect doesn't immediately fire
            // again; the drain callback below will update etagRef later.
            serverKeyRef.current = stableKey(value);
          }
        } else {
          serverKeyRef.current = stableKey(value);
        }
      } catch (e) {
        console.error(`sync ${label} on sign-in failed:`, e);
      } finally {
        if (!cancelled) setPulled(true);
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, account]);

  // Listen for queued-push drains so our etag/serverKey catch up once the
  // network recovers. The listener is registered once per mount and covers
  // every drain for this endpoint.
  useEffect(() => {
    const off = onDrainSuccess(groupKey, (body) => {
      if (!body) return;
      if (body.updatedAt) etagRef.current = body.updatedAt;
      try {
        const { value: v, stableKey: sk } = latest.current;
        const merged = fromRemote(body, v);
        serverKeyRef.current = sk(merged);
      } catch (e) {
        console.error(`drain-success merge for ${label} failed:`, e);
      }
    });
    return off;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groupKey]);

  // Debounced push on value change.
  useEffect(() => {
    if (!ready || !account || !pulled) return;
    const key = stableKey(value);
    if (serverKeyRef.current === key) return;

    let fired = false;
    const doPush = async () => {
      if (fired) return;
      fired = true;
      try {
        const pushed = await push({
          syncFetch, endpoint, groupKey,
          body: buildPushBody(value),
          etag: etagRef.current,
        });
        if (pushed?.conflict) {
          console.warn(`sync ${label}: conflict on push — refetching`);
          const remote = await pull({ syncFetch, endpoint });
          if (!remote) return;
          etagRef.current = remote.updatedAt ?? null;
          const next = fromRemote(remote, value);
          setValue(() => next);
          // Re-push the merged state with the fresh etag. For resources that
          // server-wins on merge (profile/categories/sources/prefs) this is
          // an idempotent no-op the server just accepts; for archives, whose
          // fromRemote union-merges local-only entries back in, it's what
          // actually gets those entries to the server. Single retry — if we
          // hit 412 again, something's thrashing and we bail.
          const retry = await push({
            syncFetch, endpoint, groupKey,
            body: buildPushBody(next),
            etag: etagRef.current,
          });
          if (retry?.snapshot) {
            etagRef.current = retry.snapshot.updatedAt ?? etagRef.current;
            serverKeyRef.current = stableKey(fromRemote(retry.snapshot, next));
          } else if (retry?.conflict) {
            console.warn(`sync ${label}: retry after merge also conflicted`);
            serverKeyRef.current = stableKey(next);
          } else {
            // Queued — optimistic.
            serverKeyRef.current = stableKey(next);
          }
          return;
        }
        if (pushed?.snapshot) {
          etagRef.current = pushed.snapshot.updatedAt ?? etagRef.current;
          serverKeyRef.current = stableKey(fromRemote(pushed.snapshot, value));
        } else {
          // Queued — optimistically mark pushed to avoid a re-push loop.
          serverKeyRef.current = key;
        }
      } catch (e) {
        console.error(`sync ${label} push failed:`, e);
      }
    };

    const timer = setTimeout(doPush, debounceMs);
    const unreg = registerFlusher(doPush);
    return () => {
      clearTimeout(timer);
      unreg();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, account, pulled, value]);

  return { pulled };
}

async function pull({ syncFetch, endpoint }) {
  const resp = await syncFetch(endpoint);
  if (resp.status === 404) return null;
  if (!resp.ok) throw new Error(`pull ${endpoint}: ${resp.status}`);
  return resp.json();
}

async function push({ syncFetch, endpoint, groupKey, body, etag }) {
  const headers = {};
  if (etag) headers['If-Match'] = `W/"${etag}"`;
  const resp = await syncFetch(endpoint, {
    method: 'PUT',
    body,
    headers,
    groupKey,
  });
  if (resp.status === 412) {
    return { conflict: true };
  }
  if (resp.status === 202) {
    // queued — the synthetic response echoes our body but no snapshot.
    return { queued: true };
  }
  if (!resp.ok) throw new Error(`push ${endpoint}: ${resp.status}`);
  const snapshot = await resp.json();
  return { snapshot };
}
