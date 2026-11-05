<p align="center">
  <img src="./radar-app/assets/icons/128x128@2x.png" alt="Radar" width="96" height="96" />
</p>

<h1 align="center">Radar</h1>

<p align="center">A personalized design-news briefing, delivered to your desktop each morning.</p>

[![Release](https://github.com/alamanija/radar/actions/workflows/release.yml/badge.svg)](https://github.com/alamanija/radar/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/alamanija/radar?include_prereleases&sort=semver)](https://github.com/alamanija/radar/releases/latest)
[![License](https://img.shields.io/github/license/alamanija/radar)](./LICENSE.md)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust%20%2B%20GPUI-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-FF5E5B?logo=kofi&logoColor=white)](https://ko-fi.com/alamanija)

Radar watches the RSS feeds you care about, groups every morning's stories into categories you define, and asks Claude to write short, editorially-neutral summaries tuned to your professional lens. You open the app, you see what actually matters, you move on.

## Highlights

- **Personalized briefings.** You define your own categories and describe each one; Claude reads the description and sorts every article accordingly.
- **BYO AI.** Your Anthropic API key stays in your system keychain and never leaves the device.
- **Sync across devices.** Sign in with Clerk and your sources, categories, archives, profile, and read state flow between every Mac/PC you use.
- **Offline-first.** Edits while offline are queued and replay automatically once the network returns. `If-Match`-based conflict detection keeps multi-device edits honest.
- **Signed auto-updates.** Every release is minisign-signed and verified on-device before install.
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

Radar is a Cargo workspace with a native GPUI Kit desktop app (`radar-app`), a UI-free core library (`radar-core`) and an Axum sync server (`radar-server`).

### Prerequisites

- Rust 1.99 or newer (`radar-app` needs it for GPUI); the repo is built and tested with `cargo +1.99.1`
- Docker (for local Postgres)
- A Clerk dev instance ([`clerk.com`](https://clerk.com)) with an OAuth application (see [`radar-server/README.md`](./radar-server/README.md))

### Quick start

```bash
# 1. Configure the sync server
cp radar-server/.env.example radar-server/.env   # fill CLERK_JWKS_URL + CLERK_ISSUER

# 2. Start Postgres
docker compose up -d

# 3. Start the sync server (migrations run automatically)
cargo +1.99.1 run -p radar-server

# 4. Launch the desktop app (see .env.example for the RADAR_* variables)
export RADAR_CLERK_FRONTEND_API=https://<your-instance>.clerk.accounts.dev
export RADAR_CLERK_CLIENT_ID=<oauth client id>
cargo +1.99.1 run -p radar-app
```

Signing in is optional; without the Clerk variables the app still works locally. Run the tests with `cargo +1.99.1 test --workspace`.

Full project layout, sync architecture, and release process are documented in [`CLAUDE.md`](./CLAUDE.md).

## Tech stack

| Layer | Tech |
|---|---|
| Desktop app | Rust · GPUI Kit |
| Auth | Clerk (OAuth, PKCE) |
| Sync server | Axum · sqlx · Postgres |
| Summarization | Anthropic Claude (`claude-opus-4-7`) |
| Updates | `cargo-packager` + `cargo-packager-updater`, signed GitHub Releases |
| License | MIT |

## Support the project

Radar is free and open-source. If it earns a spot on your dock, tip the jar so the feed-parsing, Claude bills, and domain renewals stay funded:

<a href="https://ko-fi.com/alamanija" target="_blank">
  <img src="https://storage.ko-fi.com/cdn/kofi_button_red.png" alt="Support me on Ko-fi" height="44" />
</a>

## Security

Found a security issue? Please **don't open a public GitHub issue.** See [SECURITY.md](./SECURITY.md) for the private-disclosure process (GitHub Security Advisories or `security@getradar.xyz`).

## License

[MIT](./LICENSE.md) © Stefan Jovanovic
