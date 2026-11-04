//! Clerk JWT verification. Clerk owns identity — the server just checks the
//! session JWT on every authed request, extracts `sub` as the user key, and
//! carries it through in request extensions. No sessions table, no user row.

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::Deserialize;

use crate::error::AppError;
use crate::jwks::JwksCache;
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct ClerkUser {
    pub id: String,
}

/// What the verifier accepts. Session JWTs and OAuth access tokens are both
/// RS256, signed by the same instance key (same JWKS).
#[derive(Clone, Debug)]
pub struct VerifyConfig {
    /// Accepted `iss` values: `CLERK_ISSUER` plus optional `CLERK_OAUTH_ISSUER`.
    pub issuers: Vec<String>,
    /// `CLERK_OAUTH_CLIENT_ID`. When set, OAuth-shaped tokens must carry it.
    pub oauth_client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClerkClaims {
    sub: String,
    client_id: Option<String>,
    azp: Option<String>,
    scope: Option<serde::de::IgnoredAny>,
    scp: Option<serde::de::IgnoredAny>,
}

/// Verify a bearer token and return the user it identifies.
pub async fn verify(
    token: &str,
    jwks: &JwksCache,
    cfg: &VerifyConfig,
) -> Result<ClerkUser, AppError> {
    let header = decode_header(token)?;
    let kid = header
        .kid
        .ok_or_else(|| AppError::Unauthorized("missing kid".into()))?;
    let key = jwks.get(&kid).await?;

    // Lock down the algorithm rather than trusting the token header — guards
    // against alg-confusion attacks. Clerk tokens are always RS256.
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&cfg.issuers);
    validation.validate_exp = true;
    // Clerk tokens don't carry a useful `aud`, so skip that check.
    validation.validate_aud = false;

    let claims = decode::<ClerkClaims>(token, &key, &validation)?.claims;

    // Session JWTs carry `azp` = the frontend origin, so the client-id match
    // applies only to tokens that look like OAuth access tokens.
    if let Some(expected) = &cfg.oauth_client_id {
        let is_oauth = claims.client_id.is_some() || claims.scope.is_some() || claims.scp.is_some();
        if is_oauth {
            let actual = claims.client_id.as_ref().or(claims.azp.as_ref());
            if actual != Some(expected) {
                return Err(AppError::Unauthorized("client_id mismatch".into()));
            }
        }
    }
    Ok(ClerkUser { id: claims.sub })
}

pub async fn session_layer(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .ok_or_else(|| AppError::Unauthorized("missing Authorization".into()))?;

    let user = verify(&token, &state.jwks, &state.verify).await?;
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for ClerkUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<ClerkUser>()
            .cloned()
            .ok_or_else(|| AppError::Internal("session_layer missing".into()))
    }
}

#[cfg(test)]
mod tests;
