const toggleStyles = {
  track: {
    width: 30, height: 18, borderRadius: 10,
    background: 'var(--border-strong)',
    position: 'relative', cursor: 'pointer',
    transition: 'background 140ms',
  },
  trackOn: { background: 'var(--accent)' },
  knob: {
    position: 'absolute', top: 2, left: 2,
    width: 14, height: 14, borderRadius: '50%',
    background: 'var(--surface)',
    transition: 'transform 140ms',
  },
};

export function Toggle({ on, onChange }) {
  return (
    <div style={{ ...toggleStyles.track, ...(on ? toggleStyles.trackOn : {}) }}
         onClick={onChange}>
      <div style={{ ...toggleStyles.knob, transform: on ? 'translateX(12px)' : 'none' }} />
    </div>
  );
}
