# Radar

A personalized design-news briefing, delivered to your desktop each morning.

[![Release](https://github.com/alamanija/radar/actions/workflows/release.yml/badge.svg)](https://github.com/alamanija/radar/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/alamanija/radar?include_prereleases&sort=semver)](https://github.com/alamanija/radar/releases/latest)
[![License](https://img.shields.io/github/license/alamanija/radar)](./LICENSE.md)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24c8db?logo=tauri&logoColor=white)](https://tauri.app)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-FF5E5B?logo=kofi&logoColor=white)](https://ko-fi.com/alamanija)

Radar watches the RSS feeds you care about, groups every morning's stories into categories you define, and asks Claude to write short, editorially-neutral summaries tuned to your professional lens. You open the app, you see what actually matters, you move on.

## Highlights

- **Personalized briefings.** You define your own categories and describe each one; Claude reads the description and sorts every article accordingly.
- **BYO AI.** Your Anthropic API key stays in your system keychain and never leaves the device.
- **Sync across devices.** Sign in with Clerk and your sources, categories, archives, profile, and read state flow between every Mac/PC you use.
- **Offline-first.** Edits while offline are queued and replay automatically once the network returns. `If-Match`-based conflict detection keeps multi-device edits honest.
- **Signed auto-updates.** Every release is Ed25519-signed and verified on-device before install.
- **Keyboard-first UI.** `⌘K` / `Ctrl+K` searches your live briefing, every archive, and every source.

## Download

Grab the latest signed build from the [Releases page](https://github.com/alamanija/radar/releases/latest):

- macOS (Apple Silicon): `Radar_<version>_aarch64.dmg`
- Windows: `Radar_<version>_x64-setup.exe`

macOS and Windows builds are not yet code-signed. First launch will show a warning that isn't actually telling the truth:

- **macOS:** "Radar is damaged and can't be opened." Not damaged — just not signed by an Apple Developer. Strip the quarantine flag once and it opens forever:
  ```bash
  xattr -dr com.apple.quarantine /Applications/Radar.app
  ```
- **Windows:** SmartScreen warns about an unrecognized publisher. Click "More info" → "Run anyway."

This goes away entirely once the macOS and Windows builds are code-signed — planned for a future release.

## Development

Radar is a Cargo workspace with a Tauri desktop shell, a React frontend, and an Axum sync server.

### Prerequisites

- Node.js 22 (see [`.nvmrc`](./.nvmrc))
- Rust stable
- Docker (for local Postgres)
- A Clerk dev instance ([`clerk.com`](https://clerk.com))

### Quick start

```bash
# 1. Install JS deps
npm install

# 2. Configure environment
cp .env.example .env                       # fill VITE_CLERK_PUBLISHABLE_KEY
cp radar-server/.env.example radar-server/.env   # fill CLERK_JWKS_URL + CLERK_ISSUER

# 3. Start Postgres
docker compose up -d

# 4. Start the sync server (migrations run automatically)
cargo run -p radar-server

# 5. Launch the desktop app
npm run tauri:dev
```

Full project layout, sync architecture, and release process are documented in [`CLAUDE.md`](./CLAUDE.md).

## Tech stack

| Layer | Tech |
|---|---|
| Desktop shell | Tauri 2 |
| Frontend | React 18 · Vite 6 |
| Auth | Clerk (JWT) |
| Sync server | Axum · sqlx · Postgres |
| Summarization | Anthropic Claude (`claude-opus-4-7`) |
| Updates | `tauri-plugin-updater` + signed GitHub Releases |
| License | MIT |

## Support the project

Radar is free and open-source. If it earns a spot on your dock, tip the jar so the feed-parsing, Claude bills, and domain renewals stay funded:

<a href="https://ko-fi.com/alamanija" target="_blank">
  <img src="https://storage.ko-fi.com/cdn/kofi_button_red.png" alt="Support me on Ko-fi" height="44" />
</a>

## License

[MIT](./LICENSE.md) © Stefan Jovanovic
