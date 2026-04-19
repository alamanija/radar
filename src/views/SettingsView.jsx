import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { SignedIn, SignedOut, SignInButton, UserButton, useAuth, useClerk, useUser } from '@clerk/clerk-react';
import { Icon } from '../components/Icon.jsx';
import { Toggle } from '../components/Toggle.jsx';
import { useUpdater } from '../hooks/useUpdater.js';

const settingsStyles = {
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
  blockHead: {
    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
    padding: '14px 18px',
    borderBottom: '1px solid var(--border)',
    background: 'var(--surface-2)',
  },
  blockTitle: { fontSize: 13, fontWeight: 500 },
  blockSub: { fontSize: 12, color: 'var(--text-3)' },
  settingRow: {
    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
    padding: '14px 18px',
    borderBottom: '1px solid var(--border)',
    fontSize: 13.5,
  },
};

function ApiKeyControl() {
  const [status, setStatus] = useState({ present: false, preview: null });
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const available = isTauri();

  const refresh = async () => {
    if (!available) return;
    try { setStatus(await invoke('anthropic_api_key_status')); }
    catch (e) { setError(String(e)); }
  };

  useEffect(() => { refresh(); }, []);

  const save = async () => {
    setBusy(true); setError(null);
    try {
      await invoke('set_anthropic_api_key', { key: draft });
      setDraft(''); setEditing(false);
      await refresh();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };

  const clear = async () => {
    setBusy(true); setError(null);
    try {
      await invoke('clear_anthropic_api_key');
      await refresh();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };

  if (!available) {
    return <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-3)' }}>desktop only</span>;
  }

  if (editing) {
    return (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <input type="password" value={draft} onChange={e => setDraft(e.target.value)}
               placeholder="sk-ant-…"
               style={{
                 width: 220, height: 28, padding: '0 8px',
                 border: '1px solid var(--border)', borderRadius: 'var(--radius)',
                 background: 'var(--surface)', color: 'var(--text)',
                 fontFamily: 'JetBrains Mono, monospace', fontSize: 12,
               }} />
        <button className="pill" onClick={save} disabled={busy || !draft.trim()}>
          {busy ? 'Saving…' : 'Save'}
        </button>
        <button className="pill" onClick={() => { setEditing(false); setDraft(''); setError(null); }}>
          Cancel
        </button>
        {error && <span style={{ fontSize: 11, color: 'var(--dot-rust)' }}>{error}</span>}
      </div>
    );
  }

  if (status.present) {
    return (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-2)' }}>
          {status.preview ?? '••••••'}
        </span>
        <button className="pill" onClick={() => setEditing(true)}>Replace</button>
        <button className="pill" onClick={clear} disabled={busy}>Clear</button>
      </div>
    );
  }

  return (
    <button className="pill" onClick={() => setEditing(true)}>Add key</button>
  );
}

function LensControl({ value, onChange }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value ?? '');

  if (!editing) {
    return (
      <button className="pill" onClick={() => { setDraft(value ?? ''); setEditing(true); }}>
        {value?.trim() ? 'Edit' : 'Add'}
      </button>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-end', width: '100%', maxWidth: 420 }}>
      <textarea
        value={draft}
        onChange={e => setDraft(e.target.value)}
        rows={4}
        placeholder="e.g. Brand studio founder focused on packaging and editorial illustration. Prefer craft over trend."
        style={{
          width: '100%',
          padding: '8px 10px',
          border: '1px solid var(--border)', borderRadius: 'var(--radius)',
          background: 'var(--surface)', color: 'var(--text)',
          fontFamily: 'inherit', fontSize: 13,
          resize: 'vertical',
        }}
        autoFocus
      />
      <div style={{ display: 'flex', gap: 6 }}>
        <button className="pill" onClick={() => { onChange(draft); setEditing(false); }}>Save</button>
        <button className="pill" onClick={() => setEditing(false)}>Cancel</button>
        {value?.trim() && (
          <button className="pill" onClick={() => { onChange(''); setEditing(false); }} style={{ color: 'var(--dot-rust)' }}>
            Clear
          </button>
        )}
      </div>
    </div>
  );
}

const textInput = {
  height: 28, padding: '0 8px',
  border: '1px solid var(--border)', borderRadius: 'var(--radius)',
  background: 'var(--surface)', color: 'var(--text)',
  fontSize: 13, fontFamily: 'inherit', width: 240,
};

function UpdatesControl() {
  const {
    state, version, notes, progress, error, supported,
    check, downloadAndInstall, restartToApply,
  } = useUpdater();
  const [appVersion, setAppVersion] = useState(null);

  useEffect(() => {
    if (!supported) return;
    import('@tauri-apps/api/app')
      .then(m => m.getVersion())
      .then(setAppVersion)
      .catch(() => {});
  }, [supported]);

  if (!supported) {
    return (
      <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-3)' }}>
        desktop only
      </span>
    );
  }

  if (state === 'available') {
    return (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexWrap: 'wrap', justifyContent: 'flex-end' }}>
        <span style={{ fontSize: 12, color: 'var(--text-2)' }}>
          {version ? `v${version} available` : 'Update available'}
        </span>
        <button className="pill" onClick={downloadAndInstall}>Download</button>
      </div>
    );
  }

  if (state === 'downloading') {
    const pct = progress != null ? Math.round(progress * 100) : null;
    return (
      <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-2)' }}>
        Downloading{pct != null ? ` ${pct}%` : '…'}
      </span>
    );
  }

  if (state === 'ready') {
    return (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <span style={{ fontSize: 12, color: 'var(--text-2)' }}>Ready to install</span>
        <button className="pill active" onClick={restartToApply}>Restart now</button>
      </div>
    );
  }

  if (state === 'checking') {
    return (
      <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-3)' }}>
        Checking…
      </span>
    );
  }

  if (state === 'error') {
    return (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <span style={{ fontSize: 12, color: 'var(--dot-rust)' }} title={error}>
          Check failed
        </span>
        <button className="pill" onClick={check}>Retry</button>
      </div>
    );
  }

  // 'idle' and 'uptodate'
  const meta = [
    appVersion ? `v${appVersion}` : null,
    state === 'uptodate' ? 'up to date' : null,
  ].filter(Boolean).join(' · ');
  return (
    <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
      {meta && (
        <span style={{
          fontFamily: 'JetBrains Mono, monospace',
          fontSize: 12,
          color: 'var(--text-3)',
          fontVariantNumeric: 'tabular-nums',
        }}>
          {meta}
        </span>
      )}
      <button className="pill" onClick={check}>Check for updates</button>
    </div>
  );
}

// Modal sign-in. `mode="modal"` renders Clerk's full `<SignIn />` in a
// same-origin overlay (no iframe), and the in-page after-sign-in redirect
// wired up on `ClerkProvider` keeps the session alive across the flow.
function SignInPanel() {
  return (
    <SignInButton mode="modal">
      <button className="pill">Sign in</button>
    </SignInButton>
  );
}

function AccountControl() {
  const { user } = useUser();
  const { isLoaded } = useAuth();
  const clerk = useClerk();

  // Clerk loads asynchronously. `isLoaded=false` is the normal state for
  // roughly the first 100-500ms after mount. If it stays false indefinitely,
  // something's blocking Clerk from initialising (most commonly unverified
  // production DNS, a publishable key from the wrong instance, or the
  // Tauri origin not being whitelisted in the Clerk dashboard).
  if (!isLoaded) {
    // Rough hint at why it might be stuck. `clerk.frontendApi` is the
    // domain the SDK is trying to reach; if it looks wrong, that's a clue.
    const api = clerk?.frontendApi ?? '(unknown)';
    return (
      <span
        style={{
          fontFamily: 'JetBrains Mono, monospace',
          fontSize: 11.5,
          color: 'var(--text-3)',
        }}
        title={`Clerk hasn't initialised. Frontend API: ${api}`}
      >
        Loading auth…
      </span>
    );
  }

  return (
    <>
      <SignedIn>
        <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
          {user && (
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', minWidth: 0 }}>
              {user.fullName && (
                <span style={{ fontSize: 13, fontWeight: 500 }}>{user.fullName}</span>
              )}
              <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 11.5, color: 'var(--text-3)' }}>
                {user.primaryEmailAddress?.emailAddress}
              </span>
            </div>
          )}
          <UserButton afterSignOutUrl="/" />
        </div>
      </SignedIn>
      <SignedOut>
        <SignInPanel />
      </SignedOut>
    </>
  );
}

export function SettingsView({
  theme, setTheme,
  tweaks, setTweaks,
  profile, setProfile,
  runAtLaunch, setRunAtLaunch,
  scheduleEnabled, setScheduleEnabled,
  scheduleTime, setScheduleTime,
  account,
}) {
  const signedIn = !!account;
  const row = (label, sub, control) => (
    <div style={settingsStyles.settingRow}>
      <div>
        <div style={{ fontWeight: 500 }}>{label}</div>
        {sub && <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>{sub}</div>}
      </div>
      {control}
    </div>
  );

  const updateProfile = (changes) => setProfile(p => ({ ...p, ...changes }));

  return (
    <div style={settingsStyles.wrap}>
      <h1 className="serif" style={settingsStyles.title}>Settings</h1>
      <p style={settingsStyles.subtitle}>
        Radar runs locally by default. Sign in to sync your sources, categories, profile, and read state across devices.
      </p>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Account</span>
          <span style={settingsStyles.blockSub}>Enables cross-device sync</span>
        </div>
        {row(
          signedIn ? 'Signed in' : 'Sign in',
          signedIn
            ? 'Signed in via Clerk. Sync runs automatically.'
            : 'Sync is off. You\'ll stay signed out and local-only until you connect.',
          <AccountControl />
        )}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Profile</span>
          <span style={settingsStyles.blockSub}>Shown in the greeting and sidebar</span>
        </div>
        {row('Name', 'How Radar addresses you.',
          <input
            style={textInput}
            placeholder="e.g. Maya"
            value={profile.name}
            onChange={e => updateProfile({ name: e.target.value })}
          />
        )}
        {row('Role', 'Short tagline under your name.',
          <input
            style={textInput}
            placeholder="e.g. Freelance · Branding"
            value={profile.role}
            onChange={e => updateProfile({ role: e.target.value })}
          />
        )}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Briefing</span>
          <span style={settingsStyles.blockSub}>Controls when and how Radar scans</span>
        </div>
        {row('Run at launch', 'Trigger a fresh Morning Briefing every time you open Radar.',
          <Toggle on={runAtLaunch} onChange={() => setRunAtLaunch(v => !v)} />
        )}
        {row('Schedule', 'Fire a briefing at this local time each day. Runs only while Radar is open; missed slots fire on next launch.',
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <input
              type="time"
              value={scheduleTime}
              onChange={e => setScheduleTime(e.target.value)}
              disabled={!scheduleEnabled}
              style={{
                height: 28, padding: '0 8px',
                border: '1px solid var(--border)', borderRadius: 'var(--radius)',
                background: 'var(--surface)', color: 'var(--text)',
                fontFamily: 'JetBrains Mono, monospace', fontSize: 12,
                opacity: scheduleEnabled ? 1 : 0.5,
              }}
            />
            <Toggle on={scheduleEnabled} onChange={() => setScheduleEnabled(v => !v)} />
          </div>
        )}
        {row('Staleness threshold', 'Not wired — articles aren\'t cached across briefings yet.',
          <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-3)' }}>—</span>
        )}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Curation</span>
          <span style={settingsStyles.blockSub}>Anthropic Claude filters and summarizes</span>
        </div>
        {row('Model', 'claude-opus-4-7', <span style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: 12, color: 'var(--text-2)' }}>Opus 4.7</span>)}
        {row('Professional lens', 'Extra context passed to Claude so it leans into what you care about.',
          <LensControl value={profile.lens} onChange={v => updateProfile({ lens: v })} />
        )}
        {row('API key', 'Stored in the system keychain. Sent only to api.anthropic.com.', <ApiKeyControl />)}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Appearance</span>
          <span style={settingsStyles.blockSub}>Local preferences</span>
        </div>
        {row('Theme', null,
          <div style={{ display: 'flex', gap: 4 }}>
            {['light', 'dark'].map(t => (
              <button key={t} className={`pill ${theme === t ? 'active' : ''}`} onClick={() => setTheme(t)} style={{ textTransform: 'capitalize' }}>
                <Icon name={t === 'light' ? 'sun' : 'moon'} size={12} /> {t}
              </button>
            ))}
          </div>
        )}
        {row('Accent bars on cards', 'Show a colored edge indicating category.', <Toggle on={tweaks.showAccent} onChange={() => setTweaks(t => ({ ...t, showAccent: !t.showAccent }))} />)}
        {row('Serif headlines', 'Use Newsreader for article titles.', <Toggle on={tweaks.useSerif} onChange={() => setTweaks(t => ({ ...t, useSerif: !t.useSerif }))} />)}
        {row('Density', 'Compact packs more rows in list view.',
          <div style={{ display: 'flex', gap: 4 }}>
            {['cozy', 'compact'].map(d => (
              <button key={d} className={`pill ${tweaks.density === d ? 'active' : ''}`} onClick={() => setTweaks(t => ({ ...t, density: d }))} style={{ textTransform: 'capitalize' }}>{d}</button>
            ))}
          </div>
        )}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Updates</span>
          <span style={settingsStyles.blockSub}>Signed releases from GitHub</span>
        </div>
        {row(
          'App updates',
          'Radar checks on launch and when you ask. Downloads are signed and verified before install.',
          <UpdatesControl />
        )}
      </div>

      <div style={settingsStyles.block}>
        <div style={settingsStyles.blockHead}>
          <span style={settingsStyles.blockTitle}>Integrations</span>
          <span style={settingsStyles.blockSub}>Not yet shipped</span>
        </div>
        {row(
          'Gmail — newsletter inbox',
          'Read-only, label-scoped. Requires a Google OAuth client ID which isn\'t bundled yet.',
          <button className="pill" disabled style={{ opacity: 0.5, cursor: 'not-allowed' }}>
            Connect
          </button>
        )}
      </div>
    </div>
  );
}
