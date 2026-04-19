import { useState } from 'react';
import { Icon } from '../components/Icon.jsx';
import { Card } from '../components/Card.jsx';
import { EmptyState } from './EmptyState.jsx';
import { viewStyles } from './BriefingView.jsx';

const archiveStyles = {
  list: {
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    background: 'var(--surface)',
    overflow: 'hidden',
  },
  row: {
    display: 'grid',
    gridTemplateColumns: '1fr 80px 80px 80px 20px',
    alignItems: 'center',
    height: 52,
    padding: '0 18px',
    borderBottom: '1px solid var(--border)',
    fontSize: 13.5,
    cursor: 'pointer',
    gap: 14,
  },
  rowDate: {
    color: 'var(--text)',
    fontWeight: 500,
  },
  rowMuted: {
    fontFamily: 'JetBrains Mono, monospace',
    fontSize: 12,
    color: 'var(--text-3)',
    textAlign: 'right',
  },
  backBtn: {
    display: 'inline-flex', alignItems: 'center', gap: 6,
    fontSize: 13, color: 'var(--text-2)',
    marginBottom: 14, cursor: 'pointer',
  },
  detailMeta: {
    fontFamily: 'JetBrains Mono, monospace',
    fontSize: 12,
    color: 'var(--text-3)',
    marginBottom: 18,
  },
};

function formatRunAt(iso) {
  const d = new Date(iso);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const yd = new Date(now); yd.setDate(yd.getDate() - 1);
  const isYesterday = d.toDateString() === yd.toDateString();

  const time = d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  if (sameDay) return `Today · ${time}`;
  if (isYesterday) return `Yesterday · ${time}`;
  return d.toLocaleDateString('en-US', {
    weekday: 'long', month: 'long', day: 'numeric',
  }) + ` · ${time}`;
}

export function ArchiveView({ archives, useSerif }) {
  const [selectedId, setSelectedId] = useState(null);
  const selected = archives.find(a => a.id === selectedId);

  if (archives.length === 0) {
    return (
      <div style={viewStyles.wrap}>
        <h1 className="serif" style={viewStyles.title}>Archive</h1>
        <p style={viewStyles.subtitle}>Past briefings. Goes back 90 days by default.</p>
        <EmptyState
          icon="archive"
          title="Your archive begins with your first briefing"
          message="Every completed Morning Briefing is kept here so you can look back and see what the week was really about." />
      </div>
    );
  }

  if (selected) {
    const unread = selected.articles.filter(a => !a.read).length;
    return (
      <div style={viewStyles.wrap}>
        <div style={archiveStyles.backBtn} onClick={() => setSelectedId(null)}>
          <Icon name="chevron-left" size={14} />
          <span>All briefings</span>
        </div>
        <h1 className="serif" style={viewStyles.title}>{formatRunAt(selected.runAt)}</h1>
        <p style={archiveStyles.detailMeta}>
          {selected.articles.length} article{selected.articles.length === 1 ? '' : 's'} ·
          {' '}{unread} unread
          {selected.errors?.length > 0 && ` · ${selected.errors.length} source${selected.errors.length === 1 ? '' : 's'} failed`}
        </p>
        <div style={viewStyles.grid}>
          {selected.articles.map((a, i) => (
            <Card
              key={a.id}
              article={a}
              i={i}
              showAccent={true}
              useSerif={useSerif}
              onToggleRead={() => {}}
              onToggleBookmark={() => {}}
            />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div style={viewStyles.wrap}>
      <h1 className="serif" style={viewStyles.title}>Archive</h1>
      <p style={viewStyles.subtitle}>
        {archives.length} briefing{archives.length === 1 ? '' : 's'}. Newest first.
      </p>

      <div style={archiveStyles.list}>
        {archives.map((b, i) => {
          const unread = b.articles.filter(a => !a.read).length;
          const saved = b.articles.filter(a => a.bookmarked).length;
          return (
            <div key={b.id}
                 style={{
                   ...archiveStyles.row,
                   borderBottom: i === archives.length - 1 ? 'none' : archiveStyles.row.borderBottom,
                 }}
                 onClick={() => setSelectedId(b.id)}>
              <span style={archiveStyles.rowDate}>{formatRunAt(b.runAt)}</span>
              <span style={archiveStyles.rowMuted}>{b.articles.length} items</span>
              <span style={archiveStyles.rowMuted}>{unread} unread</span>
              <span style={archiveStyles.rowMuted}>{saved} saved</span>
              <Icon name="chevron-right" size={14} />
            </div>
          );
        })}
      </div>
    </div>
  );
}
