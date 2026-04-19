//! Cached JWKS fetcher. Refetches on cache miss or when cache age exceeds
//! TTL. Safe for concurrent access. The URL is provided at construction so
//! this works equally for Google or Clerk (or any other RS256 issuer).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};

const CACHE_TTL: Duration = Duration::from_secs(60 * 60); // 1h

#[derive(Debug, Deserialize)]
struct JwksSet {
    keys: Vec<JwkEntry>,
}

#[derive(Debug, Deserialize)]
struct JwkEntry {
    kid: String,
    n: String,
    e: String,
    #[serde(default)]
    kty: String,
}

struct CacheInner {
    keys: HashMap<String, Arc<DecodingKey>>,
    fetched_at: Instant,
}

pub struct JwksCache {
    http: reqwest::Client,
    url: String,
    inner: RwLock<Option<CacheInner>>,
}

impl JwksCache {
    pub fn new(http: reqwest::Client, url: String) -> Arc<Self> {
        Arc::new(Self {
            http,
            url,
            inner: RwLock::new(None),
        })
    }

    /// Look up a decoding key by `kid`, refetching the JWKS set if the key
    /// isn't cached or the cache is stale.
    pub async fn get(&self, kid: &str) -> AppResult<Arc<DecodingKey>> {
        {
            let guard = self.inner.read().await;
            if let Some(inner) = guard.as_ref() {
                if inner.fetched_at.elapsed() < CACHE_TTL {
                    if let Some(k) = inner.keys.get(kid) {
                        return Ok(Arc::clone(k));
                    }
                }
            }
        }
        self.refresh().await?;
        let guard = self.inner.read().await;
        guard
            .as_ref()
            .and_then(|inner| inner.keys.get(kid).cloned())
            .ok_or_else(|| AppError::Unauthorized(format!("unknown jwks kid: {kid}")))
    }

    async fn refresh(&self) -> AppResult<()> {
        let set: JwksSet = self
            .http
            .get(&self.url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut keys = HashMap::with_capacity(set.keys.len());
        for k in set.keys {
            if k.kty != "RSA" {
                continue;
            }
            let key = DecodingKey::from_rsa_components(&k.n, &k.e)
                .map_err(|e| AppError::Internal(format!("decode jwk: {e}")))?;
            keys.insert(k.kid, Arc::new(key));
        }

        let mut guard = self.inner.write().await;
        *guard = Some(CacheInner {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }
}
