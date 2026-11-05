# Task 5 report: server accepts Clerk OAuth access tokens

Status: DONE (not verified against a live Clerk instance, per Ruling R3).

## Implemented
- `radar-server/src/clerk.rs`: `VerifyConfig { issuers, oauth_client_id }`; `verify(token, &JwksCache, &VerifyConfig)` (RS256 only, exp, `set_issuer(all issuers)`, no aud). `session_layer` is now a thin wrapper. Client-id check applies only to OAuth-shaped tokens (`client_id` or `scope`/`scp` present): `client_id` checked first, else `azp`; mismatch -> 401. Session JWTs (no client_id/scope) pass regardless of `azp`.
- `state.rs`: `clerk_issuer` replaced by `verify: Arc<VerifyConfig>`. `main.rs`: reads optional `CLERK_OAUTH_ISSUER`, `CLERK_OAUTH_CLIENT_ID` (empty = unset).
- `jwks.rs`: `#[cfg(test)] JwksCache::with_keys`.
- Tests in `radar-server/src/clerk/tests.rs` (RSA 2048 key generated at test time via `rsa` dev-dep): the 5 required tests.
- README (dashboard setup, env vars, sign_in example usage, bearer description) and `.env.example`.

## TDD
RED (verify stub using only first issuer, no client-id check): `cargo test -p radar-server` -> 3 passed, 2 FAILED (`accepts_oauth_issuer_when_configured`, `rejects_oauth_jwt_with_other_client_id`).
GREEN after implementing: `test result: ok. 5 passed; 0 failed`. No DB needed.

## Clippy
`cargo clippy -p radar-server --all-targets`: only a pre-existing `type_complexity` warning in `profile.rs:19`; none new.

## Commits
- 4fcfbfc feat(server): accept Clerk OAuth access tokens
- follow-up: chore: update Cargo.lock for rsa dev-dependency

## Concerns
- Live claims (iss/azp/client_id/scope) not observed; if Clerk OAuth tokens use `azp` only with no `scope` claim, the client-id check would not trigger (fail-open for that shape). Verify with the sign_in example.
- Token with OAuth shape but neither client_id nor azp passes when CLERK_OAUTH_CLIENT_ID is set (per spec).

## Fix round 1
Fixed fail-open: with CLERK_OAUTH_CLIENT_ID set, an OAuth-shaped token (scope/scp/client_id) must have client_id-else-azp equal to it; missing identifiers are rejected (`actual != Some(expected)`). Added test `rejects_oauth_jwt_missing_client_identifiers`. Note: the fix was applied in the same step as the test, so no separate RED run was captured for this test.
Command: `cargo test -p radar-server` -> 6 passed, 0 failed. Clippy: only the pre-existing profile.rs type_complexity warning.
The earlier concern about the missing-identifier case is resolved.
