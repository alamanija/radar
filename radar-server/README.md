# radar-server

Axum sync backend for the Radar desktop app. Identity is owned by Clerk —
clients obtain a session JWT via Clerk's frontend SDK, and this server
verifies the JWT against Clerk's JWKS on every authed request. There's no
users table, no sessions table, no server-side session cookies; the Clerk
`sub` claim is the user key.

## First-time setup

1. **Create a Clerk application.** In https://dashboard.clerk.com create an
   application (free tier is fine). From the dashboard, copy:
   - **Publishable key** — frontend-side, used by the Tauri client.
   - **JWKS URL** — `https://<your-app>.clerk.accounts.dev/.well-known/jwks.json`.
   - **Issuer** — `https://<your-app>.clerk.accounts.dev`.

2. **Start Postgres** (from the repo root):
   ```
   docker compose up -d
   ```
   Postgres is reachable on `localhost:5433`. User/password/db are all `radar`.

3. **Configure the server:**
   ```
   cp radar-server/.env.example radar-server/.env
   # fill in CLERK_JWKS_URL + CLERK_ISSUER
   ```

4. **Run:**
   ```
   cargo run -p radar-server
   ```
   Migrations run automatically on startup. The server listens on `0.0.0.0:8787`.

## Building the Tauri client with Clerk

The frontend reads credentials via `VITE_*` env vars. Copy `.env.example`
at the repo root to `.env` and fill in:

```
VITE_CLERK_PUBLISHABLE_KEY=pk_test_...
VITE_RADAR_SERVER_URL=http://127.0.0.1:8787   # optional, this is the default
```

Then `npm run tauri:dev`. The Tauri side no longer participates in auth —
the webview owns the Clerk session, and the frontend calls this server
directly attaching `Authorization: Bearer ${clerkSessionJwt}`.

## Endpoints

| Method | Path                   | Auth   | Description |
|--------|------------------------|--------|-------------|
| GET    | `/healthz`             | public | `ok`. |
| GET    | `/sync/profile`        | bearer | Returns `{name, role, lens, updatedAt}` or 404 if the user has no profile yet. |
| PUT    | `/sync/profile`        | bearer | Body: `{name?, role?, lens?}`. Upserts and echoes the stored row back. |
| GET    | `/sync/categories`     | bearer | Returns `{categories: Category[], updatedAt}` or 404. |
| PUT    | `/sync/categories`     | bearer | Body: `{categories: Category[]}`. Full-list replace. |
| GET    | `/sync/sources`        | bearer | Returns `{sources: Source[], updatedAt}` or 404. |
| PUT    | `/sync/sources`        | bearer | Body: `{sources: Source[]}`. Full-list replace. |
| GET    | `/sync/archives`       | bearer | Returns `{archives, updatedAt}` or 404. Opaque JSON. |
| PUT    | `/sync/archives`       | bearer | Body: `{archives}`. Full-list replace; client dedups/merges/caps (90). |
| GET    | `/sync/article-states` | bearer | Returns `{states: ArticleState[]}`. Empty array when nothing synced yet. |
| PATCH  | `/sync/article-states` | bearer | Body: `{articleId, read, bookmarked}`. Per-row upsert. |

- `Category` is `{id, label, description, accent}`.
- `Source` is `{id, name, category, feedUrl?, enabled, isDefault}` — device-local `lastFetchAt` and `health` are intentionally not on the wire.
- `ArticleState` is `{articleId, read, bookmarked, updatedAt}`.

All `/sync/*` routes require `Authorization: Bearer <clerk-session-jwt>`.
The server verifies the JWT against Clerk's JWKS (RS256, `iss` must match
`CLERK_ISSUER`, `aud` is not checked because Clerk's default session
template doesn't set one). Identity is the `sub` claim.

## Schema

One row per user per resource, keyed on `clerk_user_id text` (primary key
alone — Clerk owns identity so there's no local `users` table to FK
against). See `migrations/` for details.
