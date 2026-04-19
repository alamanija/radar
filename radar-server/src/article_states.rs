//! Per-(user, article) read/bookmark state. GET returns the full set for
//! the current user; PATCH upserts a single row. Keyed per row so rapid
//! toggles don't round-trip the entire list.

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::clerk::ClerkUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::types::{ArticleState, ArticleStatePatch, ArticleStatesResponse};

const MAX_ARTICLE_ID_LEN: usize = 512;

pub async fn get_article_states(
    State(state): State<AppState>,
    user: ClerkUser,
) -> AppResult<Json<ArticleStatesResponse>> {
    let rows: Vec<(String, bool, bool, DateTime<Utc>)> = sqlx::query_as(
        "select article_id, read, bookmarked, updated_at \
         from user_article_states where clerk_user_id = $1",
    )
    .bind(&user.id)
    .fetch_all(&state.db)
    .await?;

    let states = rows
        .into_iter()
        .map(|(article_id, read, bookmarked, updated_at)| ArticleState {
            article_id,
            read,
            bookmarked,
            updated_at,
        })
        .collect();

    Ok(Json(ArticleStatesResponse { states }))
}

pub async fn patch_article_state(
    State(state): State<AppState>,
    user: ClerkUser,
    Json(body): Json<ArticleStatePatch>,
) -> AppResult<Json<ArticleState>> {
    if body.article_id.is_empty() || body.article_id.len() > MAX_ARTICLE_ID_LEN {
        return Err(AppError::BadRequest("articleId: empty or too long".into()));
    }
    let row: (String, bool, bool, DateTime<Utc>) = sqlx::query_as(
        r#"
        insert into user_article_states (clerk_user_id, article_id, read, bookmarked)
        values ($1, $2, $3, $4)
        on conflict (clerk_user_id, article_id) do update
            set read = excluded.read,
                bookmarked = excluded.bookmarked,
                updated_at = now()
        returning article_id, read, bookmarked, updated_at
        "#,
    )
    .bind(&user.id)
    .bind(&body.article_id)
    .bind(body.read)
    .bind(body.bookmarked)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ArticleState {
        article_id: row.0,
        read: row.1,
        bookmarked: row.2,
        updated_at: row.3,
    }))
}
