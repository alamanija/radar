import { useState } from 'react';
import { Icon } from './Icon.jsx';
import { SyncIndicator } from './SyncIndicator.jsx';

const sidebarStyles = {
  aside: {
    background: 'var(--surface-2)',
    borderRight: '1px solid var(--border)',
    display: 'flex',
    flexDirection: 'column',
    overflow: 'hidden',
    minWidth: 0,
  },
  brand: {
    display: 'flex', alignItems: 'center', gap: 10,
    padding: '18px 18px 16px',
  },
  brandMark: {
    width: 28, height: 28, borderRadius: 6,
    background: 'var(--text)', color: 'var(--bg)',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
  },
  brandName: { fontSize: 18, fontWeight: 500, letterSpacing: '-0.01em' },
  scroll: { flex: 1, overflowY: 'auto', padding: '4px 10px 16px' },
  section: { padding: '10px 10px 6px', fontSize: 11, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', fontWeight: 500, display: 'flex', justifyContent: 'space-between', alignItems: 'center' },
  nav: { display: 'flex', flexDirection: 'column', gap: 1 },
  item: {
    display: 'flex', alignItems: 'center', gap: 10,
    height: 32, padding: '0 10px',
    borderRadius: 5,
    fontSize: 13.5,
    color: 'var(--text-2)',
    cursor: 'pointer',
    position: 'relative',
    userSelect: 'none',
  },
  itemActive: {
    background: 'var(--surface)',
    color: 'var(--text)',
    boxShadow: 'inset 0 0 0 1px var(--border)',
  },
  count: { marginLeft: 'auto', fontSize: 11, color: 'var(--text-3)', fontVariantNumeric: 'tabular-nums' },
  sourceRow: {
    display: 'flex', alignItems: 'center', gap: 10,
    height: 30, padding: '0 10px',
    borderRadius: 5,
    fontSize: 13,
    color: 'var(--text-2)',
    cursor: 'pointer',
  },
  footer: {
    padding: 12,
    borderTop: '1px solid var(--border)',
    display: 'flex', alignItems: 'center', gap: 10,
  },
  avatar: {
    width: 28, height: 28, borderRadius: '50%',
    background: 'var(--accent-soft)', color: 'var(--accent-text)',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    fontSize: 12, fontWeight: 600,
  },
};

function SidebarItem({ icon, label, count, active, onClick, dot }) {
  const [hover, setHover] = useState(false);
  const style = {
    ...sidebarStyles.item,
    ...(active ? sidebarStyles.itemActive : {}),
    ...(hover && !active ? { background: 'var(--surface)' } : {}),
  };
  return (
    <div style={style} onClick={onClick} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      {dot ? <span className={`dot dot-${dot}`} /> : <Icon name={icon} size={15} />}
      <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
      {count != null && <span style={sidebarStyles.count}>{count}</span>}
    </div>
  );
}

export function Sidebar({ view, setView, sources, toggleSource, categories, articles, profile, account, activeCategory, setActiveCategory, onAddSource }) {
  const unread = articles.filter(a => !a.read).length;
  const saved = articles.filter(a => a.bookmarked).length;
  const countFor = (catId) => articles.filter(a => a.category === catId).length;

  // Display-name fallback chain: the user's edited profile name wins, then
  // Clerk's fullName/firstName, then the email local-part, then a prompt.
  const emailPrefix = account?.email ? account.email.split('@')[0] : '';
  const displayName =
    (profile?.name?.trim()) ||
    (account?.name?.trim()) ||
    emailPrefix ||
    '';
  const displayRole =
    (profile?.role?.trim()) ||
    (account?.email) ||
    'Settings → Profile';
  const initial = displayName ? displayName.charAt(0).toUpperCase() : '?';
  return (
    <aside style={sidebarStyles.aside}>
      <div style={sidebarStyles.brand}>
        <div style={sidebarStyles.brandMark}>
          <Icon name="radar" size={16} />
        </div>
        <div className="serif" style={sidebarStyles.brandName}>Radar</div>
      </div>

      <div style={sidebarStyles.scroll}>
        <div style={sidebarStyles.nav}>
          <SidebarItem icon="sparkles" label="Briefing" count={unread} active={view === 'briefing'} onClick={() => setView('briefing')} />
          <SidebarItem icon="bookmark" label="Saved" count={saved} active={view === 'saved'} onClick={() => setView('saved')} />
          <SidebarItem icon="archive" label="Archive" active={view === 'archive'} onClick={() => setView('archive')} />
          <SidebarItem icon="feed" label="Sources" count={sources.length} active={view === 'sources'} onClick={() => setView('sources')} />
          <SidebarItem icon="tag" label="Categories" count={categories.length} active={view === 'categories'} onClick={() => setView('categories')} />
          <SidebarItem icon="settings" label="Settings" active={view === 'settings'} onClick={() => setView('settings')} />
        </div>

        {categories.length > 0 && (
          <>
            <div style={{ ...sidebarStyles.section, marginTop: 18 }}>
              <span>Categories</span>
            </div>
            <div style={sidebarStyles.nav}>
              {categories.map(c => (
                <SidebarItem
                  key={c.id}
                  dot={c.accent}
                  label={c.label}
                  count={countFor(c.id)}
                  active={view === 'briefing' && activeCategory === c.id}
                  onClick={() => { setView('briefing'); setActiveCategory(c.id); }}
                />
              ))}
            </div>
          </>
        )}

        <div style={{ ...sidebarStyles.section, marginTop: 18 }}>
          <span>Sources</span>
          <span
            onClick={onAddSource}
            style={{ cursor: 'pointer', display: 'inline-flex', color: 'var(--text-3)' }}
            title="Add source"
          >
            <Icon name="plus" size={13} />
          </span>
        </div>
        <div style={sidebarStyles.nav}>
          {sources.slice(0, 8).map(s => (
            <div key={s.id} style={{ ...sidebarStyles.sourceRow, opacity: s.enabled ? 1 : 0.45 }}
                 onClick={() => toggleSource(s.id)}>
              <Icon name={s.enabled ? 'circle-filled' : 'circle'} size={10} />
              <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.name}</span>
            </div>
          ))}
          <div style={{ ...sidebarStyles.sourceRow, color: 'var(--text-3)' }} onClick={() => setView('sources')}>
            <Icon name="chevron-right" size={12} />
            <span>View all ({sources.length})</span>
          </div>
        </div>
      </div>

      {account && <SyncIndicator />}

      <div style={sidebarStyles.footer}>
        {account?.imageUrl ? (
          <img
            src={account.imageUrl}
            alt=""
            referrerPolicy="no-referrer"
            style={{
              ...sidebarStyles.avatar,
              background: 'var(--surface-2)',
              objectFit: 'cover',
            }}
          />
        ) : (
          <div style={sidebarStyles.avatar}>{initial}</div>
        )}
        <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1, cursor: 'pointer' }}
             onClick={() => setView('settings')}
             title="Edit profile in Settings">
          <span style={{
            fontSize: 13, fontWeight: 500,
            color: displayName ? 'var(--text)' : 'var(--text-3)',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {displayName || 'Set your name'}
          </span>
          <span style={{
            fontSize: 11, color: 'var(--text-3)',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {displayRole}
          </span>
        </div>
      </div>
    </aside>
  );
}
