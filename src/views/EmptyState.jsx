import { Icon } from '../components/Icon.jsx';

export function EmptyState({ icon, title, message }) {
  return (
    <div style={{
      border: '1px dashed var(--border-strong)', borderRadius: 'var(--radius)',
      padding: '60px 24px', textAlign: 'center',
      background: 'var(--surface)',
    }}>
      <div style={{ width: 44, height: 44, borderRadius: 10, background: 'var(--surface-2)', color: 'var(--text-3)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', marginBottom: 14 }}>
        <Icon name={icon} size={20} />
      </div>
      <h3 className="serif" style={{ fontSize: 18, margin: '0 0 6px', fontWeight: 500 }}>{title}</h3>
      <p style={{ color: 'var(--text-2)', fontSize: 13.5, margin: 0 }}>{message}</p>
    </div>
  );
}
