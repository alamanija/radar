#!/usr/bin/env node
// Propagate `package.json > version` to `src-tauri/Cargo.toml`.
//
// `package.json` is the single source of truth for Radar's release version
// (Tauri reads it directly via `tauri.conf.json > "version": "../package.json"`).
// Cargo, however, demands a literal SemVer string in `Cargo.toml` — it has
// no equivalent of Tauri's cross-file indirection. This script mirrors the
// package.json value into Cargo.toml so the two never drift.
//
// Run automatically by the `version` npm lifecycle hook (see package.json's
// `scripts` section) whenever you invoke `npm version <patch|minor|major>`.
// Can also be run on demand: `node scripts/sync-version.mjs`.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..');

const pkg = JSON.parse(readFileSync(resolve(REPO, 'package.json'), 'utf8'));
const version = pkg.version;
if (!/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(version)) {
  console.error(`sync-version: package.json version ${JSON.stringify(version)} doesn't look like SemVer`);
  process.exit(1);
}

// --- 1. src-tauri/Cargo.toml --------------------------------------------
const cargoPath = resolve(REPO, 'src-tauri/Cargo.toml');
const cargo = readFileSync(cargoPath, 'utf8');

// Match the `version = "…"` line of the [package] section. Dependency
// versions are always inline (`foo = { version = "x" }`), so the only
// `version` assignment that starts its own line is the package one.
const cargoRe = /^version\s*=\s*"[^"]*"/m;
if (!cargoRe.test(cargo)) {
  console.error('sync-version: could not find [package] version line in src-tauri/Cargo.toml');
  process.exit(1);
}

const cargoUpdated = cargo.replace(cargoRe, `version = "${version}"`);
if (cargoUpdated === cargo) {
  console.log(`sync-version: src-tauri/Cargo.toml already at ${version}`);
} else {
  writeFileSync(cargoPath, cargoUpdated);
  console.log(`sync-version: src-tauri/Cargo.toml → ${version}`);
}

// --- 2. Cargo.lock (workspace root) -------------------------------------
// Cargo.lock records the resolved version of every workspace member.
// When a member's Cargo.toml version changes, its lock entry must change
// too; otherwise `cargo build --locked` (used in CI) fails. Rather than
// shelling out to `cargo` (which would need Rust in the npm hook's env),
// rewrite the `radar` entry's version line directly — only that block
// matters since radar-server has its own name and won't collide.
const lockPath = resolve(REPO, 'Cargo.lock');
const lock = readFileSync(lockPath, 'utf8');

const lockRe = /(\[\[package\]\]\s*\nname\s*=\s*"radar"\s*\nversion\s*=\s*")[^"]*(")/;
if (!lockRe.test(lock)) {
  console.error('sync-version: could not find [[package]] name="radar" entry in Cargo.lock');
  process.exit(1);
}

const lockUpdated = lock.replace(lockRe, `$1${version}$2`);
if (lockUpdated === lock) {
  console.log(`sync-version: Cargo.lock already at ${version}`);
} else {
  writeFileSync(lockPath, lockUpdated);
  console.log(`sync-version: Cargo.lock → ${version}`);
}
