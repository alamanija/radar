//! Per-user profile sync. One row per user, keyed on Clerk's `sub`.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::etag::{etag_value, parse_if_match};
use crate::state::AppState;
use crate::types::{Profile, ProfileUpsert};

pub async fn get_profile(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<impl IntoResponse> {
    let row: Option<(Option<String>, Option<String>, Option<String>, DateTime<Utc>)> =
        sqlx::query_as(
            "select name, role, lens, updated_at from profiles where clerk_user_id = $1",
        )
        .bind(&user.id)
        .fetch_optional(&state.db)
        .await?;

    let (name, role, lens, updated_at) = match row {
        Some(r) => r,
        None => return Err(AppError::NotFound),
    };
    let body = Profile {
        name,
        role,
        lens,
        updated_at,
    };
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::ETAG, etag_value(updated_at));
    Ok((headers, Json(body)))
}

pub async fn put_profile(
    State(state): State<AppState>,
    user: ClerkUser,
    headers: HeaderMap,
    Json(body): Json<ProfileUpsert>,
) -> AppResult<impl IntoResponse> {
    validate_profile(&body)?;
    let if_match = parse_if_match(&headers)?;

    let mut tx = state.db.begin().await?;

    if let Some(expected) = if_match {
        let current: Option<DateTime<Utc>> = sqlx::query_scalar(
            "select updated_at from profiles where clerk_user_id = $1 for update",
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

    let row: (Option<String>, Option<String>, Option<String>, DateTime<Utc>) = sqlx::query_as(
        r#"
        insert into profiles (clerk_user_id, name, role, lens)
        values ($1, $2, $3, $4)
        on conflict (clerk_user_id) do update
            set name = excluded.name,
                role = excluded.role,
                lens = excluded.lens,
                updated_at = now()
        returning name, role, lens, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&body.name)
    .bind(&body.role)
    .bind(&body.lens)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let body = Profile {
        name: row.0,
        role: row.1,
        lens: row.2,
        updated_at: row.3,
    };
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(axum::http::header::ETAG, etag_value(body.updated_at));
    Ok((resp_headers, Json(body)))
}

fn validate_profile(p: &ProfileUpsert) -> AppResult<()> {
    for (field, value) in [("name", &p.name), ("role", &p.role), ("lens", &p.lens)] {
        if let Some(v) = value {
            if v.len() > 4096 {
                return Err(AppError::BadRequest(format!(
                    "{field}: exceeds 4096 characters"
                )));
            }
        }
    }
    Ok(())
}
