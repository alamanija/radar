### Task 5: Server accepts Clerk OAuth access tokens

**Files:**
- Modify: `radar-server/src/clerk.rs`, `radar-server/src/state.rs`, `radar-server/src/main.rs`, `radar-server/.env.example`, `radar-server/README.md`

**Interfaces:**
- Produces: the optional env `CLERK_OAUTH_CLIENT_ID`. When set, a token that carries `client_id` (or `azp`) must equal it. Tokens are still RS256, use the same JWKS, and must have `iss == CLERK_ISSUER`. `sub` becomes `ClerkUser.id`, unchanged.

- [ ] **Step 1: Manual check against a live Clerk dev instance.** Create the OAuth app as **Public** with **Require PKCE**, the redirect `http://127.0.0.1:17823/callback`, the `offline_access` scope, and the JWT access token format. Then sign in once with the `radar-core` example `cargo run -p radar-core --example sign_in`, which prints the decoded *claims* only, never the token. Record `iss`, `sub`, `aud`, `azp`, `client_id` and `scope` in the commit message. If `iss` differs from the session issuer, also accept a configured `CLERK_OAUTH_ISSUER`.
- [ ] **Step 2: Write failing tests** in `clerk.rs`, signing with a test RSA key and injecting the JWKS into `JwksCache` through a test constructor:
  - `accepts_oauth_jwt_with_matching_client_id`;
  - `rejects_oauth_jwt_with_other_client_id`;
  - `accepts_session_jwt_without_client_id`, which keeps today's behavior;
  - `rejects_wrong_issuer`.
- [ ] **Step 3: Run `cargo test -p radar-server clerk`.** Expected: FAIL.
- [ ] **Step 4: Implement it.** Extend `ClerkClaims` with `client_id: Option<String>` and `azp: Option<String>`. Update the README: bearer is now a Clerk OAuth access token from the desktop app, plus the dashboard setup steps from Step 1.
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit** with `git commit -m "feat(server): accept Clerk OAuth access tokens"`.

