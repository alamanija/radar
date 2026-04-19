// Thin bridge to `@tauri-apps/plugin-log`. Under Tauri, messages route
// through the Rust `log` crate and hit every configured target (rolling
// file in OS logs dir, stdout in dev, Webview channel → DevTools). Under
// plain Vite the plugin isn't available, so we fall back to `console.*`.
//
// Callers: await-or-ignore at the call site. Logging should never block a
// code path, so every call is fire-and-forget — we catch the rejection
// and swallow it rather than bubbling a logging failure up.

import { isTauri } from '@tauri-apps/api/core';

function consoleFallback(level, msg) {
  const fn = console[level] ?? console.log;
  fn(msg);
}

async function emit(level, msg) {
  if (!isTauri()) {
    consoleFallback(level, msg);
    return;
  }
  try {
    const plugin = await import('@tauri-apps/plugin-log');
    await plugin[level](msg);
  } catch {
    consoleFallback(level, msg);
  }
}

export const logger = {
  info: (msg) => emit('info', msg),
  warn: (msg) => emit('warn', msg),
  error: (msg) => emit('error', msg),
  debug: (msg) => emit('debug', msg),
  trace: (msg) => emit('trace', msg),
};

/// One-shot setup for the webview side: subscribes to the Rust `Webview`
/// log target so every `log::info!` from the backend surfaces in DevTools.
/// No-op outside Tauri. Safe to call multiple times.
let consoleAttached = false;
export async function attachLogConsole() {
  if (!isTauri() || consoleAttached) return;
  try {
    const { attachConsole } = await import('@tauri-apps/plugin-log');
    await attachConsole();
    consoleAttached = true;
  } catch (e) {
    console.warn('[log] attachConsole failed:', e);
  }
}
