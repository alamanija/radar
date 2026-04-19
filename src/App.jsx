import { useState, useEffect, useRef, useMemo } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { useAuth, useUser } from '@clerk/clerk-react';
import { RADAR_DATA } from './data.js';
import { loadSnapshot, setItem } from './storage.js';
import { makeSyncFetch, SYNC_BASE } from './sync.js';
import { installSyncQueue } from './syncQueue.js';
import { useSyncedResource } from './hooks/useSyncedResource.js';
import { Sidebar } from './components/Sidebar.jsx';
import { Header } from './components/Header.jsx';
import { TweaksPanel } from './components/Tweaks.jsx';
import { SearchPalette } from './components/SearchPalette.jsx';
import { BriefingView } from './views/BriefingView.jsx';
import { SavedView } from './views/SavedView.jsx';
import { ArchiveView } from './views/ArchiveView.jsx';
import { SourcesView } from './views/SourcesView.jsx';
import { CategoriesView } from './views/CategoriesView.jsx';
import { SettingsView } from './views/SettingsView.jsx';

const DEFAULT_PREFS = {
  theme: 'light',
  viewMode: 'grid',
  density: 'cozy',
  accent: 'Camel',
  useSerif: true,
  showAccent: true,
  sidebarCollapsed: false,
  view: 'briefing',
  runAtLaunch: false,
  scheduleEnabled: false,
  scheduleTime: '07:30',
};

const DEFAULT_PROFILE = { name: '', role: '', lens: '' };

const ARCHIVE_CAP = 90;

const ACCENTS = {
  Camel:  { l: '#9A7B5B', d: '#C4A882', sl: '#EEE5D8', sd: '#2A2419', tl: '#6B5338', td: '#D9BE93' },
  Olive:  { l: '#6F7A3A', d: '#A2A468', sl: '#EAECDB', sd: '#22241A', tl: '#4B5327', td: '#C3C586' },
  Rust:   { l: '#A1573A', d: '#CB8862', sl: '#F1DED1', sd: '#2A1E17', tl: '#6E3B26', td: '#E0A184' },
  Slate:  { l: '#556170', d: '#8896A6', sl: '#E1E4E8', sd: '#1D2127', tl: '#3A4450', td: '#A8B4C2' },
  Ink:    { l: '#222428', d: '#C8CBD1', sl: '#E3E4E6', sd: '#1D1F23', tl: '#1A1A1A', td: '#E8E6E3' },
};

export default function App() {
  const [view, setView] = useState(DEFAULT_PREFS.view);
  const [theme, setTheme] = useState(DEFAULT_PREFS.theme);
  const [viewMode, setViewMode] = useState(DEFAULT_PREFS.viewMode);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(DEFAULT_PREFS.sidebarCollapsed);
  const [tweaks, setTweaks] = useState({
    density: DEFAULT_PREFS.density,
    accent: DEFAULT_PREFS.accent,
    useSerif: DEFAULT_PREFS.useSerif,
    showAccent: DEFAULT_PREFS.showAccent,
  });
  const [activeCategory, setActiveCategory] = useState('all');
  const [scanning, setScanning] = useState(false);
  const [articles, setArticles] = useState([]);
  const [sources, setSources] = useState(RADAR_DATA.sources);
  const [categories, setCategories] = useState(RADAR_DATA.categories);
  const [profile, setProfile] = useState(DEFAULT_PROFILE);
  const [runAtLaunch, setRunAtLaunch] = useState(DEFAULT_PREFS.runAtLaunch);
  const [scheduleEnabled, setScheduleEnabled] = useState(DEFAULT_PREFS.scheduleEnabled);
  const [scheduleTime, setScheduleTime] = useState(DEFAULT_PREFS.scheduleTime);
  const [sourcesIntent, setSourcesIntent] = useState(null);
  // article_states ride this ref rather than a useState so rapid toggles
  // don't re-render the whole app — state application happens at mutation
  // points (toggle + briefing completion + pull) instead of reactively.
  const articleStatesRef = useRef(new Map());

  // Clerk owns identity. `account` is derived from Clerk's hooks — non-null
  // once Clerk finishes loading and the user is signed in. `syncFetch` is
  // memoised against `getToken` so the sync effects only re-run when the
  // token-fetcher's identity actually changes, not on every render.
  const { isLoaded: authLoaded, isSignedIn, getToken } = useAuth();
  const { user: clerkUser } = useUser();
  const account = useMemo(() => {
    if (!authLoaded || !isSignedIn || !clerkUser) return null;
    return {
      id: clerkUser.id,
      email: clerkUser.primaryEmailAddress?.emailAddress ?? '',
      name: clerkUser.fullName ?? clerkUser.firstName ?? '',
      imageUrl: clerkUser.hasImage ? clerkUser.imageUrl : null,
    };
  }, [authLoaded, isSignedIn, clerkUser]);
  const syncFetch = useMemo(() => makeSyncFetch(getToken), [getToken]);

  // Install the persistent offline queue once per app load. Re-calling it
  // with a new token-getter / account is safe; it just refreshes the wiring
  // without re-attaching event listeners. Tagging the active account id
  // prevents a queued write from one user draining against another user's
  // token after a sign-out/sign-in on the same device.
  useEffect(() => {
    installSyncQueue({ getToken, base: SYNC_BASE, accountId: account?.id ?? null });
  }, [getToken, account?.id]);
  const [tweaksOpen, setTweaksOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [briefingErrors, setBriefingErrors] = useState([]);
  const [archives, setArchives] = useState([]);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const onKey = (e) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key === 'k') {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // Hydrate from disk on mount. Save effects below are gated behind `ready`
  // so we never overwrite persisted state with the defaults above.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const snap = await loadSnapshot();
      if (cancelled) return;
      if (snap.prefs) {
        const p = { ...DEFAULT_PREFS, ...snap.prefs };
        setView(p.view);
        setTheme(p.theme);
        setViewMode(p.viewMode);
        setSidebarCollapsed(p.sidebarCollapsed);
        setRunAtLaunch(p.runAtLaunch);
        setScheduleEnabled(p.scheduleEnabled);
        setScheduleTime(p.scheduleTime);
        setTweaks({
          density: p.density, accent: p.accent,
          useSerif: p.useSerif, showAccent: p.showAccent,
        });
      }
      if (snap.profile && typeof snap.profile === 'object') {
        setProfile({ ...DEFAULT_PROFILE, ...snap.profile });
      }
      // Hydrate even on empty arrays — an empty persisted list is a legitimate
      // state (e.g. user deleted all their sources) and must not silently
      // revert to the seed.
      const effectiveCategories = Array.isArray(snap.categories) ? snap.categories : RADAR_DATA.categories;
      if (Array.isArray(snap.categories)) setCategories(snap.categories);

      // Migrate any legacy source.category label ("Packaging") → matching id ("packaging").
      if (Array.isArray(snap.sources)) {
        const byLabel = Object.fromEntries(effectiveCategories.map(c => [c.label, c.id]));
        const byId = new Set(effectiveCategories.map(c => c.id));
        const migrated = snap.sources.map(s => {
          if (byId.has(s.category)) return s;
          if (byLabel[s.category]) return { ...s, category: byLabel[s.category] };
          return s; // orphaned id — leave as-is; Rust resolve() will treat it as uncategorized.
        });
        setSources(migrated);
      }
      if (Array.isArray(snap.articles)) setArticles(snap.articles);
      if (Array.isArray(snap.archives)) setArchives(snap.archives);
      setReady(true);
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    document.documentElement.setAttribute('data-density', tweaks.density);
  }, [theme, tweaks.density]);

  useEffect(() => {
    const a = ACCENTS[tweaks.accent] || ACCENTS.Camel;
    const isDark = theme === 'dark';
    const root = document.documentElement;
    root.style.setProperty('--accent', isDark ? a.d : a.l);
    root.style.setProperty('--accent-soft', isDark ? a.sd : a.sl);
    root.style.setProperty('--accent-text', isDark ? a.td : a.tl);
  }, [theme, tweaks.accent]);

  useEffect(() => {
    if (!ready) return;
    setItem('prefs', {
      view, theme, viewMode, sidebarCollapsed,
      runAtLaunch, scheduleEnabled, scheduleTime,
      ...tweaks,
    });
  }, [ready, view, theme, viewMode, sidebarCollapsed, runAtLaunch, scheduleEnabled, scheduleTime, tweaks]);

  useEffect(() => {
    if (!ready) return;
    setItem('profile', profile);
  }, [ready, profile]);

  const profileSync = useSyncedResource({
    value: profile, setValue: setProfile,
    account, ready, label: 'profile',
    syncFetch, endpoint: '/sync/profile',
    buildPushBody: (p) => ({
      name: p.name || null,
      role: p.role || null,
      lens: p.lens || null,
    }),
    fromRemote: (r) => ({
      name: r.name ?? '', role: r.role ?? '', lens: r.lens ?? '',
    }),
    stableKey: (x) => JSON.stringify({
      name: x.name ?? '', role: x.role ?? '', lens: x.lens ?? '',
    }),
    isEmpty: (p) => !p.name && !p.role && !p.lens,
  });

  const categoriesSync = useSyncedResource({
    value: categories, setValue: setCategories,
    account, ready, label: 'categories',
    syncFetch, endpoint: '/sync/categories',
    buildPushBody: (c) => ({ categories: c }),
    fromRemote: (snap) => snap.categories,
    stableKey: (xs) => JSON.stringify(xs),
    isEmpty: (xs) => xs.length === 0,
  });

  // Only user-editable fields travel over sync. `lastFetchAt` + `health` are
  // per-device observations and stay local.
  const sourceWire = (s) => ({
    id: s.id,
    name: s.name,
    category: s.category,
    feedUrl: s.feedUrl ?? null,
    enabled: !!s.enabled,
    isDefault: !!s.isDefault,
  });

  const sourcesSync = useSyncedResource({
    value: sources, setValue: setSources,
    account, ready, label: 'sources',
    syncFetch, endpoint: '/sync/sources',
    buildPushBody: (xs) => ({ sources: xs.map(sourceWire) }),
    // On pull, rebuild the local list from the server's shape but preserve
    // this device's observed freshness/health for sources it has seen before.
    fromRemote: (snap, prev) => {
      const healthById = new Map((prev ?? []).map(s => [s.id, s]));
      return snap.sources.map(r => {
        const local = healthById.get(r.id);
        return {
          ...r,
          feedUrl: r.feedUrl ?? null,
          lastFetchAt: local?.lastFetchAt ?? null,
          health: local?.health ?? 'ok',
        };
      });
    },
    // Compare on wire shape only — a briefing bumping lastFetchAt shouldn't
    // trigger a no-op push.
    stableKey: (xs) => JSON.stringify(xs.map(sourceWire)),
    isEmpty: (xs) => xs.length === 0,
  });

  // Article states — bypass useSyncedResource. Per-row PATCH semantics, so
  // there's no "full list" to diff; we pull on sign-in, apply to articles,
  // then seed any local-only read/bookmark flags the server doesn't know
  // about. Further mutations go through toggleRead/toggleBookmark, which
  // fire PATCHes individually.
  useEffect(() => {
    if (!ready || !account) return;
    let cancelled = false;
    (async () => {
      try {
        const resp = await syncFetch('/sync/article-states');
        if (cancelled) return;
        if (!resp.ok) throw new Error(`pull article states: ${resp.status}`);
        const { states = [] } = await resp.json();
        const serverMap = new Map(
          states.map(s => [s.articleId, { read: !!s.read, bookmarked: !!s.bookmarked }])
        );
        articleStatesRef.current = new Map(serverMap);

        // Snapshot local articles before the overlay setState so the seed
        // loop below sees what the user had pre-sign-in, not the merged view.
        const localBefore = articles;
        setArticles(prev => prev.map(a => {
          const s = serverMap.get(String(a.id));
          return s ? { ...a, read: s.read, bookmarked: s.bookmarked } : a;
        }));

        // Seed: push any local-only flags to the server so other devices
        // see them. If the server already has an entry for that article id,
        // the server's copy wins (already applied above) — we don't
        // overwrite it with the possibly-older local flag.
        for (const a of localBefore) {
          const id = String(a.id);
          if (serverMap.has(id)) continue;
          if (!a.read && !a.bookmarked) continue;
          const next = { read: !!a.read, bookmarked: !!a.bookmarked };
          articleStatesRef.current.set(id, next);
          syncFetch('/sync/article-states', {
            method: 'PATCH',
            body: { articleId: id, ...next },
            groupKey: `PATCH:/sync/article-states:${id}`,
          }).catch(e => console.error('seed article state:', e));
        }
      } catch (e) {
        console.error('pull article states failed:', e);
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, account]);

  // Prefs sync: visual taste (theme/accent/density/serif/showAccent/viewMode)
  // plus briefing behavior (runAtLaunch/scheduleEnabled/scheduleTime). UI
  // state that's per-device by nature — `sidebarCollapsed` and the current
  // `view` — stays local and is not round-tripped here.
  const syncedPrefs = useMemo(() => ({
    theme, viewMode, runAtLaunch, scheduleEnabled, scheduleTime,
    density: tweaks.density,
    accent: tweaks.accent,
    useSerif: tweaks.useSerif,
    showAccent: tweaks.showAccent,
  }), [theme, viewMode, runAtLaunch, scheduleEnabled, scheduleTime, tweaks]);

  const setSyncedPrefs = (updater) => {
    const next = typeof updater === 'function' ? updater(syncedPrefs) : updater;
    if ('theme' in next) setTheme(next.theme);
    if ('viewMode' in next) setViewMode(next.viewMode);
    if ('runAtLaunch' in next) setRunAtLaunch(next.runAtLaunch);
    if ('scheduleEnabled' in next) setScheduleEnabled(next.scheduleEnabled);
    if ('scheduleTime' in next) setScheduleTime(next.scheduleTime);
    setTweaks(prev => ({
      density: next.density ?? prev.density,
      accent: next.accent ?? prev.accent,
      useSerif: next.useSerif ?? prev.useSerif,
      showAccent: next.showAccent ?? prev.showAccent,
    }));
  };

  const prefsSync = useSyncedResource({
    value: syncedPrefs, setValue: setSyncedPrefs,
    account, ready, label: 'prefs',
    syncFetch, endpoint: '/sync/prefs',
    buildPushBody: (p) => ({ prefs: p }),
    // Merge remote over defaults so a missing key on an old server row falls
    // through to the default instead of becoming undefined.
    fromRemote: (snap) => {
      const r = snap.prefs ?? {};
      return {
        theme: r.theme ?? DEFAULT_PREFS.theme,
        viewMode: r.viewMode ?? DEFAULT_PREFS.viewMode,
        density: r.density ?? DEFAULT_PREFS.density,
        accent: r.accent ?? DEFAULT_PREFS.accent,
        useSerif: r.useSerif ?? DEFAULT_PREFS.useSerif,
        showAccent: r.showAccent ?? DEFAULT_PREFS.showAccent,
        runAtLaunch: r.runAtLaunch ?? DEFAULT_PREFS.runAtLaunch,
        scheduleEnabled: r.scheduleEnabled ?? DEFAULT_PREFS.scheduleEnabled,
        scheduleTime: r.scheduleTime ?? DEFAULT_PREFS.scheduleTime,
      };
    },
    stableKey: (x) => JSON.stringify(x),
    // Never empty — defaults are always present; we always want a row on the
    // server once the user signs in.
    isEmpty: () => false,
  });

  const archivesSync = useSyncedResource({
    value: archives, setValue: setArchives,
    account, ready, label: 'archives',
    syncFetch, endpoint: '/sync/archives',
    buildPushBody: (xs) => ({ archives: xs }),
    // Archives are append-only + immutable, so pulling "unions" remote with
    // local (dedup by id, newest first, 90-cap) instead of overwriting local.
    // Prevents A→B sign-in from losing A's local archives.
    fromRemote: (snap, prev) => {
      const remote = Array.isArray(snap.archives) ? snap.archives : [];
      const byId = new Map();
      for (const a of (prev ?? [])) byId.set(a.id, a);
      for (const a of remote) byId.set(a.id, a);
      return Array.from(byId.values())
        .sort((a, b) => new Date(b.runAt).getTime() - new Date(a.runAt).getTime())
        .slice(0, ARCHIVE_CAP);
    },
    stableKey: (xs) => JSON.stringify(xs),
    isEmpty: (xs) => xs.length === 0,
  });

  // Fully hydrated = local disk has loaded AND (if signed in) every synced
  // resource has resolved its pull. Used to gate first-run UI so the tutorial
  // doesn't flash in the window between `ready=true` and the server data
  // arriving. `article_states` doesn't affect the tutorial condition, so we
  // don't wait on it here.
  const hydrated = ready && (
    !account || (
      profileSync.pulled
      && categoriesSync.pulled
      && sourcesSync.pulled
      && archivesSync.pulled
      && prefsSync.pulled
    )
  );

  // Keep a live ref to onBriefing so scheduled timers always use current state
  // (profile/lens, sources, categories) rather than closing over a stale version.
  const onBriefingRef = useRef(null);
  useEffect(() => { onBriefingRef.current = onBriefing; });

  // Run-at-launch: once per app lifetime, after hydration settles, if enabled
  // and there's at least one usable source.
  const [launchFired, setLaunchFired] = useState(false);
  useEffect(() => {
    if (!ready || launchFired) return;
    setLaunchFired(true);
    if (runAtLaunch && sources.some(s => s.enabled && (s.feedUrl ?? '').length > 0)) {
      onBriefingRef.current?.();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready]);

  // Daily schedule (frontend-only — fires only while the app is running).
  // If we missed today's slot while the app was closed, fire on next startup.
  // Re-runs on every briefing because `archives` is in the dep list; that's
  // how the effect "reschedules" for tomorrow.
  useEffect(() => {
    if (!ready || !scheduleEnabled) return;
    const parts = /^(\d{1,2}):(\d{2})$/.exec(scheduleTime);
    if (!parts) return;
    const h = Number(parts[1]);
    const m = Number(parts[2]);
    if (h > 23 || m > 59) return;

    const hasUsableSources = () =>
      sources.some(s => s.enabled && (s.feedUrl ?? '').length > 0);

    const fire = () => {
      if (hasUsableSources()) onBriefingRef.current?.();
    };

    const now = new Date();
    const todaySlot = new Date(now);
    todaySlot.setHours(h, m, 0, 0);

    const lastRun = archives[0]?.runAt ? new Date(archives[0].runAt) : null;
    const ranTodayAlready = lastRun && lastRun >= todaySlot;

    if (todaySlot <= now && !ranTodayAlready) {
      // We missed today's slot (or just reached it). Fire now.
      fire();
      // Don't also setTimeout — the new archive entry will re-trigger this
      // effect and schedule tomorrow.
      return;
    }

    const next = todaySlot > now
      ? todaySlot
      : new Date(todaySlot.getTime() + 24 * 60 * 60 * 1000);
    const delay = next.getTime() - now.getTime();
    const timer = setTimeout(fire, delay);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, scheduleEnabled, scheduleTime, archives]);

  useEffect(() => {
    if (!ready) return;
    setItem('sources', sources);
  }, [ready, sources]);

  useEffect(() => {
    if (!ready) return;
    setItem('articles', articles);
  }, [ready, articles]);

  useEffect(() => {
    if (!ready) return;
    setItem('archives', archives);
  }, [ready, archives]);

  useEffect(() => {
    if (!ready) return;
    setItem('categories', categories);
  }, [ready, categories]);

  const applyArticleStates = (xs) => xs.map(a => {
    const s = articleStatesRef.current.get(String(a.id));
    return s ? { ...a, read: !!s.read, bookmarked: !!s.bookmarked } : a;
  });

  const archiveBriefing = (articles, errors) => {
    if (!articles || articles.length === 0) return;
    const snapshot = {
      id: Date.now(),
      runAt: new Date().toISOString(),
      articles,
      errors: errors ?? [],
    };
    setArchives(prev => [snapshot, ...prev].slice(0, ARCHIVE_CAP));
  };

  const onBriefing = async () => {
    setScanning(true);
    setArticles([]);
    setBriefingErrors([]);

    if (!isTauri()) {
      // Browser-only dev mode (npm run dev without Tauri): use mock data.
      await new Promise(r => setTimeout(r, 1600));
      const fresh = RADAR_DATA.articles.map(a => ({ ...a, read: false }));
      setArticles(applyArticleStates(fresh));
      archiveBriefing(fresh, []);
      setScanning(false);
      return;
    }

    try {
      const enabled = sources.filter(s => s.enabled && (s.feedUrl ?? '').length > 0);
      const sourcesPayload = enabled
        .map(s => ({ id: s.id, name: s.name, category: s.category, feedUrl: s.feedUrl }));
      const categoriesPayload = categories.map(c => ({
        id: c.id, label: c.label, description: c.description, accent: c.accent,
      }));
      const resp = await invoke('ingest_briefing', {
        sources: sourcesPayload,
        categories: categoriesPayload,
        lens: profile.lens?.trim() ? profile.lens.trim() : null,
      });
      setArticles(applyArticleStates(resp.articles));
      setBriefingErrors(resp.errors ?? []);
      archiveBriefing(resp.articles, resp.errors ?? []);

      // Stamp per-source health: warn for anything in the errors array (excluding
      // the synthetic source_id=0 used for Claude-summarization failures),
      // ok + timestamp for every other enabled+URL'd source we attempted.
      const now = Date.now();
      const failedIds = new Set(
        (resp.errors ?? [])
          .map(e => e.sourceId ?? e.source_id)
          .filter(id => id && id !== 0)
      );
      const attempted = new Set(enabled.map(s => s.id));
      setSources(prev => prev.map(s => {
        if (!attempted.has(s.id)) return s;
        return failedIds.has(s.id)
          ? { ...s, health: 'warn' }
          : { ...s, health: 'ok', lastFetchAt: now };
      }));
    } catch (e) {
      console.error('ingest_briefing failed:', e);
      setBriefingErrors([{ sourceId: 0, sourceName: 'radar', message: String(e) }]);
    } finally {
      setScanning(false);
    }
  };

  const toggleSource = (id) => {
    setSources(prev => prev.map(s => s.id === id ? { ...s, enabled: !s.enabled } : s));
  };
  const addSource = ({ name, category, feedUrl }) => {
    const nextId = sources.reduce((m, s) => Math.max(m, s.id), 0) + 1;
    setSources(prev => [
      ...prev,
      {
        id: nextId,
        name,
        category,
        feedUrl: feedUrl || null,
        enabled: true,
        isDefault: false,
        lastFetchAt: null,
        health: 'ok',
      },
    ]);
  };
  const updateSource = (id, changes) => {
    setSources(prev => prev.map(s => s.id === id ? { ...s, ...changes } : s));
  };
  const deleteSource = (id) => {
    setSources(prev => prev.filter(s => s.id !== id));
  };
  const patchArticleState = (id, next) => {
    const key = String(id);
    articleStatesRef.current.set(key, next);
    if (account) {
      syncFetch('/sync/article-states', {
        method: 'PATCH',
        body: { articleId: key, read: !!next.read, bookmarked: !!next.bookmarked },
        groupKey: `PATCH:/sync/article-states:${key}`,
      }).catch(e => console.error('patch article state failed:', e));
    }
  };
  const toggleRead = (id) => {
    const a = articles.find(x => x.id === id);
    if (!a) return;
    const next = { read: !a.read, bookmarked: !!a.bookmarked };
    setArticles(prev => prev.map(x => x.id === id ? { ...x, read: next.read } : x));
    patchArticleState(id, next);
  };
  const toggleBookmark = (id) => {
    const a = articles.find(x => x.id === id);
    if (!a) return;
    const next = { read: !!a.read, bookmarked: !a.bookmarked };
    setArticles(prev => prev.map(x => x.id === id ? { ...x, bookmarked: next.bookmarked } : x));
    patchArticleState(id, next);
  };

  const slugify = (label) => {
    const base = label.toLowerCase().trim().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
    if (!base) return 'category';
    const existing = new Set(categories.map(c => c.id));
    if (!existing.has(base)) return base;
    let n = 2;
    while (existing.has(`${base}-${n}`)) n++;
    return `${base}-${n}`;
  };
  const addCategory = ({ label, description, accent }) => {
    const id = slugify(label);
    setCategories(prev => [...prev, { id, label: label.trim(), description: description.trim(), accent }]);
  };
  const updateCategory = (id, changes) => {
    setCategories(prev => prev.map(c => c.id === id ? { ...c, ...changes } : c));
  };
  const deleteCategory = (id) => {
    // Caller should have already confirmed no sources point to this id.
    setCategories(prev => prev.filter(c => c.id !== id));
  };

  return (
    <div className="app" data-sidebar={sidebarCollapsed ? 'collapsed' : 'expanded'}>
      <Sidebar
        view={view}
        setView={setView}
        sources={sources}
        toggleSource={toggleSource}
        categories={categories}
        articles={articles}
        profile={profile}
        account={account}
        activeCategory={activeCategory}
        setActiveCategory={setActiveCategory}
        onAddSource={() => { setView('sources'); setSourcesIntent('add'); }}
      />
      <div className="main">
        <Header
          view={view}
          sidebarCollapsed={sidebarCollapsed}
          toggleSidebar={() => setSidebarCollapsed(c => !c)}
          viewMode={viewMode}
          setViewMode={setViewMode}
          theme={theme}
          setTheme={setTheme}
          onBriefing={onBriefing}
          scanning={scanning}
          onOpenSearch={() => setSearchOpen(true)}
        />
        <div className="content">
          {view === 'briefing' && (
            <BriefingView
              articles={articles}
              categories={categories}
              sources={sources}
              profile={profile}
              archives={archives}
              viewMode={viewMode}
              activeCategory={activeCategory}
              setActiveCategory={setActiveCategory}
              scanning={scanning}
              showAccent={tweaks.showAccent}
              useSerif={tweaks.useSerif}
              errors={briefingErrors}
              profileName={profile.name?.trim() || account?.name?.trim() || (account?.email ? account.email.split('@')[0] : '')}
              lastRunAt={archives[0]?.runAt ? Date.parse(archives[0].runAt) : null}
              onToggleRead={toggleRead}
              onToggleBookmark={toggleBookmark}
              setView={setView}
              onAddSource={() => { setView('sources'); setSourcesIntent('add'); }}
              ready={hydrated}
            />
          )}
          {view === 'saved' && (
            <SavedView
              articles={articles}
              onToggleRead={toggleRead}
              onToggleBookmark={toggleBookmark}
              showAccent={tweaks.showAccent}
              useSerif={tweaks.useSerif}
            />
          )}
          {view === 'archive' && <ArchiveView archives={archives} useSerif={tweaks.useSerif} />}
          {view === 'sources' && (
            <SourcesView
              sources={sources}
              categories={categories}
              toggleSource={toggleSource}
              addSource={addSource}
              updateSource={updateSource}
              deleteSource={deleteSource}
              intent={sourcesIntent}
              clearIntent={() => setSourcesIntent(null)}
            />
          )}
          {view === 'categories' && (
            <CategoriesView
              categories={categories}
              sources={sources}
              addCategory={addCategory}
              updateCategory={updateCategory}
              deleteCategory={deleteCategory}
            />
          )}
          {view === 'settings' && (
            <SettingsView
              theme={theme} setTheme={setTheme}
              tweaks={tweaks} setTweaks={setTweaks}
              profile={profile} setProfile={setProfile}
              runAtLaunch={runAtLaunch} setRunAtLaunch={setRunAtLaunch}
              scheduleEnabled={scheduleEnabled} setScheduleEnabled={setScheduleEnabled}
              scheduleTime={scheduleTime} setScheduleTime={setScheduleTime}
              account={account}
            />
          )}
        </div>
      </div>

      <button
        className="iconbtn"
        style={{
          position: 'fixed', right: 20, bottom: 20, width: 40, height: 40,
          background: 'var(--surface)', border: '1px solid var(--border-strong)',
          boxShadow: 'var(--shadow-md)', zIndex: 90,
          display: tweaksOpen ? 'none' : 'inline-flex',
        }}
        onClick={() => setTweaksOpen(true)}
        title="Tweaks"
      >
        {/* inline to avoid extra import; mirrors Icon sliders size */}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
             strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
          <path d="M4 6h10M18 6h2M4 12h2M10 12h10M4 18h14M20 18h0"/>
          <circle cx="16" cy="6" r="2"/><circle cx="8" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>
        </svg>
      </button>

      {searchOpen && (
        <SearchPalette
          articles={articles}
          archives={archives}
          sources={sources}
          onClose={() => setSearchOpen(false)}
          onNavigate={(v) => setView(v)}
        />
      )}

      {tweaksOpen && (
        <TweaksPanel
          tweaks={tweaks} setTweaks={setTweaks}
          theme={theme} setTheme={setTheme}
          viewMode={viewMode} setViewMode={setViewMode}
          sidebarCollapsed={sidebarCollapsed} setSidebarCollapsed={setSidebarCollapsed}
          onClose={() => setTweaksOpen(false)}
        />
      )}
    </div>
  );
}
