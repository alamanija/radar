import { Card, Row, SkeletonCard } from '../components/Card.jsx';
import { OnboardingTutorial } from '../components/OnboardingTutorial.jsx';
import { relativeTime } from '../time.js';

const viewStyles = {
  wrap: { padding: '24px 32px 80px', maxWidth: 1240, margin: '0 auto' },
  title: { fontSize: 28, fontWeight: 500, letterSpacing: '-0.02em', marginBottom: 6 },
  subtitle: { color: 'var(--text-2)', fontSize: 14, marginBottom: 22 },
  pills: { display: 'flex', flexWrap: 'wrap', gap: 6, marginBottom: 22, paddingBottom: 18, borderBottom: '1px solid var(--border)' },
  grid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))',
    gap: 14,
  },
  list: {
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    overflow: 'hidden',
  },
  listHead: {
    display: 'grid',
    gridTemplateColumns: '20px 160px 1fr 120px 80px 60px',
    alignItems: 'center',
    height: 36,
    padding: '0 16px',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
    fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.08em',
    color: 'var(--text-3)', fontWeight: 500,
    gap: 14,
  },
  statStrip: {
    display: 'flex', gap: 0, marginBottom: 22,
    border: '1px solid var(--border)', borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    overflow: 'hidden',
  },
  stat: {
    flex: 1, padding: '14px 18px',
    borderRight: '1px solid var(--border)',
    display: 'flex', flexDirection: 'column', gap: 4,
  },
  statLabel: { fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.08em', color: 'var(--text-3)' },
  statValue: { fontSize: 22, fontWeight: 500, letterSpacing: '-0.01em' },
};

export { viewStyles };

export function BriefingView({ articles, categories, sources, profile, archives, viewMode, activeCategory, setActiveCategory, scanning, showAccent, useSerif, errors = [], lastRunAt, profileName, onToggleRead, onToggleBookmark, setView, onAddSource, ready }) {
  const filtered = activeCategory === 'all' ? articles : articles.filter(a => a.category === activeCategory);
  const unread = articles.filter(a => !a.read).length;
  const countFor = (catId) => articles.filter(a => a.category === catId).length;
  const sourceCount = new Set(articles.map(a => a.source)).size;

  // First-run condition: user has never produced a briefing and has no
  // sources yet. Gated on `ready` so the tutorial doesn't flash during the
  // pre-hydration render window, when every list defaults to [].
  const showTutorial = ready
    && !scanning
    && articles.length === 0
    && (sources?.length ?? 0) === 0
    && (archives?.length ?? 0) === 0;

  return (
    <div style={viewStyles.wrap}>
      <h1 className="serif" style={viewStyles.title}>
        {profileName ? `Good morning, ${profileName}.` : 'Good morning.'}
      </h1>
      <p style={viewStyles.subtitle}>
        {scanning
          ? 'Scanning sources…'
          : articles.length === 0
            ? showTutorial
              ? <>Let's get you set up.</>
              : <>No briefing yet. Add sources, set feed URLs, then press <strong style={{ color: 'var(--text)' }}>Morning Briefing</strong>.</>
            : <>Your briefing surfaced <strong style={{ color: 'var(--text)' }}>{articles.length} articles</strong> across {sourceCount} source{sourceCount === 1 ? '' : 's'} · {unread} unread.</>
        }
      </p>

      {showTutorial && (
        <OnboardingTutorial
          profile={profile}
          categories={categories}
          sources={sources}
          setView={setView}
          onAddSource={onAddSource}
        />
      )}

      {!showTutorial && (
        <div style={viewStyles.statStrip}>
          <div style={viewStyles.stat}>
            <span style={viewStyles.statLabel}>Fetched</span>
            <span style={viewStyles.statValue}>{articles.length}</span>
          </div>
          <div style={viewStyles.stat}>
            <span style={viewStyles.statLabel}>Unread</span>
            <span style={viewStyles.statValue}>{unread}</span>
          </div>
          <div style={viewStyles.stat}>
            <span style={viewStyles.statLabel}>Saved</span>
            <span style={viewStyles.statValue}>{articles.filter(a => a.bookmarked).length}</span>
          </div>
          <div style={{ ...viewStyles.stat, borderRight: 'none' }}>
            <span style={viewStyles.statLabel}>Last run</span>
            <span style={{ ...viewStyles.statValue, fontSize: 15, color: 'var(--text-2)', fontFamily: 'JetBrains Mono, monospace' }}>
              {relativeTime(lastRunAt)}
            </span>
          </div>
        </div>
      )}

      {errors.length > 0 && !scanning && (
        <div style={{
          border: '1px solid var(--border-strong)',
          background: 'var(--surface-2)',
          borderRadius: 'var(--radius)',
          padding: '10px 14px',
          marginBottom: 18,
          fontSize: 12.5,
          color: 'var(--text-2)',
          fontFamily: 'JetBrains Mono, monospace',
        }}>
          <strong style={{ color: 'var(--text)', fontWeight: 500 }}>
            {errors.length} source{errors.length === 1 ? '' : 's'} failed:
          </strong>{' '}
          {errors.map(e => e.sourceName).join(', ')}
        </div>
      )}

      {categories.length > 0 && (
        <div style={viewStyles.pills}>
          <button
            className={`pill ${activeCategory === 'all' ? 'active' : ''}`}
            onClick={() => setActiveCategory('all')}>
            <span>All</span>
            <span className="count">{articles.length}</span>
          </button>
          {categories.map(c => (
            <button key={c.id}
              className={`pill ${activeCategory === c.id ? 'active' : ''}`}
              onClick={() => setActiveCategory(c.id)}>
              {activeCategory !== c.id && <span className={`dot dot-${c.accent}`} />}
              <span>{c.label}</span>
              <span className="count">{countFor(c.id)}</span>
            </button>
          ))}
        </div>
      )}

      {scanning ? (
        viewMode === 'grid' ? (
          <div style={viewStyles.grid}>
            {Array.from({ length: 8 }).map((_, i) => <SkeletonCard key={i} />)}
          </div>
        ) : (
          <div style={viewStyles.list}>
            {Array.from({ length: 10 }).map((_, i) => (
              <div key={i} style={{ padding: '0 16px', height: 44, borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 14 }}>
                <div className="skel" style={{ width: 10, height: 10, borderRadius: '50%' }} />
                <div className="skel" style={{ width: 110, height: 10 }} />
                <div className="skel" style={{ flex: 1, height: 10 }} />
                <div className="skel" style={{ width: 60, height: 10 }} />
              </div>
            ))}
          </div>
        )
      ) : viewMode === 'grid' ? (
        <div style={viewStyles.grid}>
          {filtered.map((a, i) => (
            <Card key={a.id} article={a} i={i}
                  showAccent={showAccent} useSerif={useSerif}
                  onToggleRead={() => onToggleRead(a.id)}
                  onToggleBookmark={() => onToggleBookmark(a.id)} />
          ))}
        </div>
      ) : (
        <div style={viewStyles.list}>
          <div style={viewStyles.listHead}>
            <span />
            <span>Source</span>
            <span>Headline</span>
            <span>Category</span>
            <span style={{ textAlign: 'right' }}>Time</span>
            <span />
          </div>
          {filtered.map((a, i) => (
            <Row key={a.id} article={a} i={i}
                 onToggleRead={() => onToggleRead(a.id)}
                 onToggleBookmark={() => onToggleBookmark(a.id)} />
          ))}
        </div>
      )}
    </div>
  );
}
