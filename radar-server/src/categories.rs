//! Per-user categories sync. One jsonb blob per user, full-list replace.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::etag::{etag_value, parse_if_match};
use crate::state::AppState;
use crate::types::{CategoriesSnapshot, CategoriesUpsert, Category};

const MAX_CATEGORIES: usize = 64;
const MAX_STRING_LEN: usize = 2048;

pub async fn get_categories(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<impl IntoResponse> {
    let row: Option<(serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
        "select categories, updated_at from user_categories where clerk_user_id = $1",
    )
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some((value, updated_at)) => {
            let categories: Vec<Category> = serde_json::from_value(value)
                .map_err(|e| AppError::Internal(format!("decode categories: {e}")))?;
            let body = CategoriesSnapshot {
                categories,
                updated_at,
            };
            let mut headers = HeaderMap::new();
            headers.insert(axum::http::header::ETAG, etag_value(updated_at));
            Ok((headers, Json(body)))
        }
        None => Err(AppError::NotFound),
    }
}

pub async fn put_categories(
    State(state): State<AppState>,
    user: ClerkUser,
    headers: HeaderMap,
    Json(body): Json<CategoriesUpsert>,
) -> AppResult<impl IntoResponse> {
    validate_categories(&body.categories)?;
    let if_match = parse_if_match(&headers)?;

    let encoded = serde_json::to_value(&body.categories)
        .map_err(|e| AppError::Internal(format!("encode categories: {e}")))?;

    let mut tx = state.db.begin().await?;

    if let Some(expected) = if_match {
        let current: Option<DateTime<Utc>> = sqlx::query_scalar(
            "select updated_at from user_categories where clerk_user_id = $1 for update",
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
        insert into user_categories (clerk_user_id, categories)
        values ($1, $2)
        on conflict (clerk_user_id) do update
            set categories = excluded.categories,
                updated_at = now()
        returning categories, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&encoded)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let categories: Vec<Category> = serde_json::from_value(row.0)
        .map_err(|e| AppError::Internal(format!("decode categories: {e}")))?;
    let snapshot = CategoriesSnapshot {
        categories,
        updated_at: row.1,
    };
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(axum::http::header::ETAG, etag_value(snapshot.updated_at));
    Ok((resp_headers, Json(snapshot)))
}

fn validate_categories(cats: &[Category]) -> AppResult<()> {
    if cats.len() > MAX_CATEGORIES {
        return Err(AppError::BadRequest(format!(
            "too many categories (max {MAX_CATEGORIES})"
        )));
    }
    let mut seen_ids = std::collections::HashSet::with_capacity(cats.len());
    for c in cats {
        if c.id.is_empty() || c.id.len() > 128 {
            return Err(AppError::BadRequest("category id: empty or >128".into()));
        }
        if !seen_ids.insert(&c.id) {
            return Err(AppError::BadRequest(format!("duplicate id: {}", c.id)));
        }
        if c.label.len() > MAX_STRING_LEN {
            return Err(AppError::BadRequest("category label too long".into()));
        }
        if c.description.len() > MAX_STRING_LEN {
            return Err(AppError::BadRequest("category description too long".into()));
        }
        if c.accent.len() > 64 {
            return Err(AppError::BadRequest("category accent too long".into()));
        }
    }
    Ok(())
}
