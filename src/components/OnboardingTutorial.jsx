import { Icon } from './Icon.jsx';

// First-run walkthrough. Rendered inline on the Briefing view when a new user
// has no sources and no archives. Each step exposes a CTA that navigates to
// the relevant view; completion is inferred from app state (profile name,
// categories count, sources count) so the user sees their progress in-place.
export function OnboardingTutorial({ profile, categories, sources, setView, onAddSource }) {
  const steps = [
    {
      n: 1,
      title: 'Tell Radar who you are',
      body: 'Your name, role, and a short "lens" help Claude weigh what\'s relevant to you.',
      done: !!profile?.name?.trim(),
      cta: 'Open Profile',
      onClick: () => setView('settings'),
    },
    {
      n: 2,
      title: 'Create categories',
      body: 'Categories are buckets Claude sorts stories into — e.g. Packaging, Tools, Industry. Describe each one clearly.',
      done: (categories?.length ?? 0) > 0,
      cta: 'Add categories',
      onClick: () => setView('categories'),
    },
    {
      n: 3,
      title: 'Add RSS sources',
      body: 'Paste a few feed URLs for the sites you already follow. Radar fetches them in parallel each briefing.',
      done: (sources?.length ?? 0) > 0,
      cta: 'Add sources',
      onClick: onAddSource,
    },
    {
      n: 4,
      title: 'Run your briefing',
      body: 'Press Morning Briefing in the top bar. Optional: add your Anthropic API key in Settings → Integrations for AI summaries.',
      done: false,
      cta: 'Open Settings',
      onClick: () => setView('settings'),
    },
  ];

  const remaining = steps.filter(s => !s.done).length;

  return (
    <div style={styles.wrap}>
      <div style={styles.header}>
        <div style={styles.mark}><Icon name="radar" size={18} /></div>
        <div>
          <h2 className="serif" style={styles.title}>Welcome to Radar.</h2>
          <p style={styles.lede}>
            {remaining === 0
              ? 'You\'re set up — press Morning Briefing at the top to fetch your first briefing.'
              : 'A short walkthrough to get your first briefing on the page.'}
          </p>
        </div>
      </div>

      <ol style={styles.list}>
        {steps.map(s => (
          <li key={s.n} style={{
            ...styles.step,
            opacity: s.done ? 0.55 : 1,
          }}>
            <div style={{
              ...styles.num,
              background: s.done ? 'var(--accent-soft)' : 'var(--surface-2)',
              color: s.done ? 'var(--accent-text)' : 'var(--text-2)',
            }}>
              {s.done ? <Icon name="check" size={14} /> : s.n}
            </div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={styles.stepTitle}>{s.title}</div>
              <div style={styles.stepBody}>{s.body}</div>
            </div>
            {!s.done && (
              <button style={styles.cta} onClick={s.onClick}>
                {s.cta}
                <Icon name="chevron-right" size={13} />
              </button>
            )}
          </li>
        ))}
      </ol>
    </div>
  );
}

const styles = {
  wrap: {
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    padding: '28px 28px 18px',
  },
  header: {
    display: 'flex', alignItems: 'flex-start', gap: 14,
    paddingBottom: 18, marginBottom: 6,
    borderBottom: '1px solid var(--border)',
  },
  mark: {
    width: 36, height: 36, borderRadius: 8,
    background: 'var(--accent-soft)', color: 'var(--accent-text)',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    flexShrink: 0,
  },
  title: { fontSize: 22, fontWeight: 500, letterSpacing: '-0.02em', margin: '2px 0 4px' },
  lede: { fontSize: 13.5, color: 'var(--text-2)', margin: 0, lineHeight: 1.5 },
  list: { listStyle: 'none', padding: 0, margin: 0 },
  step: {
    display: 'flex', alignItems: 'center', gap: 14,
    padding: '14px 0',
    borderBottom: '1px solid var(--border)',
  },
  num: {
    width: 26, height: 26, borderRadius: '50%',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    fontSize: 12.5, fontWeight: 500,
    flexShrink: 0,
    fontVariantNumeric: 'tabular-nums',
  },
  stepTitle: { fontSize: 14, fontWeight: 500, marginBottom: 2 },
  stepBody: { fontSize: 12.5, color: 'var(--text-2)', lineHeight: 1.5 },
  cta: {
    display: 'inline-flex', alignItems: 'center', gap: 6,
    height: 30, padding: '0 12px',
    background: 'var(--surface-2)',
    border: '1px solid var(--border-strong)',
    borderRadius: 6,
    fontSize: 12.5, color: 'var(--text)',
    cursor: 'pointer',
    flexShrink: 0,
  },
};
