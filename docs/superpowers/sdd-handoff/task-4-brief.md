### Task 4: Clerk OAuth (PKCE + loopback), session and token storage

**Files:**
- Create: `radar-core/src/auth.rs`
- Modify: `radar-core/src/keychain.rs` (add the token fns)

**Interfaces:**
- Produces:
  - `auth::AuthConfig { frontend_api: String, client_id: String, redirect_port: u16 /*17823*/ }` with `AuthConfig::from_env() -> Option<Self>`.
  - `auth::Tokens { access_token, refresh_token: Option<String>, expires_at: i64 /*unix s*/, id_token: Option<String> }` (Serialize, Deserialize).
  - `auth::Account { id: String /*sub*/, email: String, name: String, image_url: Option<String> }`, built from unverified `id_token` claims `sub,email,name,picture`. It is display-only, and the server verifies the access token.
  - `auth::Pkce::new() -> Pkce { verifier, challenge, state }` (S256, 64-char verifier).
  - `auth::sign_in(cfg: &AuthConfig, open: impl FnOnce(&str), timeout: Duration) -> Result<(Tokens, Account), AuthError>`. It binds `127.0.0.1:{port}`, then calls `open(authorize_url)`, waits for `GET /callback?code&state`, serves a small "You can close this tab" HTML page, and exchanges the code at `{frontend_api}/oauth/token`.
  - `auth::refresh(cfg, refresh_token) -> Result<Tokens, AuthError>`.
  - `auth::Session` is a `Clone` wrapper, `Arc<Mutex<Option<Tokens>>>` + cfg:
    - `Session::load(cfg) -> Self` reads the keychain.
    - `Session::access_token(&self) -> Result<Option<String>, AuthError>` refreshes when `expires_at - now < 60` and persists the result.
    - `Session::sign_out()` clears the keychain.
    - `Session::account() -> Option<Account>`.
  - `AuthError` = `PortInUse | Timeout | Denied(String) | StateMismatch | Http(String) | NotConfigured`.
  - `keychain::KeyStore` trait (`get(user) -> Result<Option<String>>`, `set(user, &str)`, `delete(user)`), with `OsKeyStore` (keyring, service `com.radar.dev`) and `MemKeyStore` (pub, for tests in both crates). The Anthropic-key fns take `&dyn KeyStore`, and `ingest::run_briefing` uses `OsKeyStore`. OAuth tokens are stored as JSON under username `clerk_oauth`.
  - `Session::load(cfg, store: Arc<dyn KeyStore>)`.

- [ ] **Step 1: Write failing tests:**
  - `pkce_rfc7636_vector`: the verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` gives the challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`.
  - `authorize_url_has_params`: the URL contains `response_type=code`, `code_challenge_method=S256`, the URL-encoded redirect `http%3A%2F%2F127.0.0.1%3A17823%2Fcallback`, and `scope=openid+profile+email+offline_access`.
  - `callback_exchanges_code`: `httpmock` serves `/oauth/token`, and the test simulates the browser by GETting the loopback with the correct state. The result is `Ok`, and the request body had `grant_type=authorization_code`, `code_verifier`, `client_id`, and no `client_secret`.
  - `callback_state_mismatch_errors` returns `StateMismatch`.
  - `callback_error_param_is_denied`: `?error=access_denied` returns `Denied("access_denied")`.
  - `callback_times_out`: with a 200 ms timeout and no request, the result is `Timeout`.
  - `port_in_use_errors`: the port is pre-bound, and the result is `PortInUse`.
  - `session_refreshes_when_near_expiry`: `expires_at = now+30` causes one refresh call, and the new token is returned.
  - `account_from_id_token`: a hand-built unsigned JWT payload maps to `Account`.
- [ ] **Step 2: Run `cargo test -p radar-core auth`.** Expected: FAIL.
- [ ] **Step 3: Implement it.** Run the loopback listener on `tokio::net::TcpListener` with a minimal HTTP request-line parse; don't add a server framework. The token endpoint is form-encoded through reqwest. Tests set `frontend_api` to the httpmock base URL and use an ephemeral port from `AuthConfig { redirect_port: 0 }`. With port 0, `sign_in` reports the bound port through the `open` URL.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "feat(core): Clerk OAuth PKCE loopback sign-in"`.

