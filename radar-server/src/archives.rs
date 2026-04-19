//! Per-user archives sync. One jsonb blob per user, full-list replace.
//! Archive contents are mostly opaque to the server — we still sanity-check
//! the top-level shape (array, capped, each entry has an id + runAt) to
//! prevent a bad client from poisoning a user's archive blob.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::etag::{etag_value, parse_if_match};
use crate::state::AppState;
use crate::types::{ArchivesSnapshot, ArchivesUpsert};

const MAX_ARCHIVES: usize = 90;
const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

pub async fn get_archives(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<impl IntoResponse> {
    let row: Option<(serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
        "select archives, updated_at from user_archives where clerk_user_id = $1",
    )
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some((archives, updated_at)) => {
            let body = ArchivesSnapshot {
                archives,
                updated_at,
            };
            let mut headers = HeaderMap::new();
            headers.insert(axum::http::header::ETAG, etag_value(updated_at));
            Ok((headers, Json(body)))
        }
        None => Err(AppError::NotFound),
    }
}

pub async fn put_archives(
    State(state): State<AppState>,
    user: ClerkUser,
    headers: HeaderMap,
    Json(body): Json<ArchivesUpsert>,
) -> AppResult<impl IntoResponse> {
    validate_archives(&body.archives)?;
    let if_match = parse_if_match(&headers)?;

    let mut tx = state.db.begin().await?;

    if let Some(expected) = if_match {
        let current: Option<DateTime<Utc>> = sqlx::query_scalar(
            "select updated_at from user_archives where clerk_user_id = $1 for update",
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
        insert into user_archives (clerk_user_id, archives)
        values ($1, $2)
        on conflict (clerk_user_id) do update
            set archives = excluded.archives,
                updated_at = now()
        returning archives, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&body.archives)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let snapshot = ArchivesSnapshot {
        archives: row.0,
        updated_at: row.1,
    };
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(axum::http::header::ETAG, etag_value(snapshot.updated_at));
    Ok((resp_headers, Json(snapshot)))
}

fn validate_archives(value: &serde_json::Value) -> AppResult<()> {
    let arr = value
        .as_array()
        .ok_or_else(|| AppError::BadRequest("archives must be an array".into()))?;
    if arr.len() > MAX_ARCHIVES {
        return Err(AppError::BadRequest(format!(
            "too many archives (max {MAX_ARCHIVES})"
        )));
    }
    // Ballpark size guard — serialize once to measure. Cheap relative to the
    // round-trip cost.
    let size = serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if size > MAX_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(format!(
            "archives payload too large ({size} bytes)"
        )));
    }
    let mut seen_ids = std::collections::HashSet::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        let obj = entry.as_object().ok_or_else(|| {
            AppError::BadRequest(format!("archives[{i}] must be an object"))
        })?;
        let id = obj
            .get("id")
            .ok_or_else(|| AppError::BadRequest(format!("archives[{i}] missing id")))?;
        let id_str = match id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => {
                return Err(AppError::BadRequest(format!(
                    "archives[{i}].id must be string or number"
                )))
            }
        };
        if !seen_ids.insert(id_str.clone()) {
            return Err(AppError::BadRequest(format!(
                "duplicate archive id: {id_str}"
            )));
        }
        if obj.get("runAt").and_then(|v| v.as_str()).is_none() {
            return Err(AppError::BadRequest(format!(
                "archives[{i}] missing runAt"
            )));
        }
    }
    Ok(())
}
