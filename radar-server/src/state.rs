use std::sync::Arc;

use sqlx::postgres::PgPool;

use crate::jwks::JwksCache;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwks: Arc<JwksCache>,
    /// Accepted issuers / optional OAuth client id for Clerk bearer tokens.
    pub verify: Arc<crate::clerk::VerifyConfig>,
}
