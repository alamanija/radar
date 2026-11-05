//! Per-article read/bookmark sync (`/sync/article-states`). Per-row PATCH
//! semantics, so this doesn't ride `SyncedResource`.

use super::queue::QueuedReq;
use super::{Resp, SyncClient, SyncError};
use crate::model::ArticleState;
use crate::types::Article;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const ENDPOINT: &str = "/sync/article-states";

/// Queue group key: repeated toggles on one article dedup.
pub fn group_key(id: &str) -> String {
    format!("PATCH:{ENDPOINT}:{id}")
}

/// All server-side states for this account, keyed by article id.
pub async fn pull(client: &SyncClient) -> Result<HashMap<String, ArticleState>, SyncError> {
    let resp = client.get(ENDPOINT).await?;
    if !resp.ok() {
        return Err(SyncError::Http {
            status: resp.status,
            path: ENDPOINT.to_string(),
        });
    }
    let states = resp
        .body
        .get("states")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(states
        .iter()
        .filter_map(|s| {
            let id = s.get("articleId")?.as_str()?.to_string();
            let flag = |k: &str| s.get(k).and_then(Value::as_bool).unwrap_or(false);
            Some((
                id,
                ArticleState {
                    read: flag("read"),
                    bookmarked: flag("bookmarked"),
                },
            ))
        })
        .collect())
}

/// Server states win over local flags for the ids they cover.
pub fn overlay(articles: &mut [Article], server: &HashMap<String, ArticleState>) {
    for a in articles {
        if let Some(s) = server.get(&a.id.to_string()) {
            a.read = s.read;
            a.bookmarked = s.bookmarked;
        }
    }
}

/// PATCHes local read/bookmarked flags the server doesn't know about yet.
/// Ids the server already has are skipped (its copy wins). Call with the
/// articles as they were *before* the overlay. Returns the seeded states.
pub async fn seed_local_only(
    client: &SyncClient,
    local_articles: &[Article],
    server: &HashMap<String, ArticleState>,
) -> Vec<(String, ArticleState)> {
    let mut seeded = Vec::new();
    for a in local_articles {
        let id = a.id.to_string();
        if server.contains_key(&id) || (!a.read && !a.bookmarked) {
            continue;
        }
        let state = ArticleState {
            read: a.read,
            bookmarked: a.bookmarked,
        };
        if let Err(e) = patch(client, &id, state).await {
            log::warn!("[sync] seed article state failed: {e}");
        }
        seeded.push((id, state));
    }
    seeded
}

/// One toggle → one PATCH (queued on transient failure).
pub async fn patch(client: &SyncClient, id: &str, state: ArticleState) -> Result<Resp, SyncError> {
    let body = json!({ "articleId": id, "read": state.read, "bookmarked": state.bookmarked });
    client
        .mutate(QueuedReq::new("PATCH", ENDPOINT, body, Some(group_key(id))))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::client;
    use httpmock::prelude::*;

    fn article(id: u64, read: bool, bookmarked: bool) -> Article {
        Article {
            id,
            title: format!("T{id}"),
            source: "S".into(),
            source_id: 1,
            category: "c".into(),
            category_label: "C".into(),
            summary: String::new(),
            published: String::new(),
            published_at: None,
            read,
            bookmarked,
            accent: "camel".into(),
            url: String::new(),
            image_url: None,
            summarized_by: None,
            importance: None,
        }
    }

    #[tokio::test]
    async fn pull_maps_states_by_id() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/sync/article-states");
                then.status(200).json_body(json!({"states": [
                    {"articleId": "1", "read": true, "bookmarked": false, "updatedAt": "t"},
                    {"articleId": "2", "read": false, "bookmarked": true, "updatedAt": "t"}
                ]}));
            })
            .await;
        let c = client(&server.base_url());
        let m = pull(&c).await.unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(
            m["1"],
            ArticleState {
                read: true,
                bookmarked: false
            }
        );
        assert_eq!(
            m["2"],
            ArticleState {
                read: false,
                bookmarked: true
            }
        );

        let mut arts = vec![article(1, false, true), article(3, true, false)];
        overlay(&mut arts, &m);
        assert!(arts[0].read && !arts[0].bookmarked);
        assert!(arts[1].read);
    }

    #[tokio::test]
    async fn article_states_seed_skips_server_known_ids() {
        let server = MockServer::start_async().await;
        let seeded = server
            .mock_async(|when, then| {
                when.method(PATCH)
                    .path("/sync/article-states")
                    .json_body(json!({"articleId": "2", "read": false, "bookmarked": true}));
                then.status(200).json_body(json!({
                    "articleId": "2", "read": false, "bookmarked": true, "updatedAt": "t"
                }));
            })
            .await;
        let known = server
            .mock_async(|when, then| {
                when.method(PATCH)
                    .path("/sync/article-states")
                    .json_body_includes(r#"{"articleId": "1"}"#);
                then.status(200).json_body(json!({}));
            })
            .await;
        let unflagged = server
            .mock_async(|when, then| {
                when.method(PATCH)
                    .path("/sync/article-states")
                    .json_body_includes(r#"{"articleId": "3"}"#);
                then.status(200).json_body(json!({}));
            })
            .await;
        let c = client(&server.base_url());
        let server_map = HashMap::from([(
            "1".to_string(),
            ArticleState {
                read: false,
                bookmarked: false,
            },
        )]);
        let local = vec![
            article(1, true, true),   // server knows it: skipped
            article(2, false, true),  // local-only flag: seeded
            article(3, false, false), // nothing to seed
        ];
        let out = seed_local_only(&c, &local, &server_map).await;
        seeded.assert_calls_async(1).await;
        known.assert_calls_async(0).await;
        unflagged.assert_calls_async(0).await;
        assert_eq!(
            out,
            vec![(
                "2".to_string(),
                ArticleState {
                    read: false,
                    bookmarked: true
                }
            )]
        );
    }

    #[tokio::test]
    async fn patch_uses_per_article_group_key() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(PATCH).path("/sync/article-states");
                then.status(503);
            })
            .await;
        let c = client(&server.base_url());
        let st = ArticleState {
            read: true,
            bookmarked: false,
        };
        assert_eq!(patch(&c, "7", st).await.unwrap().status, 202);
        assert_eq!(patch(&c, "7", st).await.unwrap().status, 202);
        assert_eq!(patch(&c, "8", st).await.unwrap().status, 202);
        let q = c.queued();
        assert_eq!(q.len(), 2);
        assert_eq!(
            q[0].group_key.as_deref(),
            Some("PATCH:/sync/article-states:7")
        );
        assert_eq!(q[0].method, "PATCH");
        assert_eq!(
            q[0].body,
            json!({"articleId": "7", "read": true, "bookmarked": false})
        );
    }
}
