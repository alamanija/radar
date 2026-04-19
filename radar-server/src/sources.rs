//! Per-user sources sync. One jsonb blob per user, full-list replace.
//! Only user-editable fields live on the wire; device-local `lastFetchAt`
//! and `health` never leave the client.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::etag::{etag_value, parse_if_match};
use crate::state::AppState;
use crate::types::{Source, SourcesSnapshot, SourcesUpsert};

const MAX_SOURCES: usize = 512;
const MAX_STRING_LEN: usize = 2048;
const MAX_URL_LEN: usize = 4096;

pub async fn get_sources(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<impl IntoResponse> {
    let row: Option<(serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
        "select sources, updated_at from user_sources where clerk_user_id = $1",
    )
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some((value, updated_at)) => {
            let sources: Vec<Source> = serde_json::from_value(value)
                .map_err(|e| AppError::Internal(format!("decode sources: {e}")))?;
            let body = SourcesSnapshot {
                sources,
                updated_at,
            };
            let mut headers = HeaderMap::new();
            headers.insert(axum::http::header::ETAG, etag_value(updated_at));
            Ok((headers, Json(body)))
        }
        None => Err(AppError::NotFound),
    }
}

pub async fn put_sources(
    State(state): State<AppState>,
    user: ClerkUser,
    headers: HeaderMap,
    Json(body): Json<SourcesUpsert>,
) -> AppResult<impl IntoResponse> {
    validate_sources(&body.sources)?;
    let if_match = parse_if_match(&headers)?;

    let encoded = serde_json::to_value(&body.sources)
        .map_err(|e| AppError::Internal(format!("encode sources: {e}")))?;

    let mut tx = state.db.begin().await?;

    if let Some(expected) = if_match {
        let current: Option<DateTime<Utc>> = sqlx::query_scalar(
            "select updated_at from user_sources where clerk_user_id = $1 for update",
        )
        .bind(&user.id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(cur) = current {
            if cur != expected {
                return Err(AppError::PreconditionFailed);
            }
        }
    }

    let row: (serde_json::Value, DateTime<Utc>) = sqlx::query_as(
        r#"
        insert into user_sources (clerk_user_id, sources)
        values ($1, $2)
        on conflict (clerk_user_id) do update
            set sources = excluded.sources,
                updated_at = now()
        returning sources, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&encoded)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let sources: Vec<Source> = serde_json::from_value(row.0)
        .map_err(|e| AppError::Internal(format!("decode sources: {e}")))?;
    let snapshot = SourcesSnapshot {
        sources,
        updated_at: row.1,
    };
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(axum::http::header::ETAG, etag_value(snapshot.updated_at));
    Ok((resp_headers, Json(snapshot)))
}

fn validate_sources(sources: &[Source]) -> AppResult<()> {
    if sources.len() > MAX_SOURCES {
        return Err(AppError::BadRequest(format!(
            "too many sources (max {MAX_SOURCES})"
        )));
    }
    let mut seen_ids = std::collections::HashSet::with_capacity(sources.len());
    for s in sources {
        if !seen_ids.insert(s.id) {
            return Err(AppError::BadRequest(format!("duplicate source id: {}", s.id)));
        }
        if s.name.is_empty() || s.name.len() > MAX_STRING_LEN {
            return Err(AppError::BadRequest("source name: empty or too long".into()));
        }
        if s.category.len() > 128 {
            return Err(AppError::BadRequest("source category too long".into()));
        }
        if let Some(url) = &s.feed_url {
            if url.len() > MAX_URL_LEN {
                return Err(AppError::BadRequest("feedUrl too long".into()));
            }
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(AppError::BadRequest("feedUrl must be http(s)".into()));
            }
        }
    }
    Ok(())
}
