use chrono::{DateTime, Utc};
use feed_rs::parser;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use crate::keychain;
use crate::summarize;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceInput {
    pub id: u32,
    pub name: String,
    /// Category id, e.g. "packaging". Resolved via the categories map.
    pub category: String,
    pub feed_url: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub id: String,
    pub label: String,
    pub description: String,
    pub accent: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub id: u64,
    pub title: String,
    pub source: String,
    pub source_id: u32,
    pub category: String,
    pub category_label: String,
    pub summary: String,
    pub published: String,
    pub published_at: Option<DateTime<Utc>>,
    pub read: bool,
    pub bookmarked: bool,
    pub accent: String,
    pub url: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceError {
    pub source_id: u32,
    pub source_name: String,
    pub message: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BriefingResponse {
    pub articles: Vec<Article>,
    pub errors: Vec<SourceError>,
    pub fetched_at: DateTime<Utc>,
}

/// Resolve a category id to (label, accent), with a safe fallback for ids
/// that no longer exist in the user's category list (e.g. they deleted a
/// category a source still refers to, or Claude returned something unknown).
fn resolve(cats: &HashMap<String, CategoryInput>, id: &str) -> (String, String, String) {
    match cats.get(id) {
        Some(c) => (c.id.clone(), c.label.clone(), c.accent.clone()),
        None => (
            "uncategorized".to_string(),
            "Uncategorized".to_string(),
            "ink".to_string(),
        ),
    }
}

#[tauri::command]
pub async fn ingest_briefing(
    sources: Vec<SourceInput>,
    categories: Vec<CategoryInput>,
    lens: Option<String>,
) -> Result<BriefingResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent("Radar/0.1 (desktop news briefing)")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let cat_map: Arc<HashMap<String, CategoryInput>> = Arc::new(
        categories.iter().map(|c| (c.id.clone(), c.clone())).collect(),
    );

    let targets: Vec<SourceInput> = sources
        .into_iter()
        .filter(|s| s.feed_url.as_deref().is_some_and(|u| !u.is_empty()))
        .collect();

    let fetches = targets.into_iter().map(|src| {
        let client = client.clone();
        let cats = Arc::clone(&cat_map);
        async move { fetch_source(&client, src, &cats).await }
    });
    let results = join_all(fetches).await;

    let mut articles: Vec<Article> = Vec::new();
    let mut errors: Vec<SourceError> = Vec::new();
    for result in results {
        match result {
            Ok(mut fetched) => articles.append(&mut fetched),
            Err(e) => errors.push(e),
        }
    }

    // Claude pass: requires both an API key AND at least one category (the
    // output schema's enum can't be empty).
    if !categories.is_empty() {
        if let Some(api_key) = keychain::read_api_key() {
            let claude_cats: Vec<(String, String)> = categories
                .iter()
                .map(|c| (c.id.clone(), c.description.clone()))
                .collect();
            let claude_cats = Arc::new(claude_cats);
            let lens_arc = Arc::new(lens.clone());

            let futures = articles.into_iter().map(|article| {
                let client = client.clone();
                let api_key = api_key.clone();
                let cats = Arc::clone(&cat_map);
                let claude_cats = Arc::clone(&claude_cats);
                let lens = Arc::clone(&lens_arc);
                async move { enhance(&client, &api_key, article, &cats, &claude_cats, lens.as_deref()).await }
            });
            let enhanced = join_all(futures).await;

            articles = Vec::with_capacity(enhanced.len());
            let mut claude_failures = 0usize;
            let mut first_claude_error: Option<String> = None;
            for result in enhanced {
                match result {
                    Ok(a) => articles.push(a),
                    Err((a, msg)) => {
                        claude_failures += 1;
                        if first_claude_error.is_none() {
                            first_claude_error = Some(msg);
                        }
                        articles.push(a);
                    }
                }
            }
            if claude_failures > 0 {
                errors.push(SourceError {
                    source_id: 0,
                    source_name: "Claude (summarization)".to_string(),
                    message: format!(
                        "{claude_failures} article(s) not summarized: {}",
                        first_claude_error.unwrap_or_default()
                    ),
                });
            }
        }
    }

    // Most recent first; articles without a timestamp fall to the bottom.
    articles.sort_by(|a, b| match (b.published_at, a.published_at) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    Ok(BriefingResponse {
        articles,
        errors,
        fetched_at: Utc::now(),
    })
}

async fn enhance(
    client: &reqwest::Client,
    api_key: &str,
    mut article: Article,
    cats: &HashMap<String, CategoryInput>,
    claude_cats: &[(String, String)],
    lens: Option<&str>,
) -> Result<Article, (Article, String)> {
    match summarize::summarize(
        client,
        api_key,
        &article.title,
        &article.source,
        &article.summary,
        claude_cats,
        lens,
    )
    .await
    {
        Ok(s) => {
            article.summary = s.summary;
            let (cat, label, accent) = resolve(cats, &s.category);
            article.category = cat;
            article.category_label = label;
            article.accent = accent;
            Ok(article)
        }
        Err(msg) => Err((article, msg)),
    }
}

async fn fetch_source(
    client: &reqwest::Client,
    src: SourceInput,
    cats: &HashMap<String, CategoryInput>,
) -> Result<Vec<Article>, SourceError> {
    let err = |msg: String| SourceError {
        source_id: src.id,
        source_name: src.name.clone(),
        message: msg,
    };

    let url = src.feed_url.as_deref().unwrap_or_default();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| err(format!("fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| err(format!("bad status: {e}")))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| err(format!("read body: {e}")))?;

    let feed = parser::parse(&bytes[..]).map_err(|e| err(format!("parse: {e}")))?;

    let now = Utc::now();
    let (category, category_label, accent) = resolve(cats, &src.category);

    let articles: Vec<Article> = feed
        .entries
        .iter()
        .take(10)
        .filter_map(|entry| {
            let title = entry.title.as_ref().map(|t| t.content.trim().to_string())?;
            let url = entry.links.first().map(|l| l.href.clone())?;

            let raw_summary = entry
                .summary
                .as_ref()
                .map(|t| t.content.clone())
                .or_else(|| entry.content.as_ref().and_then(|c| c.body.clone()))
                .unwrap_or_default();
            let summary = strip_html_and_truncate(&raw_summary, 280);

            let published_at = entry.published.or(entry.updated);
            let published = published_at
                .map(|t| relative_time(t, now))
                .unwrap_or_else(|| "recent".to_string());

            Some(Article {
                id: hash_id(src.id, &url),
                title,
                source: src.name.clone(),
                source_id: src.id,
                category: category.clone(),
                category_label: category_label.clone(),
                summary,
                published,
                published_at,
                read: false,
                bookmarked: false,
                accent: accent.clone(),
                url,
            })
        })
        .collect();

    Ok(articles)
}

fn relative_time(when: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let diff = now.signed_duration_since(when);
    let secs = diff.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else if diff.num_days() < 7 {
        format!("{}d ago", diff.num_days())
    } else if diff.num_days() < 30 {
        format!("{}w ago", diff.num_days() / 7)
    } else {
        format!("{}mo ago", diff.num_days() / 30)
    }
}

fn strip_html_and_truncate(input: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max_chars {
        let mut truncated: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    } else {
        collapsed
    }
}

fn hash_id(source_id: u32, url: &str) -> u64 {
    let mut h = DefaultHasher::new();
    source_id.hash(&mut h);
    url.hash(&mut h);
    h.finish()
}
