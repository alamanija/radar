import { useEffect, useMemo, useRef, useState } from 'react';
import { Icon } from './Icon.jsx';
import { openExternal } from '../external.js';

const styles = {
  scrim: {
    position: 'fixed', inset: 0, zIndex: 200,
    background: 'rgba(0,0,0,0.35)',
    display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
    paddingTop: '10vh',
  },
  panel: {
    width: 600, maxWidth: '90vw', maxHeight: '70vh',
    background: 'var(--surface)',
    border: '1px solid var(--border-strong)',
    borderRadius: 10,
    boxShadow: '0 20px 60px rgba(0,0,0,0.25)',
    display: 'flex', flexDirection: 'column',
    overflow: 'hidden',
  },
  inputRow: {
    display: 'flex', alignItems: 'center', gap: 10,
    padding: '14px 16px', borderBottom: '1px solid var(--border)',
    color: 'var(--text-2)',
  },
  input: {
    flex: 1, border: 'none', outline: 'none',
    background: 'transparent', color: 'var(--text)',
    fontSize: 15, fontFamily: 'inherit',
  },
  kbd: {
    fontSize: 10.5, fontFamily: 'JetBrains Mono, monospace',
    color: 'var(--text-3)', padding: '1px 5px',
    border: '1px solid var(--border)', borderRadius: 3,
    background: 'var(--surface-2)',
  },
  results: { overflowY: 'auto', flex: 1 },
  sectionHead: {
    fontSize: 10.5, textTransform: 'uppercase', letterSpacing: '0.08em',
    color: 'var(--text-3)', fontWeight: 500,
    padding: '10px 16px 6px',
    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
  },
  row: {
    display: 'flex', alignItems: 'center', gap: 12,
    padding: '9px 16px',
    cursor: 'pointer',
  },
  rowActive: {
    background: 'var(--accent-soft)',
    color: 'var(--accent-text)',
  },
  empty: {
    padding: '40px 16px', textAlign: 'center',
    color: 'var(--text-3)', fontSize: 13,
  },
};

function SectionHeader({ label, count }) {
  return (
    <div style={styles.sectionHead}>
      <span>{label}</span>
      <span style={{ fontFamily: 'JetBrains Mono, monospace' }}>{count}</span>
    </div>
  );
}

function ResultRow({ active, onHover, onClick, icon, title, subtitle }) {
  return (
    <div
      style={{ ...styles.row, ...(active ? styles.rowActive : {}) }}
      onMouseEnter={onHover}
      onClick={onClick}
    >
      <span style={{ width: 16, display: 'inline-flex', justifyContent: 'center', flexShrink: 0 }}>
        {icon}
      </span>
      <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 }}>
        <span style={{
          color: 'inherit',
          fontSize: 13.5, fontWeight: 500,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{title}</span>
        <span style={{
          color: active ? 'var(--accent-text)' : 'var(--text-3)',
          fontSize: 11.5, fontFamily: 'JetBrains Mono, monospace',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{subtitle}</span>
      </div>
    </div>
  );
}

export function SearchPalette({ articles, archives, sources, onClose, onNavigate }) {
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef(null);

  useEffect(() => { inputRef.current?.focus(); }, []);

  // Dedupe articles by id (current wins over archived snapshots).
  const allArticles = useMemo(() => {
    const byId = new Map();
    for (const a of articles) byId.set(a.id, a);
    for (const b of archives) {
      for (const a of b.articles) {
        if (!byId.has(a.id)) byId.set(a.id, a);
      }
    }
    return [...byId.values()];
  }, [articles, archives]);

  const { articleHits, sourceHits } = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return { articleHits: [], sourceHits: [] };
    const match = (s) => typeof s === 'string' && s.toLowerCase().includes(q);
    return {
      articleHits: allArticles
        .filter(a => match(a.title) || match(a.source) || match(a.summary))
        .slice(0, 12),
      sourceHits: sources
        .filter(s => match(s.name) || match(s.category) || match(s.feedUrl))
        .slice(0, 8),
    };
  }, [query, allArticles, sources]);

  const total = articleHits.length + sourceHits.length;

  useEffect(() => { setActiveIndex(0); }, [query]);

  const pick = (idx) => {
    if (idx < articleHits.length) {
      const a = articleHits[idx];
      if (a.url) openExternal(a.url);
    } else {
      const s = sourceHits[idx - articleHits.length];
      if (s) onNavigate('sources');
    }
    onClose();
  };

  const onKeyDown = (e) => {
    if (e.key === 'Escape') { e.preventDefault(); onClose(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); setActiveIndex(i => Math.min(i + 1, Math.max(total - 1, 0))); }
    else if (e.key === 'ArrowUp')   { e.preventDefault(); setActiveIndex(i => Math.max(i - 1, 0)); }
    else if (e.key === 'Enter')     { e.preventDefault(); pick(activeIndex); }
  };

  return (
    <div style={styles.scrim} onClick={onClose}>
      <div style={styles.panel} onClick={(e) => e.stopPropagation()} onKeyDown={onKeyDown}>
        <div style={styles.inputRow}>
          <Icon name="search" size={16} />
          <input
            ref={inputRef}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search articles, sources…"
            style={styles.input}
          />
          <span style={styles.kbd}>esc</span>
        </div>

        <div style={styles.results}>
          {query.trim() === '' ? (
            <div style={styles.empty}>Type to search articles and sources.</div>
          ) : total === 0 ? (
            <div style={styles.empty}>No matches.</div>
          ) : (
            <>
              {articleHits.length > 0 && <SectionHeader label="Articles" count={articleHits.length} />}
              {articleHits.map((a, i) => (
                <ResultRow
                  key={`a-${a.id}`}
                  active={i === activeIndex}
                  onHover={() => setActiveIndex(i)}
                  onClick={() => pick(i)}
                  icon={<span className={`dot dot-${a.accent}`} />}
                  title={a.title}
                  subtitle={`${a.source} · ${a.categoryLabel} · ${a.published}`}
                />
              ))}
              {sourceHits.length > 0 && <SectionHeader label="Sources" count={sourceHits.length} />}
              {sourceHits.map((s, i) => {
                const idx = articleHits.length + i;
                return (
                  <ResultRow
                    key={`s-${s.id}`}
                    active={idx === activeIndex}
                    onHover={() => setActiveIndex(idx)}
                    onClick={() => pick(idx)}
                    icon={<Icon name="feed" size={14} />}
                    title={s.name}
                    subtitle={`${s.category} · ${s.feedUrl ?? 'no feed URL'}`}
                  />
                );
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
