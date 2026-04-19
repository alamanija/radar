import { useState } from 'react';
import { Icon } from '../components/Icon.jsx';
import { viewStyles } from './BriefingView.jsx';

const ACCENTS = ['olive', 'rust', 'slate', 'plum', 'sand', 'ink'];

const styles = {
  wrap: { padding: '24px 32px 80px', maxWidth: 880, margin: '0 auto' },
  title: viewStyles.title,
  subtitle: viewStyles.subtitle,
  block: {
    background: 'var(--surface)',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    overflow: 'hidden',
    marginBottom: 22,
  },
  head: {
    display: 'grid',
    gridTemplateColumns: '20px 1fr 2fr 80px 44px',
    gap: 14, alignItems: 'center',
    padding: '10px 18px',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
    fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.08em',
    color: 'var(--text-3)', fontWeight: 500,
  },
  row: {
    display: 'grid',
    gridTemplateColumns: '20px 1fr 2fr 80px 44px',
    gap: 14, alignItems: 'center',
    padding: '12px 18px',
    borderBottom: '1px solid var(--border)',
    fontSize: 13.5,
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
  swatchGroup: { display: 'flex', gap: 6 },
  swatch: (accent, selected) => ({
    width: 24, height: 24, borderRadius: '50%',
    background: `var(--dot-${accent})`,
    cursor: 'pointer',
    boxShadow: selected
      ? '0 0 0 2px var(--surface), 0 0 0 3px var(--text)'
      : '0 0 0 1px var(--border)',
  }),
};

function Swatches({ value, onChange }) {
  return (
    <div style={styles.swatchGroup}>
      {ACCENTS.map(a => (
        <div key={a}
             style={styles.swatch(a, value === a)}
             title={a}
             onClick={() => onChange(a)} />
      ))}
    </div>
  );
}

function EditRow({ draft, setDraft, onSave, onCancel, onDelete, sourceCount, saveLabel = 'Save' }) {
  const [error, setError] = useState(null);
  const attemptSave = () => {
    if (!draft.label.trim()) { setError('Label is required'); return; }
    if (!draft.accent) { setError('Pick an accent'); return; }
    setError(null);
    onSave({
      label: draft.label.trim(),
      description: draft.description.trim(),
      accent: draft.accent,
    });
  };
  return (
    <div style={{
      padding: '14px 18px',
      borderBottom: '1px solid var(--border)',
      background: 'var(--surface-2)',
    }}>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 2fr auto', gap: 10, alignItems: 'center' }}>
        <input
          style={styles.input}
          placeholder="Category name"
          value={draft.label}
          onChange={e => setDraft({ ...draft, label: e.target.value })}
          autoFocus
        />
        <input
          style={styles.input}
          placeholder="Description — helps Claude pick this category"
          value={draft.description}
          onChange={e => setDraft({ ...draft, description: e.target.value })}
        />
        <div style={{ display: 'flex', gap: 6 }}>
          <button className="pill" onClick={attemptSave}>{saveLabel}</button>
          <button className="pill" onClick={onCancel}>Cancel</button>
          {onDelete && (
            <button
              className="pill"
              onClick={onDelete}
              disabled={sourceCount > 0}
              title={sourceCount > 0 ? `${sourceCount} source(s) use this category` : undefined}
              style={sourceCount > 0 ? {} : { color: 'var(--dot-rust)' }}
            >
              Delete
            </button>
          )}
        </div>
      </div>
      <div style={{ marginTop: 10, display: 'flex', alignItems: 'center', gap: 12 }}>
        <span style={{ fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.08em', color: 'var(--text-3)', fontWeight: 500 }}>Accent</span>
        <Swatches value={draft.accent} onChange={(v) => setDraft({ ...draft, accent: v })} />
      </div>
      {error && <div style={{ fontSize: 11, color: 'var(--dot-rust)', marginTop: 8 }}>{error}</div>}
    </div>
  );
}

export function CategoriesView({ categories, sources, addCategory, updateCategory, deleteCategory }) {
  const [editing, setEditing] = useState(null);

  const sourceCountFor = (catId) => sources.filter(s => s.category === catId).length;

  const emptyDraft = () => ({ label: '', description: '', accent: 'olive' });
  const startAdd = () => setEditing({ id: 'new', draft: emptyDraft() });
  const startEdit = (c) => setEditing({ id: c.id, draft: { label: c.label, description: c.description, accent: c.accent } });
  const cancel = () => setEditing(null);

  const saveAdd = (draft) => { addCategory(draft); setEditing(null); };
  const saveEdit = (draft) => { updateCategory(editing.id, draft); setEditing(null); };
  const handleDelete = () => {
    if (sourceCountFor(editing.id) > 0) return; // button should be disabled, but defensive
    deleteCategory(editing.id);
    setEditing(null);
  };

  return (
    <div style={styles.wrap}>
      <h1 className="serif" style={styles.title}>Categories</h1>
      <p style={styles.subtitle}>
        {categories.length === 0
          ? 'No categories yet. Add at least one before you can create sources or run a Claude-enhanced briefing.'
          : `${categories.length} categor${categories.length === 1 ? 'y' : 'ies'}. Descriptions flow into the Claude prompt so the model knows how to bucket articles.`}
      </p>

      <div style={{ display: 'flex', marginBottom: 16 }}>
        <button
          className="pill"
          style={{ marginLeft: 'auto', background: 'var(--text)', color: 'var(--bg)', borderColor: 'var(--text)' }}
          onClick={startAdd}
          disabled={editing?.id === 'new'}
        >
          <Icon name="plus" size={13} /> Add category
        </button>
      </div>

      <div style={styles.block}>
        <div style={styles.head}>
          <span />
          <span>Name</span>
          <span>Description</span>
          <span>Sources</span>
          <span />
        </div>

        {editing?.id === 'new' && (
          <EditRow
            draft={editing.draft}
            setDraft={d => setEditing({ ...editing, draft: d })}
            onSave={saveAdd}
            onCancel={cancel}
            sourceCount={0}
            saveLabel="Add"
          />
        )}

        {categories.map((c, i) => {
          const isLast = i === categories.length - 1 && editing?.id !== 'new';
          const usedBy = sourceCountFor(c.id);
          if (editing?.id === c.id) {
            return (
              <EditRow
                key={c.id}
                draft={editing.draft}
                setDraft={d => setEditing({ ...editing, draft: d })}
                onSave={saveEdit}
                onCancel={cancel}
                onDelete={handleDelete}
                sourceCount={usedBy}
              />
            );
          }
          return (
            <div key={c.id} style={{ ...styles.row, borderBottom: isLast ? 'none' : styles.row.borderBottom }}>
              <span className={`dot dot-${c.accent}`} />
              <span style={{ fontWeight: 500, color: 'var(--text)' }}>{c.label}</span>
              <span style={{ color: 'var(--text-2)', fontSize: 12.5, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={c.description}>
                {c.description || <span style={{ color: 'var(--text-3)' }}>— no description</span>}
              </span>
              <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-3)' }}>
                {usedBy}
              </span>
              <button
                className="iconbtn"
                style={{ width: 26, height: 26, justifySelf: 'end' }}
                onClick={() => startEdit(c)}
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
