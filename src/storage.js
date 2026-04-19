// Persistent storage: Tauri plugin-store on desktop, localStorage in browser dev.
//
// Shape on disk (radar.json):
//   { version: 1, prefs, sources, articles }
// Each key is fetched/written independently so the React tree can drive its
// own debounce points.

import { isTauri } from '@tauri-apps/api/core';

const FILE = 'radar.json';
const SCHEMA_VERSION = 1;
const LS_PREFIX = 'radar:';

let storePromise = null;
async function getStore() {
  if (!storePromise) {
    const { load } = await import('@tauri-apps/plugin-store');
    storePromise = load(FILE, { autoSave: 100 });
    const store = await storePromise;
    // Stamp the schema version once, so future migrations have something to
    // key off of. Harmless on every boot.
    await store.set('version', SCHEMA_VERSION);
  }
  return storePromise;
}

export async function getItem(key) {
  if (isTauri()) {
    try {
      const store = await getStore();
      return await store.get(key);
    } catch (e) {
      console.error('storage.getItem failed:', key, e);
      return undefined;
    }
  }
  try {
    const raw = localStorage.getItem(LS_PREFIX + key);
    return raw == null ? undefined : JSON.parse(raw);
  } catch {
    return undefined;
  }
}

export async function setItem(key, value) {
  if (isTauri()) {
    try {
      const store = await getStore();
      await store.set(key, value);
    } catch (e) {
      console.error('storage.setItem failed:', key, e);
    }
    return;
  }
  try {
    localStorage.setItem(LS_PREFIX + key, JSON.stringify(value));
  } catch (e) {
    console.error('storage.setItem (ls) failed:', key, e);
  }
}

export async function loadSnapshot() {
  const [prefs, sources, articles, archives, categories, profile] = await Promise.all([
    getItem('prefs'),
    getItem('sources'),
    getItem('articles'),
    getItem('archives'),
    getItem('categories'),
    getItem('profile'),
  ]);
  return { prefs, sources, articles, archives, categories, profile };
}
