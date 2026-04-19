import { useEffect, useState } from 'react';
import { Icon } from '../components/Icon.jsx';
import { Toggle } from '../components/Toggle.jsx';
import { relativeTime } from '../time.js';

const sourcesStyles = {
  wrap: { padding: '24px 32px 80px', maxWidth: 880, margin: '0 auto' },
  title: { fontSize: 28, fontWeight: 500, letterSpacing: '-0.02em', marginBottom: 6 },
  subtitle: { color: 'var(--text-2)', fontSize: 14, marginBottom: 22 },
  block: {
    background: 'var(--surface)',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    overflow: 'hidden',
    marginBottom: 22,
  },
  srcRow: {
    display: 'grid',
    gridTemplateColumns: '20px 1fr 140px 110px 44px',
    gap: 14,
    alignItems: 'center',
    padding: '12px 18px',
    borderBottom: '1px solid var(--border)',
    fontSize: 13.5,
  },
  editRow: {
    display: 'grid',
    gridTemplateColumns: '20px 1fr 140px auto',
    gap: 10,
    alignItems: 'center',
    padding: '12px 18px',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
  },
  input: {
    height: 28,
    padding: '0 8px',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    color: 'var(--text)',
    fontSize: 13,
    fontFamily: 'inherit',
    width: '100%',
  },
  select: {
    height: 28,
    padding: '0 8px',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    color: 'var(--text)',
    fontSize: 13,
    fontFamily: 'inherit',
  },
  feedUrl: {
    fontSize: 11.5,
    color: 'var(--text-3)',
    fontFamily: 'JetBrains Mono, monospace',
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
  },
  healthOk:    { color: '#6B8E4E',      fontFamily: 'JetBrains Mono, monospace', fontSize: 11.5 },
  healthWarn:  { color: '#B07E3C',      fontFamily: 'JetBrains Mono, monospace', fontSize: 11.5 },
  healthStale: { color: 'var(--text-3)', fontFamily: 'JetBrains Mono, monospace', fontSize: 11.5 },
  validationError: {
    fontSize: 11,
    color: 'var(--dot-rust)',
    marginTop: 6,
    gridColumn: '2 / -1',
  },
};

function isValidFeedUrl(u) {
  if (!u) return true; // empty is OK — means "no URL yet"
  try {
    const parsed = new URL(u);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    return false;
  }
}

function EditRow({ draft, setDraft, onSave, onCancel, onDelete, categories, saveLabel = 'Save' }) {
  const [error, setError] = useState(null);

  const attemptSave = () => {
    if (!draft.name.trim()) { setError('Name is required'); return; }
    if (!draft.category) { setError('Pick a category'); return; }
    if (!isValidFeedUrl(draft.feedUrl.trim())) { setError('Feed URL must start with http:// or https://'); return; }
    setError(null);
    onSave({ ...draft, name: draft.name.trim(), feedUrl: draft.feedUrl.trim() || null });
  };

  return (
    <div style={{
      padding: '14px 18px',
      borderBottom: '1px solid var(--border)',
      background: 'var(--surface-2)',
    }}>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 180px auto', gap: 10, alignItems: 'center' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <input
            style={sourcesStyles.input}
            placeholder="Source name (e.g. The Dieline)"
            value={draft.name}
            onChange={e => setDraft({ ...draft, name: e.target.value })}
            autoFocus
          />
          <input
            style={{ ...sourcesStyles.input, fontFamily: 'JetBrains Mono, monospace', fontSize: 12 }}
            placeholder="https://example.com/feed/"
            value={draft.feedUrl}
            onChange={e => setDraft({ ...draft, feedUrl: e.target.value })}
          />
        </div>
        <select
          style={sourcesStyles.select}
          value={draft.category}
          onChange={e => setDraft({ ...draft, category: e.target.value })}
        >
          {categories.length === 0 && <option value="">— no categories —</option>}
          {categories.map(c => <option key={c.id} value={c.id}>{c.label}</option>)}
        </select>
        <div style={{ display: 'flex', gap: 6 }}>
          <button className="pill" onClick={attemptSave}>{saveLabel}</button>
          <button className="pill" onClick={onCancel}>Cancel</button>
          {onDelete && (
            <button className="pill" onClick={onDelete} style={{ color: 'var(--dot-rust)' }}>Delete</button>
          )}
        </div>
      </div>
      {error && <div style={{ fontSize: 11, color: 'var(--dot-rust)', marginTop: 8 }}>{error}</div>}
    </div>
  );
}

export function SourcesView({ sources, categories, toggleSource, addSource, updateSource, deleteSource, intent, clearIntent }) {
  const [filter, setFilter] = useState('all');
  const [editing, setEditing] = useState(null); // null | { id: 'new' | number, draft }
  const enabled = sources.filter(s => s.enabled).length;
  const healthLabel = (h) => h === 'ok' ? 'Live' : h === 'warn' ? 'Delayed' : 'Stale';
  const healthStyle = (h) => h === 'ok' ? sourcesStyles.healthOk : h === 'warn' ? sourcesStyles.healthWarn : sourcesStyles.healthStale;
  const categoryLabel = (id) => categories.find(c => c.id === id)?.label ?? '— uncategorized';

  const filtered = sources.filter(s =>
    filter === 'all' || (filter === 'on' && s.enabled) || (filter === 'custom' && !s.isDefault)
  );

  const emptyDraft = () => ({ name: '', category: categories[0]?.id ?? '', feedUrl: '' });
  const startAdd = () => setEditing({ id: 'new', draft: emptyDraft() });

  // Consume a one-shot "open add form" intent from the sidebar's + button.
  useEffect(() => {
    if (intent === 'add' && categories.length > 0) {
      startAdd();
    }
    if (intent) clearIntent?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intent]);

  const startEdit = (s) => setEditing({
    id: s.id,
    draft: { name: s.name, category: s.category, feedUrl: s.feedUrl ?? '' },
  });
  const cancel = () => setEditing(null);

  const saveAdd = (draft) => {
    addSource(draft);
    setEditing(null);
  };
  const saveEdit = (draft) => {
    updateSource(editing.id, draft);
    setEditing(null);
  };
  const handleDelete = () => {
    deleteSource(editing.id);
    setEditing(null);
  };

  return (
    <div style={sourcesStyles.wrap}>
      <h1 className="serif" style={sourcesStyles.title}>Sources</h1>
      <p style={sourcesStyles.subtitle}>{enabled} of {sources.length} enabled. Add any RSS/Atom feed — Radar will fetch and summarize it.</p>

      <div style={{ display: 'flex', gap: 6, marginBottom: 16, alignItems: 'center' }}>
        <button className={`pill ${filter === 'all' ? 'active' : ''}`} onClick={() => setFilter('all')}>All <span className="count">{sources.length}</span></button>
        <button className={`pill ${filter === 'on' ? 'active' : ''}`} onClick={() => setFilter('on')}>Enabled <span className="count">{enabled}</span></button>
        <button className={`pill ${filter === 'custom' ? 'active' : ''}`} onClick={() => setFilter('custom')}>Custom <span className="count">{sources.filter(s => !s.isDefault).length}</span></button>
        <button
          className="pill"
          style={{ marginLeft: 'auto', background: 'var(--text)', color: 'var(--bg)', borderColor: 'var(--text)' }}
          onClick={startAdd}
          disabled={editing?.id === 'new' || categories.length === 0}
          title={categories.length === 0 ? 'Add a category first' : undefined}
        >
          <Icon name="plus" size={13} /> Add source
        </button>
      </div>

      <div style={sourcesStyles.block}>
        <div style={{ ...sourcesStyles.srcRow, background: 'var(--surface-2)', fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.08em', color: 'var(--text-3)', fontWeight: 500 }}>
          <span />
          <span>Source</span>
          <span>Category</span>
          <span>Last fetch</span>
          <span />
        </div>

        {editing?.id === 'new' && (
          <EditRow
            draft={editing.draft}
            setDraft={d => setEditing({ ...editing, draft: d })}
            onSave={saveAdd}
            onCancel={cancel}
            categories={categories}
            saveLabel="Add"
          />
        )}

        {filtered.map((s, i) => {
          const isLast = i === filtered.length - 1 && editing?.id !== 'new';
          if (editing?.id === s.id) {
            return (
              <EditRow
                key={s.id}
                draft={editing.draft}
                setDraft={d => setEditing({ ...editing, draft: d })}
                onSave={saveEdit}
                onCancel={cancel}
                onDelete={!s.isDefault ? handleDelete : undefined}
                categories={categories}
              />
            );
          }
          return (
            <div key={s.id} style={{
              ...sourcesStyles.srcRow,
              borderBottom: isLast ? 'none' : sourcesStyles.srcRow.borderBottom,
            }}>
              <Toggle on={s.enabled} onChange={() => toggleSource(s.id)} />
              <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
                <span style={{ fontWeight: 500, color: 'var(--text)' }}>{s.name}</span>
                <span style={sourcesStyles.feedUrl} title={s.feedUrl ?? ''}>
                  {s.feedUrl ?? '— no feed URL set'}
                </span>
              </div>
              <span style={{ fontSize: 12.5, color: 'var(--text-2)' }}>{categoryLabel(s.category)}</span>
              <span style={healthStyle(s.health)}>● {healthLabel(s.health)} · {relativeTime(s.lastFetchAt)}</span>
              <button
                className="iconbtn"
                style={{ width: 26, height: 26, justifySelf: 'end' }}
                onClick={() => startEdit(s)}
                title="Edit"
              >
                <Icon name="dot-v" size={14} />
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
