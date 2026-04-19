import { useState } from 'react';
import { Icon } from './Icon.jsx';
import { openExternal } from '../external.js';

const cardStyles = {
  card: {
    position: 'relative',
    background: 'var(--surface)',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius)',
    cursor: 'pointer',
    transition: 'border-color 140ms, transform 140ms, box-shadow 140ms',
    display: 'flex', flexDirection: 'column',
    minHeight: 180,
    overflow: 'hidden',
  },
  body: {
    padding: 'var(--density-pad)',
    display: 'flex', flexDirection: 'column', flex: 1,
    minWidth: 0,
  },
  thumb: {
    height: 96,
    borderBottom: '1px solid var(--border)',
    position: 'relative',
    overflow: 'hidden',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
  },
  thumbLabel: {
    position: 'absolute',
    bottom: 8, left: 10,
    fontFamily: 'JetBrains Mono, monospace',
    fontSize: 9.5,
    textTransform: 'uppercase',
    letterSpacing: '0.08em',
    color: 'var(--text-3)',
    background: 'var(--surface)',
    padding: '2px 6px',
    borderRadius: 3,
    border: '1px solid var(--border)',
  },
  accentBar: {
    position: 'absolute', left: 0, top: 0, bottom: 0, width: 3,
    borderTopLeftRadius: 'var(--radius)',
    borderBottomLeftRadius: 'var(--radius)',
    zIndex: 2,
  },
  meta: {
    display: 'flex', alignItems: 'center', gap: 8,
    fontSize: 11.5, color: 'var(--text-3)',
    marginBottom: 10,
    fontFamily: 'JetBrains Mono, monospace',
    letterSpacing: '0.02em',
    flexWrap: 'nowrap', minWidth: 0,
    whiteSpace: 'nowrap',
  },
  source: { color: 'var(--text-2)', fontWeight: 500, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 },
  headline: {
    fontSize: 17,
    lineHeight: 1.35,
    color: 'var(--text)',
    marginBottom: 10,
    fontWeight: 500,
    letterSpacing: '-0.01em',
    textWrap: 'pretty',
  },
  summary: {
    fontSize: 13.5, lineHeight: 1.55,
    color: 'var(--text-2)',
    marginBottom: 14,
    display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical',
    overflow: 'hidden',
  },
  footer: {
    display: 'flex', alignItems: 'center', gap: 10,
    marginTop: 'auto',
    paddingTop: 10,
    borderTop: '1px dashed var(--border)',
    fontSize: 12, color: 'var(--text-3)',
  },
  readDot: {
    width: 7, height: 7, borderRadius: '50%',
    background: 'var(--accent)', flexShrink: 0,
  },
};

export function Card({ article, showAccent, onToggleRead, onToggleBookmark, useSerif, i }) {
  const [hover, setHover] = useState(false);
  const style = {
    ...cardStyles.card,
    borderColor: hover ? 'var(--border-strong)' : 'var(--border)',
    transform: hover ? 'translateY(-1px)' : 'translateY(0)',
    boxShadow: hover ? 'var(--shadow-md)' : 'none',
    animationDelay: `${Math.min(i, 12) * 40}ms`,
    opacity: article.read ? 0.7 : 1,
  };
  return (
    <div className="fade-up" style={style}
         onMouseEnter={() => setHover(true)}
         onMouseLeave={() => setHover(false)}
         onClick={onToggleRead}>
      {showAccent && <div style={{ ...cardStyles.accentBar, background: `var(--dot-${article.accent})` }} />}
      <Thumb article={article} />
      <div style={cardStyles.body}>
        <div style={cardStyles.meta}>
          <span className={`dot dot-${article.accent}`} />
          <span style={cardStyles.source}>{article.source}</span>
          <span style={{ flexShrink: 0 }}>·</span>
          <span style={{ textTransform: 'uppercase', fontSize: 10.5, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0, flexShrink: 1 }}>{article.categoryLabel}</span>
          <span style={{ marginLeft: 'auto', whiteSpace: 'nowrap', flexShrink: 0, paddingLeft: 6 }}>{article.published}</span>
        </div>
        <h3 style={{ ...cardStyles.headline, fontFamily: useSerif ? "'Newsreader', Georgia, serif" : 'inherit', fontSize: useSerif ? 19 : 17, fontWeight: useSerif ? 500 : 500 }}>
          {article.title}
        </h3>
        <p style={cardStyles.summary}>{article.summary}</p>
        <div style={cardStyles.footer}>
          {!article.read && <span style={cardStyles.readDot} title="Unread" />}
          <span>{article.read ? 'Read' : 'New'}</span>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 2 }}>
            <button className="iconbtn" style={{ width: 26, height: 26 }}
                    onClick={(e) => { e.stopPropagation(); onToggleBookmark(); }}
                    title={article.bookmarked ? 'Remove bookmark' : 'Bookmark'}>
              <Icon name={article.bookmarked ? 'bookmark-filled' : 'bookmark'} size={13} />
            </button>
            <button className="iconbtn" style={{ width: 26, height: 26 }}
                    onClick={(e) => { e.stopPropagation(); openExternal(article.url); }}
                    title="Open in browser"
                    disabled={!article.url}>
              <Icon name="external" size={13} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Thumb({ article }) {
  const patterns = ['stripes', 'grid', 'dots', 'diag', 'arcs'];
  const pattern = patterns[article.id % patterns.length];
  const color = `var(--dot-${article.accent})`;
  const bg = {
    stripes: `repeating-linear-gradient(90deg, ${color} 0 1px, transparent 1px 14px)`,
    grid: `repeating-linear-gradient(0deg, ${color} 0 1px, transparent 1px 18px), repeating-linear-gradient(90deg, ${color} 0 1px, transparent 1px 18px)`,
    dots: `radial-gradient(${color} 1px, transparent 1.4px) 0 0 / 14px 14px`,
    diag: `repeating-linear-gradient(45deg, ${color} 0 1px, transparent 1px 12px)`,
    arcs: `radial-gradient(circle at 0 100%, ${color} 0 1px, transparent 1.5px 24px), radial-gradient(circle at 100% 100%, ${color} 0 1px, transparent 1.5px 24px)`,
  }[pattern];
  return (
    <div style={{
      ...cardStyles.thumb,
      backgroundColor: 'var(--surface-2)',
      backgroundImage: bg,
      opacity: 0.9,
    }}>
      <span style={{
        fontFamily: "'Newsreader', Georgia, serif",
        fontSize: 32,
        color: color,
        opacity: 0.55,
        letterSpacing: '-0.02em',
        fontWeight: 500,
        fontStyle: 'italic',
        background: 'var(--surface)',
        padding: '2px 14px',
        border: '1px solid var(--border)',
        borderRadius: 4,
      }}>
        {article.source.split(' ').map(w => w[0]).join('').slice(0,2)}
      </span>
      <span style={cardStyles.thumbLabel}>cover · {article.categoryLabel}</span>
    </div>
  );
}

const rowStyles = {
  row: {
    display: 'grid',
    gridTemplateColumns: '20px 160px 1fr 120px 80px 60px',
    alignItems: 'center',
    height: 'var(--row-h)',
    padding: '0 16px',
    borderBottom: '1px solid var(--border)',
    fontSize: 13.5,
    cursor: 'pointer',
    gap: 14,
  },
};

export function Row({ article, onToggleRead, onToggleBookmark, i }) {
  const [hover, setHover] = useState(false);
  return (
    <div className="fade-up" style={{
      ...rowStyles.row,
      background: hover ? 'var(--surface-2)' : 'transparent',
      opacity: article.read ? 0.6 : 1,
      animationDelay: `${Math.min(i, 16) * 25}ms`,
    }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={onToggleRead}>
      <span className={`dot dot-${article.accent}`} />
      <span style={{ color: 'var(--text-2)', fontSize: 12.5, fontFamily: 'JetBrains Mono, monospace', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{article.source}</span>
      <span style={{ color: 'var(--text)', fontWeight: article.read ? 400 : 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {article.title}
      </span>
      <span style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{article.categoryLabel}</span>
      <span style={{ color: 'var(--text-3)', fontSize: 12, fontFamily: 'JetBrains Mono, monospace', textAlign: 'right' }}>{article.published}</span>
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 2 }}>
        <button className="iconbtn" style={{ width: 26, height: 26, opacity: article.bookmarked || hover ? 1 : 0 }}
                onClick={(e) => { e.stopPropagation(); onToggleBookmark(); }}>
          <Icon name={article.bookmarked ? 'bookmark-filled' : 'bookmark'} size={13} />
        </button>
      </div>
    </div>
  );
}

export function SkeletonCard() {
  return (
    <div style={{ ...cardStyles.card, cursor: 'default', minHeight: 180 }}>
      <div className="skel" style={{ height: 96, borderRadius: 0 }} />
      <div style={{ padding: 'var(--density-pad)', flex: 1 }}>
        <div style={{ display: 'flex', gap: 8, marginBottom: 12 }}>
          <div className="skel" style={{ width: 80, height: 10 }} />
          <div className="skel" style={{ width: 50, height: 10 }} />
        </div>
        <div className="skel" style={{ height: 14, marginBottom: 8, width: '92%' }} />
        <div className="skel" style={{ height: 14, marginBottom: 14, width: '70%' }} />
        <div className="skel" style={{ height: 10, marginBottom: 6 }} />
        <div className="skel" style={{ height: 10, width: '60%' }} />
      </div>
    </div>
  );
}
