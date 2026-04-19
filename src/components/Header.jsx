import { Icon } from './Icon.jsx';

const headerStyles = {
  bar: {
    height: 56,
    borderBottom: '1px solid var(--border)',
    display: 'flex', alignItems: 'center',
    padding: '0 24px',
    gap: 12,
    background: 'var(--bg)',
    position: 'sticky', top: 0, zIndex: 10,
  },
  breadcrumb: {
    display: 'flex', alignItems: 'center', gap: 8,
    fontSize: 13, color: 'var(--text-2)',
    minWidth: 0, flex: 1,
    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
  },
  crumbMain: { color: 'var(--text)', fontWeight: 500, whiteSpace: 'nowrap' },
  date: { fontSize: 12, color: 'var(--text-3)', marginLeft: 4, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 },
  actions: { display: 'flex', alignItems: 'center', gap: 6 },
  briefBtn: {
    display: 'inline-flex', alignItems: 'center', gap: 8,
    height: 32, padding: '0 14px 0 12px',
    borderRadius: 6,
    background: 'var(--text)', color: 'var(--bg)',
    fontSize: 13, fontWeight: 500,
    transition: 'opacity 120ms',
  },
  search: {
    display: 'flex', alignItems: 'center', gap: 8,
    width: 280, minWidth: 120, height: 32,
    border: '1px solid var(--border)',
    borderRadius: 6,
    padding: '0 10px',
    background: 'var(--surface)',
    color: 'var(--text-3)',
    fontSize: 13,
    flexShrink: 1,
    whiteSpace: 'nowrap', overflow: 'hidden',
  },
  kbd: {
    marginLeft: 'auto',
    fontSize: 10.5, fontFamily: 'JetBrains Mono, monospace',
    color: 'var(--text-3)',
    padding: '1px 5px',
    border: '1px solid var(--border)',
    borderRadius: 3,
    background: 'var(--surface-2)',
  },
};

export function Header({
  view, sidebarCollapsed, toggleSidebar,
  viewMode, setViewMode, theme, setTheme,
  onBriefing, scanning, onOpenSearch,
}) {
  const labels = {
    briefing: 'Morning Briefing',
    saved: 'Saved',
    archive: 'Archive',
    sources: 'Sources',
    settings: 'Settings',
  };
  const dateStr = new Date().toLocaleDateString('en-US', { weekday: 'long', month: 'long', day: 'numeric' });

  return (
    <div style={headerStyles.bar}>
      <div className="iconbtn" onClick={toggleSidebar} title="Toggle sidebar">
        <Icon name={sidebarCollapsed ? 'chevron-right' : 'chevron-left'} size={16} />
      </div>
      <div style={headerStyles.breadcrumb}>
        <Icon name={
          view === 'briefing' ? 'sparkles' :
          view === 'saved' ? 'bookmark' :
          view === 'archive' ? 'archive' :
          view === 'sources' ? 'feed' : 'settings'
        } size={14} />
        <span style={headerStyles.crumbMain}>{labels[view]}</span>
        {view === 'briefing' && <span style={headerStyles.date}>· {dateStr}</span>}
      </div>

      <div style={{ ...headerStyles.search, cursor: 'pointer' }} onClick={onOpenSearch}>
        <Icon name="search" size={14} />
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>Search articles, sources…</span>
        <span style={headerStyles.kbd}>⌘K</span>
      </div>

      <div style={headerStyles.actions}>
        {view === 'briefing' && (
          <>
            <div style={{ display: 'flex', border: '1px solid var(--border)', borderRadius: 6, padding: 2, background: 'var(--surface)' }}>
              <button className={`iconbtn ${viewMode === 'grid' ? 'active' : ''}`} style={{ width: 28, height: 28 }} onClick={() => setViewMode('grid')} title="Grid view">
                <Icon name="grid" size={14} />
              </button>
              <button className={`iconbtn ${viewMode === 'list' ? 'active' : ''}`} style={{ width: 28, height: 28 }} onClick={() => setViewMode('list')} title="List view">
                <Icon name="list" size={14} />
              </button>
            </div>
            <button style={{ ...headerStyles.briefBtn, opacity: scanning ? 0.85 : 1 }} onClick={onBriefing} disabled={scanning}>
              <Icon name="refresh" size={14} className={scanning ? 'spin' : ''} />
              <span>{scanning ? 'Scanning…' : 'Morning Briefing'}</span>
            </button>
          </>
        )}
        <div className="iconbtn" onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')} title="Toggle theme">
          <Icon name={theme === 'light' ? 'moon' : 'sun'} size={15} />
        </div>
      </div>
    </div>
  );
}
