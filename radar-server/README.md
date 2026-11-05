# radar-server

Axum sync backend for the Radar desktop app. Identity is owned by Clerk —
the native client signs in with Clerk OAuth (see below), and this server
verifies the resulting token against Clerk's JWKS on every authed request. There's no
users table, no sessions table, no server-side session cookies; the Clerk
`sub` claim is the user key.

## First-time setup

1. **Create a Clerk application.** In https://dashboard.clerk.com create an
   application (free tier is fine). From the dashboard, copy:
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

## Running the desktop client against this server

`radar-app` reads its config from the environment (runtime first, then the
value baked in at compile time). Copy the names from the repo-root
`.env.example` and export them in your shell:

```
RADAR_SERVER_URL=http://127.0.0.1:8787     # optional, this is the default
RADAR_CLERK_FRONTEND_API=https://<your-app>.clerk.accounts.dev
RADAR_CLERK_CLIENT_ID=<oauth client id>    # from the OAuth application below
```

Then `cargo +1.99.1 run -p radar-app`. The client attaches
`Authorization: Bearer <clerk-oauth-access-token>` to every `/sync/*` call.

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

All `/sync/*` routes require `Authorization: Bearer <clerk-oauth-access-token-or-session-jwt>`.
The server verifies the JWT against Clerk's JWKS (RS256, `iss` must match
`CLERK_ISSUER`, `aud` is not checked because Clerk's default session
template doesn't set one). Identity is the `sub` claim.

## Schema

One row per user per resource, keyed on `clerk_user_id text` (primary key
alone — Clerk owns identity so there's no local `users` table to FK
against). See `migrations/` for details.

## Clerk OAuth access tokens (native desktop app)

The native Radar desktop app signs in with Clerk OAuth (PKCE, loopback
redirect) and sends the resulting **Clerk OAuth access token** as the bearer.
Clerk session JWTs (used by older clients) are still accepted. Both are RS256,
signed by the same instance key (same JWKS), validated for `exp` and `iss`;
`aud` is not checked. Identity is `sub`.

**Clerk dashboard setup:** create an OAuth application with
- client type **Public**, **Require PKCE** enabled
- redirect URI `http://127.0.0.1:17823/callback`
- scopes `openid profile email offline_access`
- access token format **JWT**

**Optional env vars:**
- `CLERK_OAUTH_ISSUER` — additional accepted `iss`, if OAuth tokens use a
  different issuer than session tokens.
- `CLERK_OAUTH_CLIENT_ID` — when set, a token that looks like an OAuth token
  (has a `client_id` claim or a `scope`/`scp` claim) must have `client_id`
  (else `azp`) equal to it. Session JWTs carry `azp` = the frontend origin and
  no scope, so they pass through unchanged.

To inspect the live claims of a real token (decoded claims only, never the
token itself): `cargo run -p radar-core --example sign_in` with
`RADAR_CLERK_FRONTEND_API` and `RADAR_CLERK_CLIENT_ID` set.
