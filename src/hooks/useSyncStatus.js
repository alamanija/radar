import { useEffect, useState } from 'react';
import { subscribeSyncStatus } from '../syncQueue.js';

// Reactive view over the sync queue's status snapshot. Subscribers are
// notified on every queue transition (enqueue / drain success / drain fail /
// give-up) so the UI reflects the truth without polling.
export function useSyncStatus() {
  const [status, setStatus] = useState({
    pending: 0,
    inflight: false,
    lastError: null,
    lastSuccessAt: null,
  });
  useEffect(() => subscribeSyncStatus(setStatus), []);
  return status;
}
