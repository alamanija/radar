import { Icon } from './Icon.jsx';
import { Toggle } from './Toggle.jsx';

const accents = [
  { name: 'Camel', light: '#9A7B5B', dark: '#C4A882', soft: '#EEE5D8', softDark: '#2A2419', text: '#6B5338', textDark: '#D9BE93' },
  { name: 'Olive', light: '#6F7A3A', dark: '#A2A468', soft: '#EAECDB', softDark: '#22241A', text: '#4B5327', textDark: '#C3C586' },
  { name: 'Rust', light: '#A1573A', dark: '#CB8862', soft: '#F1DED1', softDark: '#2A1E17', text: '#6E3B26', textDark: '#E0A184' },
  { name: 'Slate', light: '#556170', dark: '#8896A6', soft: '#E1E4E8', softDark: '#1D2127', text: '#3A4450', textDark: '#A8B4C2' },
  { name: 'Ink', light: '#222428', dark: '#C8CBD1', soft: '#E3E4E6', softDark: '#1D1F23', text: '#1A1A1A', textDark: '#E8E6E3' },
];

const style = {
  panel: {
    position: 'fixed', right: 20, bottom: 20,
    width: 300,
    background: 'var(--surface)',
    border: '1px solid var(--border-strong)',
    borderRadius: 10,
    boxShadow: '0 12px 40px rgba(0,0,0,0.18), 0 2px 6px rgba(0,0,0,0.08)',
    zIndex: 100,
    overflow: 'hidden',
  },
  head: {
    display: 'flex', alignItems: 'center', gap: 8,
    padding: '12px 14px',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
  },
  title: { fontSize: 12, fontWeight: 500, textTransform: 'uppercase', letterSpacing: '0.08em', color: 'var(--text-2)' },
  body: { padding: 14, display: 'flex', flexDirection: 'column', gap: 14 },
  group: { display: 'flex', flexDirection: 'column', gap: 6 },
  label: { fontSize: 11, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', fontWeight: 500 },
  seg: { display: 'flex', border: '1px solid var(--border)', borderRadius: 5, padding: 2, background: 'var(--surface-2)' },
  segBtn: { flex: 1, padding: '5px 8px', fontSize: 12, borderRadius: 3, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 6, color: 'var(--text-2)' },
  segBtnActive: { background: 'var(--surface)', color: 'var(--text)', boxShadow: '0 0 0 1px var(--border)' },
  swatches: { display: 'flex', gap: 6 },
  swatch: (c, active) => ({
    width: 24, height: 24, borderRadius: 5,
    background: c,
    cursor: 'pointer',
    boxShadow: active ? '0 0 0 2px var(--surface), 0 0 0 3px var(--text)' : '0 0 0 1px var(--border)',
  }),
  toggleRow: { display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: 12.5 },
};

function Seg({ value, options, onChange }) {
  return (
    <div style={style.seg}>
      {options.map(o => (
        <button key={o.value} style={{ ...style.segBtn, ...(value === o.value ? style.segBtnActive : {}) }}
          onClick={() => onChange(o.value)}>
          {o.icon && <Icon name={o.icon} size={12} />}
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function TweaksPanel({ tweaks, setTweaks, theme, setTheme, viewMode, setViewMode, sidebarCollapsed, setSidebarCollapsed, onClose }) {
  const selectAccent = (a) => setTweaks(t => ({ ...t, accent: a.name }));

  return (
    <div style={style.panel}>
      <div style={style.head}>
        <Icon name="sliders" size={14} />
        <span style={style.title}>Tweaks</span>
        <button className="iconbtn" style={{ marginLeft: 'auto', width: 24, height: 24 }} onClick={onClose}>
          <Icon name="x" size={13} />
        </button>
      </div>
      <div style={style.body}>
        <div style={style.group}>
          <span style={style.label}>Theme</span>
          <Seg value={theme} onChange={setTheme}
            options={[{ value: 'light', label: 'Light', icon: 'sun' }, { value: 'dark', label: 'Dark', icon: 'moon' }]} />
        </div>
        <div style={style.group}>
          <span style={style.label}>View</span>
          <Seg value={viewMode} onChange={setViewMode}
            options={[{ value: 'grid', label: 'Grid', icon: 'grid' }, { value: 'list', label: 'List', icon: 'list' }]} />
        </div>
        <div style={style.group}>
          <span style={style.label}>Density</span>
          <Seg value={tweaks.density} onChange={(v) => setTweaks(t => ({ ...t, density: v }))}
            options={[{ value: 'cozy', label: 'Cozy' }, { value: 'compact', label: 'Compact' }]} />
        </div>
        <div style={style.group}>
          <span style={style.label}>Accent</span>
          <div style={style.swatches}>
            {accents.map(a => (
              <div key={a.name} title={a.name}
                style={style.swatch(theme === 'dark' ? a.dark : a.light, tweaks.accent === a.name)}
                onClick={() => selectAccent(a)} />
            ))}
          </div>
        </div>
        <div style={style.toggleRow}>
          <span>Serif headlines</span>
          <Toggle on={tweaks.useSerif} onChange={() => setTweaks(t => ({ ...t, useSerif: !t.useSerif }))} />
        </div>
        <div style={style.toggleRow}>
          <span>Accent bars on cards</span>
          <Toggle on={tweaks.showAccent} onChange={() => setTweaks(t => ({ ...t, showAccent: !t.showAccent }))} />
        </div>
        <div style={style.toggleRow}>
          <span>Sidebar collapsed</span>
          <Toggle on={sidebarCollapsed} onChange={() => setSidebarCollapsed(c => !c)} />
        </div>
      </div>
    </div>
  );
}
