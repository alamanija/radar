use std::sync::Arc;

use sqlx::postgres::PgPool;

use crate::jwks::JwksCache;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwks: Arc<JwksCache>,
    /// Expected `iss` claim on Clerk session JWTs. Typically looks like
    /// `https://<your-app>.clerk.accounts.dev` or a custom Clerk domain.
    pub clerk_issuer: Arc<String>,
}
