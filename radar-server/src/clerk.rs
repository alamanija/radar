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
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct ClerkUser {
    pub id: String,
}

#[derive(Debug, Deserialize)]
struct ClerkClaims {
    sub: String,
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

    let header = decode_header(&token)?;
    let kid = header
        .kid
        .ok_or_else(|| AppError::Unauthorized("missing kid".into()))?;
    let key = state.jwks.get(&kid).await?;

    // Lock down the algorithm rather than trusting the token header — guards
    // against alg-confusion attacks. Clerk session JWTs are always RS256.
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[state.clerk_issuer.as_str()]);
    validation.validate_exp = true;
    // Clerk's default session template doesn't set `aud`, so skip that check
    // instead of requiring an exact match.
    validation.validate_aud = false;

    let data = decode::<ClerkClaims>(&token, &key, &validation)?;

    req.extensions_mut().insert(ClerkUser {
        id: data.claims.sub,
    });
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
