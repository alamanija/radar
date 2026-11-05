use chrono::{DateTime, Utc};
use feed_rs::parser;
use futures::future::join_all;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use crate::keychain;
use crate::summarize;
use crate::types::{Article, BriefingResponse, SourceError, SummaryModel};

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

/// A summary an earlier briefing got from Claude, reused for the same
/// article id instead of asking again.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownSummary {
    pub summary: String,
    /// Category id Claude picked.
    pub category: String,
    /// Model id that wrote it.
    pub model: String,
    /// Its 1–5 importance score, when it has one.
    pub importance: Option<u8>,
}

/// Everything one briefing needs.
#[derive(Debug, Clone, Default)]
pub struct BriefingRequest {
    pub sources: Vec<SourceInput>,
    pub categories: Vec<CategoryInput>,
    /// The reader's lens, injected into the system prompt when set.
    pub lens: Option<String>,
    pub model: SummaryModel,
    /// Earlier summaries by article id.
    pub known: HashMap<u64, KnownSummary>,
    /// Leave out articles published more than this many days ago; `0` keeps
    /// all.
    pub stale_after_days: u32,
}

/// Claude requests in flight at once during a briefing.
const SUMMARY_CONCURRENCY: usize = 6;

/// Fetch every source with a feed URL, drop stale articles, reuse known
/// summaries, summarize the rest via Claude (with an API key and at least
/// one category), and return the briefing newest first.
pub async fn run_briefing(req: BriefingRequest) -> Result<BriefingResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent("Radar/0.1 (desktop news briefing)")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let cat_map: Arc<HashMap<String, CategoryInput>> = Arc::new(
        req.categories
            .iter()
            .map(|c| (c.id.clone(), c.clone()))
            .collect(),
    );

    let targets: Vec<SourceInput> = req
        .sources
        .into_iter()
        .filter(|s| s.feed_url.as_deref().is_some_and(|u| !u.is_empty()))
        .collect();

    let fetches = targets.into_iter().map(|src| {
        let client = client.clone();
        let cats = Arc::clone(&cat_map);
        async move { fetch_source(&client, src, &cats).await }
    });
    let results = join_all(fetches).await;

    let mut fetched: Vec<Article> = Vec::new();
    let mut errors: Vec<SourceError> = Vec::new();
    for result in results {
        match result {
            Ok(mut arts) => fetched.append(&mut arts),
            Err(e) => errors.push(e),
        }
    }

    let fresh = drop_stale(fetched, req.stale_after_days, Utc::now());
    let (mut articles, todo) = split_known(fresh, &req.known, &cat_map);

    // Claude pass: requires an API key AND at least one category (the output
    // schema's enum can't be empty). The keychain is only read when there
    // is something to summarize.
    let api_key = (!req.categories.is_empty() && !todo.is_empty())
        .then(keychain::read_api_key)
        .flatten();
    match api_key {
        None => articles.extend(todo),
        Some(api_key) => {
            let claude_cats: Arc<Vec<(String, String)>> = Arc::new(
                req.categories
                    .iter()
                    .map(|c| (c.id.clone(), c.description.clone()))
                    .collect(),
            );
            let lens = Arc::new(req.lens);
            let model = req.model;
            let enhanced: Vec<_> = stream::iter(todo)
                .map(|article| {
                    let client = client.clone();
                    let api_key = api_key.clone();
                    let cats = Arc::clone(&cat_map);
                    let claude_cats = Arc::clone(&claude_cats);
                    let lens = Arc::clone(&lens);
                    async move {
                        enhance(
                            &client,
                            &api_key,
                            model,
                            article,
                            &cats,
                            &claude_cats,
                            lens.as_deref(),
                        )
                        .await
                    }
                })
                .buffer_unordered(SUMMARY_CONCURRENCY)
                .collect()
                .await;

            let mut claude_failures = 0usize;
            let mut first_claude_error: Option<String> = None;
            for result in enhanced {
                match result {
                    Ok(a) => articles.push(a),
                    Err(boxed) => {
                        let (a, msg) = *boxed;
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

/// Articles published within `days` of `now`; `0` keeps all, and an
/// article without a date is always kept.
fn drop_stale(articles: Vec<Article>, days: u32, now: DateTime<Utc>) -> Vec<Article> {
    if days == 0 {
        return articles;
    }
    let cutoff = now - chrono::Duration::days(i64::from(days));
    articles
        .into_iter()
        .filter(|a| a.published_at.is_none_or(|t| t >= cutoff))
        .collect()
}

/// Splits fetched articles into those a known summary covers (filled in,
/// label and accent from the category as it is now) and those to send to
/// Claude. A known summary whose category no longer exists doesn't count.
fn split_known(
    articles: Vec<Article>,
    known: &HashMap<u64, KnownSummary>,
    cats: &HashMap<String, CategoryInput>,
) -> (Vec<Article>, Vec<Article>) {
    let mut reused = Vec::new();
    let mut todo = Vec::new();
    for mut a in articles {
        match known.get(&a.id).filter(|k| cats.contains_key(&k.category)) {
            Some(k) => {
                let (cat, label, accent) = resolve(cats, &k.category);
                a.summary = k.summary.clone();
                a.category = cat;
                a.category_label = label;
                a.accent = accent;
                a.summarized_by = Some(k.model.clone());
                a.importance = k.importance;
                reused.push(a);
            }
            None => todo.push(a),
        }
    }
    (reused, todo)
}

async fn enhance(
    client: &reqwest::Client,
    api_key: &str,
    model: SummaryModel,
    mut article: Article,
    cats: &HashMap<String, CategoryInput>,
    claude_cats: &[(String, String)],
    lens: Option<&str>,
) -> Result<Article, Box<(Article, String)>> {
    match summarize::summarize(
        client,
        api_key,
        model,
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
            article.summarized_by = Some(model.id().to_string());
            article.importance = s.importance;
            Ok(article)
        }
        Err(msg) => Err(Box::new((article, msg))),
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
        .take(30)
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

            let image_url = extract_image(entry, &raw_summary);

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
                image_url,
                summarized_by: None,
                importance: None,
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
        let mut truncated: String = collapsed
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect();
        truncated.push('…');
        truncated
    } else {
        collapsed
    }
}

/// Best-effort cover image from a feed entry, in this order:
///   1. `media:thumbnail` (feed-rs `entry.media[*].thumbnails`)
///   2. `media:content` / `<enclosure>` with an `image/*` content-type
///   3. First `<img src="...">` in the entry's HTML body or summary
fn extract_image(entry: &feed_rs::model::Entry, raw_html: &str) -> Option<String> {
    for m in &entry.media {
        if let Some(t) = m.thumbnails.first() {
            return Some(t.image.uri.clone());
        }
    }
    for m in &entry.media {
        for c in &m.content {
            let is_image = c
                .content_type
                .as_ref()
                .map(|t| t.ty().as_str().starts_with("image"))
                .unwrap_or(false);
            if is_image {
                if let Some(url) = &c.url {
                    return Some(url.to_string());
                }
            }
            // Some feeds set url without content_type — accept if extension looks image-y.
            if c.content_type.is_none() {
                if let Some(url) = &c.url {
                    let s = url.as_str();
                    let lower = s.to_ascii_lowercase();
                    if lower.ends_with(".jpg")
                        || lower.ends_with(".jpeg")
                        || lower.ends_with(".png")
                        || lower.ends_with(".webp")
                        || lower.ends_with(".gif")
                        || lower.ends_with(".avif")
                    {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    let body = entry
        .content
        .as_ref()
        .and_then(|c| c.body.as_deref())
        .unwrap_or("");
    first_img_src(body).or_else(|| first_img_src(raw_html))
}

/// Scan an HTML fragment for the first `<img ... src="...">` and return the
/// src value. Tolerant of single quotes and whitespace; ignores data: URLs
/// (tracking pixels, inline placeholders).
fn first_img_src(html: &str) -> Option<String> {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1].eq_ignore_ascii_case(&b'i')
            && bytes[i + 2].eq_ignore_ascii_case(&b'm')
            && bytes[i + 3].eq_ignore_ascii_case(&b'g')
            && bytes[i + 4].is_ascii_whitespace()
        {
            let Some(rel) = html[i..].find('>') else {
                break;
            };
            let tag = &html[i..i + rel];
            if let Some(src) = attr_value(tag, "src") {
                if !src.starts_with("data:") {
                    return Some(src);
                }
            }
            i += rel + 1;
            continue;
        }
        i += 1;
    }
    None
}

/// Pull `name="value"` or `name='value'` out of an HTML tag fragment,
/// case-insensitive on the attribute name.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = name.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(idx) = lower[search_from..].find(&key) {
        let start = search_from + idx;
        // must be preceded by whitespace or `<img` boundary
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let after = start + key.len();
        let rest = &tag[after..];
        let rest_trimmed = rest.trim_start();
        if before_ok && rest_trimmed.starts_with('=') {
            let after_eq = rest_trimmed[1..].trim_start();
            let quote = after_eq.as_bytes().first().copied();
            if quote == Some(b'"') || quote == Some(b'\'') {
                let q = quote.unwrap() as char;
                let after_q = &after_eq[1..];
                if let Some(end) = after_q.find(q) {
                    return Some(after_q[..end].to_string());
                }
            }
        }
        search_from = start + key.len();
    }
    None
}

/// Article id: the hash of `(source_id, url)`, in the form earlier releases
/// stored it, so ids from existing installs (read/bookmark state, archives,
/// article-state rows on the server) keep matching.
fn hash_id(source_id: u32, url: &str) -> u64 {
    let mut h = DefaultHasher::new();
    source_id.hash(&mut h);
    url.hash(&mut h);
    js_number(h.finish())
}

/// `n` as earlier releases kept it, a JavaScript number round-trip: the
/// nearest f64, written back in its shortest decimal form (`JSON.stringify` /
/// `String`), which Rust's f64 `Display` reproduces. Above 2^53 that is a
/// rounded value. One that rounds up to 2^64 doesn't fit a u64 and is kept
/// as is.
fn js_number(n: u64) -> u64 {
    (n as f64).to_string().parse::<u64>().unwrap_or(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_and_collapses_whitespace() {
        assert_eq!(strip_html_and_truncate("<p>a  <b>b</b></p>", 280), "a b");
    }

    #[test]
    fn truncates_with_ellipsis() {
        let out = strip_html_and_truncate(&"x".repeat(400), 280);
        assert_eq!(out.chars().count(), 280);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn first_img_src_skips_data_urls() {
        let html =
            r#"<img src="data:image/gif;base64,AAA"><img alt="x" src='https://e.com/a.png'>"#;
        assert_eq!(first_img_src(html).as_deref(), Some("https://e.com/a.png"));
    }

    /// The ids earlier releases stored. The full 64-bit hash went out as a
    /// JSON number; JavaScript kept the nearest f64 and wrote that back
    /// (`JSON.stringify` into `radar.json`, `String(id)` in article-state
    /// PATCHes). Expected values computed with node 22:
    /// `String(JSON.parse("13612357342556405454"))`.
    #[test]
    fn article_ids_are_the_js_rounded_ids() {
        assert_eq!(
            hash_id(1, "https://thedieline.com/oat-milk-cartons"),
            13_612_357_342_556_406_000
        );
        assert_eq!(
            hash_id(1, "https://thedieline.com/carton-lab"),
            15_651_859_121_617_930_000
        );
        assert_eq!(
            js_number(7_565_627_446_702_248_511),
            7_565_627_446_702_249_000
        );
        assert_eq!(js_number(9_007_199_254_740_993), 9_007_199_254_740_992);
        assert_eq!(js_number(42), 42);
        // Rounds up to 2^64, which JavaScript prints but a u64 can't hold: kept.
        assert_eq!(js_number(u64::MAX), u64::MAX);
    }

    const DIELINE_FEED: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>Dieline</title><link>https://thedieline.com</link><description>Packaging</description>
<item><title>A fresh take on oat milk cartons</title><link>https://thedieline.com/oat-milk-cartons</link><description>A studio rethinks the carton.</description></item>
<item><title>Inside a carton lab</title><link>https://thedieline.com/carton-lab</link><description>A visit.</description></item>
</channel></rss>"#;

    /// An install upgraded from an earlier release (the fixture holds what it
    /// wrote): the next briefing yields the same `(source_id, url)` articles,
    /// and both kinds of stored state find them again: the flags in
    /// `radar.json`, and the article-state row it queued (served as the
    /// server holds it once drained). New writes dedup against the legacy ones.
    #[tokio::test]
    async fn legacy_state_overlays_after_a_briefing() {
        use crate::model::RadarModel;
        use crate::sync::article_states;
        use crate::sync::queue::{Queue, QueuedReq};
        use crate::sync::resource::SyncedResource;
        use crate::sync::resources::PrefsRes;
        use httpmock::prelude::*;
        use serde_json::{json, Value};
        use std::collections::HashSet;

        let snap = crate::store::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy_store.json"),
        );
        let queued: Vec<QueuedReq> =
            serde_json::from_value(Value::Array(snap.sync_queue.clone())).unwrap();
        let row = queued
            .iter()
            .find(|q| q.method == "PATCH")
            .unwrap()
            .body
            .clone();

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/feed");
                then.status(200).body(DIELINE_FEED);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path(article_states::ENDPOINT);
                then.status(200).json_body(json!({ "states": [row] }));
            })
            .await;
        let source = SourceInput {
            id: 1,
            name: "Dieline".into(),
            category: "packaging".into(),
            feed_url: Some(server.url("/feed")),
        };
        // No categories: no Claude pass, so the keychain is never read.
        let resp = run_briefing(BriefingRequest {
            sources: vec![source],
            ..BriefingRequest::default()
        })
        .await
        .unwrap();
        assert_eq!(resp.articles.len(), 2);
        let by_url = |arts: &[Article], tail: &str| {
            arts.iter()
                .find(|a| a.url.ends_with(tail))
                .cloned()
                .unwrap()
        };

        // The flags in radar.json.
        let mut m = RadarModel::from_snapshot(snap);
        m.begin_briefing();
        let out = m.finish_briefing(resp.clone(), &HashSet::from([1]), 1_767_300_000_000);
        // The stored archive run has the oat-milk article under the id the
        // new briefing derives, so only carton-lab (never archived) is new.
        assert_eq!(out.new_count, 1, "only the article the stored run lacks");
        let oat = by_url(&m.articles, "/oat-milk-cartons");
        assert!(
            !m.new_ids().contains(&oat.id),
            "the stored run's id matches"
        );
        let saved: Vec<&str> = m.saved().iter().map(|a| a.url.as_str()).collect();
        assert_eq!(saved, ["https://thedieline.com/oat-milk-cartons"]);
        assert!(by_url(&m.articles, "/carton-lab").read);

        // The article-state row, keyed by the legacy `String(id)`.
        let c = crate::sync::test_support::client(&server.base_url());
        let states = article_states::pull(&c).await.unwrap();
        let mut fresh = resp.articles.clone();
        assert!(!by_url(&fresh, "/carton-lab").read);
        article_states::overlay(&mut fresh, &states);
        assert!(
            by_url(&fresh, "/carton-lab").read,
            "the row finds the new id"
        );

        // A new write for the same article or resource replaces the legacy one.
        let lab = by_url(&m.articles, "/carton-lab");
        let mut q = Queue::new(queued);
        q.enqueue(QueuedReq::new(
            "PATCH",
            article_states::ENDPOINT,
            json!({}),
            Some(article_states::group_key(&lab.id.to_string())),
        ));
        q.enqueue(QueuedReq::new(
            "PUT",
            "/sync/prefs",
            json!({}),
            Some(SyncedResource::<PrefsRes>::group_key()),
        ));
        assert_eq!(q.items.len(), 2, "both older queue items were superseded");
        assert!(q.items.iter().all(|i| i.account_id.is_none()));
    }

    #[test]
    fn resolve_unknown_falls_back() {
        let cats = HashMap::new();
        assert_eq!(
            resolve(&cats, "nope"),
            ("uncategorized".into(), "Uncategorized".into(), "ink".into())
        );
    }

    fn cat(id: &str, label: &str, accent: &str) -> CategoryInput {
        CategoryInput {
            id: id.into(),
            label: label.into(),
            description: format!("{label} news"),
            accent: accent.into(),
        }
    }

    fn cat_map(cats: &[CategoryInput]) -> HashMap<String, CategoryInput> {
        cats.iter().map(|c| (c.id.clone(), c.clone())).collect()
    }

    fn raw(id: u64, published_at: Option<DateTime<Utc>>) -> Article {
        Article {
            id,
            title: format!("T{id}"),
            source: "S".into(),
            source_id: 1,
            category: "packaging".into(),
            category_label: "Packaging".into(),
            summary: format!("raw excerpt {id}"),
            published: "recent".into(),
            published_at,
            read: false,
            bookmarked: false,
            accent: "camel".into(),
            url: format!("https://e.com/{id}"),
            image_url: None,
            summarized_by: None,
            importance: None,
        }
    }

    fn known(summary: &str, category: &str) -> KnownSummary {
        KnownSummary {
            summary: summary.into(),
            category: category.into(),
            model: "claude-haiku-5-5".into(),
            importance: None,
        }
    }

    #[test]
    fn known_summaries_are_reused_and_the_rest_go_to_claude() {
        let cats = cat_map(&[
            cat("packaging", "Packaging", "camel"),
            // Renamed since the summary was written: the new label wins.
            cat("branding", "Brand & Identity", "olive"),
        ]);
        let known = HashMap::from([
            (
                1,
                KnownSummary {
                    importance: Some(5),
                    ..known("Cached one.", "branding")
                },
            ),
            // Its category was deleted: summarize again.
            (2, known("Cached two.", "gone")),
        ]);
        let (reused, todo) = split_known(
            vec![raw(1, None), raw(2, None), raw(3, None)],
            &known,
            &cats,
        );

        assert_eq!(reused.len(), 1);
        let a = &reused[0];
        assert_eq!(a.id, 1);
        assert_eq!(a.summary, "Cached one.");
        assert_eq!(a.category, "branding");
        assert_eq!(a.category_label, "Brand & Identity");
        assert_eq!(a.accent, "olive");
        assert_eq!(a.summarized_by.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(a.importance, Some(5), "the score is reused too");

        let ids: Vec<u64> = todo.iter().map(|a| a.id).collect();
        assert_eq!(ids, [2, 3]);
        assert!(todo.iter().all(|a| a.summarized_by.is_none()));
        assert_eq!(todo[0].summary, "raw excerpt 2", "the raw excerpt is kept");
    }

    #[test]
    fn stale_articles_are_dropped_when_a_limit_is_set() {
        let now = DateTime::parse_from_rfc3339("2026-11-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let days_ago = |d: i64| Some(now - chrono::Duration::days(d));
        let arts = || {
            vec![
                raw(1, days_ago(1)),
                raw(2, days_ago(7)),
                raw(3, days_ago(8)),
                // No date: kept, there's nothing to judge it by.
                raw(4, None),
            ]
        };
        let ids = |v: Vec<Article>| v.iter().map(|a| a.id).collect::<Vec<_>>();

        assert_eq!(ids(drop_stale(arts(), 0, now)), [1, 2, 3, 4], "0 keeps all");
        assert_eq!(ids(drop_stale(arts(), 7, now)), [1, 2, 4]);
        assert_eq!(ids(drop_stale(arts(), 3, now)), [1, 4]);
    }

    /// Every fetched article already has a summary: nothing is sent to
    /// Claude, so the keychain is never read, and the cached text comes back.
    #[tokio::test]
    async fn fully_cached_briefing_makes_no_claude_requests() {
        use httpmock::prelude::*;
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/feed");
                then.status(200).body(DIELINE_FEED);
            })
            .await;
        let ids = [
            hash_id(1, "https://thedieline.com/oat-milk-cartons"),
            hash_id(1, "https://thedieline.com/carton-lab"),
        ];
        let resp = run_briefing(BriefingRequest {
            sources: vec![SourceInput {
                id: 1,
                name: "Dieline".into(),
                category: "packaging".into(),
                feed_url: Some(server.url("/feed")),
            }],
            categories: vec![cat("packaging", "Packaging", "camel")],
            known: ids
                .iter()
                .map(|&id| (id, known("Cached.", "packaging")))
                .collect(),
            ..BriefingRequest::default()
        })
        .await
        .unwrap();
        assert_eq!(resp.articles.len(), 2);
        assert!(resp.articles.iter().all(|a| a.summary == "Cached."));
        assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    }
}
