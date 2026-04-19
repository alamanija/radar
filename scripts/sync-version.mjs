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

const cargoPath = resolve(REPO, 'src-tauri/Cargo.toml');
const cargo = readFileSync(cargoPath, 'utf8');

// Match the `version = "…"` line of the [package] section. Dependency
// versions are always inline (`foo = { version = "x" }`), so the only
// `version` assignment that starts its own line is the package one.
const re = /^version\s*=\s*"[^"]*"/m;
if (!re.test(cargo)) {
  console.error('sync-version: could not find [package] version line in src-tauri/Cargo.toml');
  process.exit(1);
}

const updated = cargo.replace(re, `version = "${version}"`);
if (updated === cargo) {
  console.log(`sync-version: src-tauri/Cargo.toml already at ${version}`);
  process.exit(0);
}

writeFileSync(cargoPath, updated);
console.log(`sync-version: src-tauri/Cargo.toml → ${version}`);
