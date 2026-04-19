import { useSyncStatus } from '../hooks/useSyncStatus.js';

// Three-state sync indicator for the sidebar footer. Silent on the happy
// path (a single muted line), bumps to amber when changes are in-flight,
// red when the queue has permanently given up on something. Deliberately
// text-light — users in the success case shouldn't be asked to interpret
// anything beyond "OK".
export function SyncIndicator() {
  const { pending, inflight, lastError } = useSyncStatus();

  let tone = 'idle';
  let label = 'Synced';
  let title = 'All changes saved to Radar';

  if (lastError && pending === 0) {
    tone = 'error';
    label = 'Sync issue';
    title = lastError;
  } else if (pending > 0 || inflight) {
    tone = 'busy';
    label = pending > 0 ? `Syncing ${pending} change${pending === 1 ? '' : 's'}…` : 'Syncing…';
    title = 'Changes will keep retrying until they reach the server';
  }

  const dot = dots[tone];
  return (
    <div style={styles.row} title={title}>
      <span style={{ ...styles.dot, background: dot.color }} />
      <span style={styles.label}>{label}</span>
    </div>
  );
}

const dots = {
  idle:  { color: 'var(--text-3)' },
  busy:  { color: '#c48a2a' },
  error: { color: '#b3513a' },
};

const styles = {
  row: {
    display: 'flex', alignItems: 'center', gap: 8,
    padding: '6px 12px',
    fontSize: 11,
    color: 'var(--text-3)',
    borderTop: '1px solid var(--border)',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
    userSelect: 'none',
  },
  dot: { width: 7, height: 7, borderRadius: '50%', flexShrink: 0 },
  label: {
    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
    fontVariantNumeric: 'tabular-nums',
  },
};
