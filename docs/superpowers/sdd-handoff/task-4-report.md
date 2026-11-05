# Task 4 report: Clerk OAuth (PKCE + loopback), session and token storage

**Status:** DONE_WITH_CONCERNS. The only concern is size: auth.rs is 743 physical lines before tests. About 590 of those are code, not comments or blank lines.
**Commit:** `8c32043 feat(core): Clerk OAuth PKCE loopback sign-in` on branch `worktree-gpui-migration`.

## What was implemented

### `radar-core/src/keychain.rs`
- `trait KeyStore: Send + Sync { get / set / delete }` with the exact signatures from the ruling.
- `OsKeyStore`:
  - Backed by keyring, service `com.radar.dev`.
  - `NoEntry` gives `Ok(None)` from `get` and `Ok(())` from `delete`.
- `pub struct MemKeyStore`:
  - A `Mutex<HashMap>`, with `new()` and `Default`.
  - Not cfg(test)-gated.
- Constants: `ANTHROPIC_USER = "anthropic"` and `CLERK_OAUTH_USER = "clerk_oauth"`.
- The Anthropic-key fns now take `&dyn KeyStore`:
  - `set_anthropic_api_key(store, key)`, `clear_anthropic_api_key(store)` and `anthropic_api_key_status(store)`.
  - A new `read_anthropic_api_key(store)`.
- `read_api_key()` is still a convenience over `OsKeyStore`, so `ingest::run_briefing` is unchanged.
- `ApiKeyStatus` now also derives `Debug` and `PartialEq`, for tests.

### `radar-core/src/auth.rs`
**Config and authorize URL**
- `AuthConfig { frontend_api, client_id, redirect_port }`.
- `AuthConfig::from_env()`:
  - For each var it uses the runtime env var, else the `option_env!` value. Empty or whitespace values count as missing.
  - It trims trailing `/` from frontend_api. The port defaults to `DEFAULT_REDIRECT_PORT = 17823`.
  - The pure helpers `from_values` and `env_value` are unit-tested, so no test mutates the process env.
- `AuthConfig::authorize_url(&pkce, port)` builds `{frontend_api}/oauth/authorize?...`:
  - Params: `response_type=code`, `client_id`, `redirect_uri=http://127.0.0.1:{port}/callback`, `scope=openid profile email offline_access`, `state`, `code_challenge` and `code_challenge_method=S256`.
  - Encoded with `url::form_urlencoded`.

**Tokens and PKCE**
- `Tokens` (Serialize/Deserialize/Clone/PartialEq). Its `Debug` impl prints `<redacted>` for every token field.
- `Pkce::new()`:
  - The verifier is 48 random bytes, giving 64 base64url chars. The state is 24 random bytes.
  - The challenge is `BASE64URL_NOPAD(SHA256(verifier))`.
  - Randomness comes from rand 0.9 `rand::rng()`, a CSPRNG.
  - Its `Debug` impl hides the verifier and state.

**Errors**
- `AuthError` has exactly `PortInUse | Timeout | Denied(String) | StateMismatch | Http(String) | NotConfigured`, with a user-facing `Display`.
- `Http` carries a status plus a short snippet (at most 200 chars), with control characters stripped and submitted secret values replaced by `<redacted>`.
- When the token endpoint returns an OAuth error JSON, only `error` and `error_description` are kept.
- A malformed success body reports only serde's error category, line and column. serde's own message can quote values, so it is not used.

**JWT claims**
- `decode_jwt_unverified(jwt) -> Option<(header, payload)>`. It is pub, documented as display-only, and used by the example.
- `account_from_id_token(jwt) -> Option<Account>`:
  - Maps `sub` (required), `email`, `name` (falling back to given_name + family_name) and `picture`.
  - `pub use crate::types::Account;`, so the type is not redefined.
- The Account comes from the id_token, falling back to the access token's claims if that is a JWT.

**`sign_in(cfg, open, timeout)` (async)**
- Binds `TcpListener` on `127.0.0.1:{port}`.
  - `AddrInUse` maps to `PortInUse`. So does `PermissionDenied`, because Windows reports ports in excluded ranges as WSAEACCES.
  - The browser is never opened if the bind fails.
- With port 0 the actual bound port goes into `redirect_uri`.
- The callback wait is wrapped in `tokio::time::timeout`. Elapsed gives `Timeout`, and the listener is dropped.
- The loopback server needs no framework:
  - The accept loop and per-connection handlers run concurrently through `FuturesUnordered` inside `select!`, so an idle browser preconnect can't stall the flow. Each connection gets a 10s read budget.
  - It reads the full request head, at most 16 KiB. Closing with unread bytes would RST the socket and hide the page.
  - Non-GET requests get 405 and other paths (such as `/favicon.ico`) get 404, and the server keeps waiting.
  - `/callback` is parsed with `url`. `error` gives `Denied(code)`, with the code sanitized to at most 64 chars.
  - A state that doesn't exactly match gives `StateMismatch`. A missing code gives `Http("callback carried no authorization code")`.
  - The server answers 200 `text/html` with "You can close this tab and return to Radar.", or 400 with a static error page. Query content is never echoed back.
- The code is exchanged by a form POST to `{frontend_api}/oauth/token`:
  - Fields: `grant_type=authorization_code`, `code`, `redirect_uri`, `client_id` and `code_verifier`.
  - No `client_secret` is sent.
  - The reqwest client is shared, with a 30s timeout.

**`refresh(cfg, refresh_token)`**
- Sends `grant_type=refresh_token`, `refresh_token` and `client_id`.
- If the response has no new refresh_token, it keeps the old one.

**`Session` (Clone)**
- Fields: `cfg`, `Arc<dyn KeyStore>`, `Arc<std::sync::Mutex<Option<Tokens>>>` and an `Arc<tokio::sync::Mutex<()>>` refresh lock.
- `load(cfg, store)` is the only constructor (R2). A missing, unreadable or corrupt keychain entry means signed out, and a warning is logged.
- `access_token()`:
  - Uses the fast path when more than 60s remain.
  - Otherwise it takes the refresh lock and re-checks, so concurrent callers make one HTTP call (tested). Then it refreshes and persists.
  - `invalid_grant` clears the store and returns `Ok(None)`. Other errors return `Err` and keep the tokens.
  - An expired token with no refresh_token gets signed out and returns `Ok(None)`. One that is near expiry but still valid is returned as is.
  - It keeps the old id_token if the refresh doesn't return one.
  - `replace_if_current` makes sure a sign-out or new sign-in that happens during a refresh is not overwritten.
- `sign_in(&self, open, timeout) -> Result<Account, AuthError>` runs the flow and persists the tokens. I added this so the app crate doesn't have to reach into internals.
- `sign_out() -> Result<(), String>` clears memory and the keychain.
- `account()` and `is_signed_in()`.
- Keychain write failures are logged and don't fail the call. The in-memory session stays valid for that run.

### `radar-core/examples/sign_in.rs`
- Runs `AuthConfig::from_env()`, then `sign_in` on `radar_core::runtime()` with a 300s timeout.
- Opens the URL through the OS:
  - Windows: `cmd /C start "" "<url>"` via `raw_arg`, with the URL quoted so `&` isn't treated as a command separator.
  - macOS: `open`. Linux: `xdg-open`.
- If the launch fails it prints the URL. The URL holds no secrets: the state and the public challenge are not credentials.
- Prints the Account, `expires_at`, whether a refresh token is present, and the decoded header and claims of the access token and id_token. A token that isn't a JWT gets "not a JWT (opaque, N chars)". Raw tokens are never printed.

### Cargo
- Deps added: `sha2 0.10`, `base64 0.22`, `rand 0.9` and `url 2`.
- tokio gained the `net`, `time`, `sync` and `io-util` features.
- Dev-dep added: `httpmock 0.9.0`.
- reqwest 0.12 needed no `form` feature.

## Tests
`cargo test -p radar-core` passes 69 of 69. 22 of those are auth tests and 2 are keychain tests; the rest were already there. `cargo clippy -p radar-core --all-targets` gives 0 warnings. `cargo fmt --check` is clean. I ran the auth tests 5 times in a row and they were stable, at about 0.2s each run.

No test touches the OS keychain (they use `MemKeyStore`) or opens a browser. They play the browser with a no-proxy reqwest client against the loopback.

**Brief tests, all present**
- `pkce_rfc7636_vector`
- `authorize_url_has_params`
- `callback_exchanges_code`, which also checks:
  - Through an httpmock `is_true` matcher: the content-type, the `redirect_uri` equality, and that the posted `code_verifier` hashes to the URL's `code_challenge`.
  - That a favicon request gets 404 without ending the flow.
- `callback_state_mismatch_errors`, which also checks that the token endpoint gets 0 calls and the forged state is not echoed.
- `callback_error_param_is_denied`
- `callback_times_out`, which also checks the port can be bound again afterwards.
- `port_in_use_errors`, which also checks the browser was not opened.
- `session_refreshes_when_near_expiry`
- `account_from_id_token`

**Extra tests**
- `pkce_new_shape`, `config_from_values`, `env_value_prefers_runtime_then_compiled`, `secrets_not_in_debug`
- `refresh_keeps_refresh_token_when_absent`, `http_error_is_bounded_and_secret_free`
- `session_concurrent_callers_refresh_once`, `session_fresh_token_skips_refresh`, `session_invalid_grant_signs_out`, `session_other_refresh_error_keeps_tokens`
- `session_signed_out_and_sign_out` (which includes a corrupt stored entry), `session_expired_without_refresh_token_is_signed_out`, `session_sign_in_persists_tokens`
- keychain: `mem_store_roundtrip_and_missing`, `anthropic_key_fns_use_store`

## TDD evidence
**RED.** I wrote the full test module against an API skeleton whose bodies were `todo!()`:
```
$ cargo test -p radar-core auth
test auth::tests::pkce_rfc7636_vector ... FAILED
test auth::tests::callback_exchanges_code ... FAILED
test auth::tests::session_refreshes_when_near_expiry ... FAILED
... (all except one)
thread 'auth::tests::pkce_rfc7636_vector' panicked at radar-core\src\auth.rs:73:5:  (not yet implemented)
test result: FAILED. 1 passed; 21 failed; 0 ignored; 0 measured; 47 filtered out
```
The one that passed was `secrets_not_in_debug`, because the skeleton's `Debug` impl already printed nothing.

**GREEN** after the implementation:
```
$ cargo test -p radar-core auth
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 47 filtered out; finished in 0.22s
$ cargo test -p radar-core
test result: ok. 69 passed; 0 failed; ...
$ cargo clippy -p radar-core --all-targets
Finished `dev` profile ...   (0 warnings)
```
The keychain change is a refactor of existing behaviour. Its two tests were written alongside it and were not run red first.

## Files changed
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\auth.rs` (new)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\keychain.rs`
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\src\lib.rs` (`pub mod auth;`)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\examples\sign_in.rs` (new)
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\radar-core\Cargo.toml`
- `D:\Work\Projects\radar\.claude\worktrees\gpui-migration\Cargo.lock`

I did not commit `.claude/settings.local.json`. It was already modified before this task and is not mine.

## Self-review findings
- **Secrets.**
  - No log line contains a token, code or verifier. The logs carry the port, "sign-in complete", refresh outcomes, and keychain errors (keyring messages only).
  - Error messages carry status, the OAuth error and description, or a scrubbed snippet.
  - The reqwest error Display includes only the token endpoint URL, which holds no secrets.
- **Loopback.**
  - It binds to `Ipv4Addr::LOCALHOST` only.
  - The listener closes when the callback returns or the timeout drops the future.
  - Handlers spawn no tasks, so nothing outlives `sign_in`.
- **Concurrency.**
  - The std mutex is never held across an `.await`.
  - A refresh holds the async lock, so `sign_out()` and `Session::sign_in()` don't wait on the HTTP call. `replace_if_current` makes sure their result isn't overwritten.
- The brief's `Session::sign_out()` signature had no return type. I return `Result<(), String>`, the KeyStore error, because the AuthError variants are fixed and none fits a keychain failure.
- A missing `code` in an otherwise valid callback maps to `Http(...)`, because the variant set is fixed. `Denied("missing_code")` would also work. Easy to change.

## Concerns
1. **Size.** auth.rs is 743 lines before the test module, about 590 of them code. I did not split it, per the instructions. If a split is wanted, the natural seam is to move the loopback server (`wait_for_callback`, `handle_connection`, `read_request_head`, `respond`, `page`, about 130 lines) into `auth/loopback.rs`.
2. **Refresh failure with a still-valid token.** Per the decision, a refresh error other than invalid_grant returns `Err` even when the current access token still has up to 60s left. The other option is to return the old token in that window. Flagging it in case the sync task would rather degrade gracefully.
3. **Untested against live Clerk.** Tested only against httpmock. Things I can't verify from here:
   - Whether Clerk's OAuth access token is a JWT or opaque. The example reports either case.
   - Whether the id_token carries `email`, `name` and `picture`.
   - That `http://127.0.0.1:17823/callback` must be registered exactly as the OAuth app's redirect URI.

   The later task that runs `cargo run -p radar-core --example sign_in` will confirm these.
4. **Windows `PermissionDenied`.** It maps to `PortInUse`, as described in the sign_in section above. That is a small interpretation beyond the brief.
