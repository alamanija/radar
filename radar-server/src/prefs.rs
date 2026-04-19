//! Per-user app preferences sync. One jsonb blob per user, full replace.
//! Contents are opaque to the server; we still enforce that the top level
//! is a JSON object and that the payload is small.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::etag::{etag_value, parse_if_match};
use crate::state::AppState;
use crate::types::{PrefsSnapshot, PrefsUpsert};

const MAX_PAYLOAD_BYTES: usize = 64 * 1024; // 64 KiB — prefs are tiny

pub async fn get_prefs(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<impl IntoResponse> {
    let row: Option<(serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
        "select prefs, updated_at from user_prefs where clerk_user_id = $1",
    )
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some((prefs, updated_at)) => {
            let body = PrefsSnapshot { prefs, updated_at };
            let mut headers = HeaderMap::new();
            headers.insert(axum::http::header::ETAG, etag_value(updated_at));
            Ok((headers, Json(body)))
        }
        None => Err(AppError::NotFound),
    }
}

pub async fn put_prefs(
    State(state): State<AppState>,
    user: ClerkUser,
    headers: HeaderMap,
    Json(body): Json<PrefsUpsert>,
) -> AppResult<impl IntoResponse> {
    validate_prefs(&body.prefs)?;
    let if_match = parse_if_match(&headers)?;

    let mut tx = state.db.begin().await?;

    if let Some(expected) = if_match {
        let current: Option<DateTime<Utc>> = sqlx::query_scalar(
            "select updated_at from user_prefs where clerk_user_id = $1 for update",
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
        insert into user_prefs (clerk_user_id, prefs)
        values ($1, $2)
        on conflict (clerk_user_id) do update
            set prefs = excluded.prefs,
                updated_at = now()
        returning prefs, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&body.prefs)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let snapshot = PrefsSnapshot {
        prefs: row.0,
        updated_at: row.1,
    };
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(axum::http::header::ETAG, etag_value(snapshot.updated_at));
    Ok((resp_headers, Json(snapshot)))
}

fn validate_prefs(value: &serde_json::Value) -> AppResult<()> {
    if !value.is_object() {
        return Err(AppError::BadRequest("prefs must be a JSON object".into()));
    }
    let size = serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if size > MAX_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(format!(
            "prefs payload too large ({size} bytes)"
        )));
    }
    Ok(())
}
