//! Per-clerk-user token-bucket rate limiter. In-memory, not shared across
//! processes — if the server is horizontally scaled, swap this for a Redis
//! bucket. Single-node is fine for current scale.
//!
//! The bucket is keyed on the `sub` claim so one user's chatty client can't
//! drown out another user; requests without a ClerkUser extension (i.e.
//! unauth routes like /healthz) bypass the limiter entirely.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::Mutex;

use crate::clerk::ClerkUser;
use crate::error::AppError;

/// Tokens are replenished at `refill_rate` per second, capped at `capacity`.
/// Defaults pass bursty editing (fill the bucket then drain) while keeping
/// sustained rate bounded — ~10 rps per user.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<Inner>>,
    capacity: f64,
    refill_rate: f64,
    sweep_after: Duration,
}

struct Inner {
    buckets: HashMap<String, Bucket>,
    last_sweep: Instant,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                buckets: HashMap::new(),
                last_sweep: Instant::now(),
            })),
            capacity: capacity as f64,
            refill_rate: refill_per_sec as f64,
            sweep_after: Duration::from_secs(300),
        }
    }

    /// Try to consume a token. Returns true if allowed.
    async fn allow(&self, key: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let now = Instant::now();

        // Periodic sweep: drop buckets that have been full for a while so we
        // don't leak memory on a user-rich deployment. Cheap and bounded.
        if now.duration_since(inner.last_sweep) > self.sweep_after {
            let cap = self.capacity;
            let rate = self.refill_rate;
            inner.buckets.retain(|_, b| {
                let elapsed = now.duration_since(b.last_refill).as_secs_f64();
                (b.tokens + elapsed * rate) < cap
            });
            inner.last_sweep = now;
        }

        let bucket = inner
            .buckets
            .entry(key.to_string())
            .or_insert(Bucket {
                tokens: self.capacity,
                last_refill: now,
            });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_rate).min(self.capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Middleware layer for the /sync/* routes. Keys on the ClerkUser carried by
/// the upstream `session_layer`; unauthenticated requests would never reach
/// this layer (and if they did, they'd be rate-limited under a single "_"
/// key, which fails safe).
pub async fn layer(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let key = req
        .extensions()
        .get::<ClerkUser>()
        .map(|u| u.id.clone())
        .unwrap_or_else(|| "_".into());
    if !limiter.allow(&key).await {
        return Err(AppError::RateLimited);
    }
    Ok(next.run(req).await)
}
