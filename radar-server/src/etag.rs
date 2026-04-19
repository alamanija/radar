//! ETag parsing + conflict-detection plumbing. The server uses each row's
//! `updated_at` (RFC 3339) as a weak ETag. Clients echo the value back via
//! `If-Match` on PUT; a mismatch against the current server-side timestamp
//! indicates a concurrent write and must return 412 Precondition Failed.
//!
//! Format on the wire: `W/"2026-04-19T12:34:56.789012+00:00"` (weak) or the
//! same string unquoted/without the `W/` prefix. We accept any of those
//! shapes to be lenient; the canonical form we emit is the weak variant.

use axum::http::{header, HeaderMap, HeaderValue};
use chrono::{DateTime, Utc};

use crate::error::{AppError, AppResult};

/// Pull the `If-Match` header off a request and parse it as the RFC 3339
/// timestamp the server previously emitted. Returns `None` if the client
/// didn't supply one — absent `If-Match` means "don't care", so the caller
/// should treat that as a first-write / trust-the-client path.
pub fn parse_if_match(headers: &HeaderMap) -> AppResult<Option<DateTime<Utc>>> {
    let raw = match headers.get(header::IF_MATCH) {
        Some(v) => v,
        None => return Ok(None),
    };
    let s = raw
        .to_str()
        .map_err(|_| AppError::BadRequest("If-Match: non-ascii".into()))?
        .trim();
    if s == "*" {
        // Not meaningful for our upsert semantics. Treat as "don't care".
        return Ok(None);
    }
    let unquoted = strip_etag_wrapping(s);
    let ts = DateTime::parse_from_rfc3339(unquoted)
        .map_err(|e| AppError::BadRequest(format!("If-Match: bad timestamp: {e}")))?
        .with_timezone(&Utc);
    Ok(Some(ts))
}

fn strip_etag_wrapping(s: &str) -> &str {
    let s = s.strip_prefix("W/").unwrap_or(s);
    s.trim_matches('"')
}

/// Build the `ETag` response-header value for a timestamp. Weak because we
/// don't care about byte-level identity — semantic equality is enough.
pub fn etag_value(ts: DateTime<Utc>) -> HeaderValue {
    // RFC 3339 is ASCII-safe; unwrap is fine.
    HeaderValue::from_str(&format!("W/\"{}\"", ts.to_rfc3339())).expect("ascii")
}
