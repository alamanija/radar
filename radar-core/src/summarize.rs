//! One Claude request per article: a short summary plus one of the user's
//! category ids, returned as structured output (raw HTTP; there is no
//! official Rust SDK).

use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::types::SummaryModel;

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Beta header for `fallbacks: "default"` (server-side refusal fallback).
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Room for the model's (low-effort) thinking plus the short JSON answer;
/// thinking counts toward this cap.
const MAX_TOKENS: u32 = 2048;
/// Longest `retry-after` honored before the single retry on 429/529.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(20);

pub struct Summary {
    pub summary: String,
    pub category: String,
    /// 1–5; `None` when the model returned a value outside that range.
    pub importance: Option<u8>,
}

#[derive(Deserialize)]
struct Completion {
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// Only `text` carries the answer; `thinking` (and `fallback`) blocks are
/// skipped.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Parsed {
    summary: String,
    category: String,
    #[serde(default)]
    importance: i64,
}

/// `categories` must be non-empty — an empty JSON-schema `enum` is invalid and
/// the request would 400. Callers guard this before invoking.
#[allow(clippy::too_many_arguments)]
pub async fn summarize(
    client: &reqwest::Client,
    api_key: &str,
    model: SummaryModel,
    title: &str,
    source: &str,
    raw_excerpt: &str,
    categories: &[(String, String)],
    lens: Option<&str>,
) -> Result<Summary, String> {
    summarize_at(
        ENDPOINT,
        client,
        api_key,
        model,
        title,
        source,
        raw_excerpt,
        categories,
        lens,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn summarize_at(
    endpoint: &str,
    client: &reqwest::Client,
    api_key: &str,
    model: SummaryModel,
    title: &str,
    source: &str,
    raw_excerpt: &str,
    categories: &[(String, String)],
    lens: Option<&str>,
) -> Result<Summary, String> {
    if categories.is_empty() {
        return Err("no categories configured".to_string());
    }
    let enum_values: Vec<String> = categories.iter().map(|(id, _)| id.clone()).collect();
    let user = format!("Source: {source}\nTitle: {title}\n\nExcerpt:\n{raw_excerpt}");
    let body = request_body(model, &system_prompt(categories, lens), &user, &enum_values);

    let resp = post_with_retry(client, endpoint, api_key, model, &body).await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("HTTP {status}: {snippet}"));
    }
    let completion: Completion = resp
        .json()
        .await
        .map_err(|e| format!("decode response: {e}"))?;
    parse_completion(completion)
}

fn system_prompt(categories: &[(String, String)], lens: Option<&str>) -> String {
    let cat_list = categories
        .iter()
        .map(|(id, desc)| format!("- {id}: {desc}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lens_block = lens
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| format!("\n\nReader context (lean into what this person cares about):\n{s}"))
        .unwrap_or_default();
    format!(
        "You summarize and categorize articles for a morning briefing. \
         The reader is scanning ~20 articles in under 2 minutes. \
         Given an article's source, title, and raw excerpt, return a JSON object:\n\
         - summary: 2-3 sentences, under 280 characters, in the voice of a journalist. \
         Do NOT restate the title. Lead with the most concrete detail.\n\
         - category: exactly one ID from:\n{cat_list}\n\
         - importance: integer 1-5, how much this reader would want to read it today. \
         5 = must-read, 3 = a typical item, 1 = skip. Judge against the Reader context \
         when given; otherwise as a designer following the field.{lens_block}\n\n\
         Return only the JSON object."
    )
}

/// The Messages request. `thinking` is left unset (adaptive on these
/// models) and effort is `low`: a two-sentence summary needs little
/// reasoning. Models with a server-side refusal fallback opt into it.
fn request_body(model: SummaryModel, system: &str, user: &str, enum_values: &[String]) -> Value {
    let mut body = json!({
        "model": model.id(),
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "output_config": {
            "effort": "low",
            "format": {
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": {
                        "summary": { "type": "string" },
                        "category": { "type": "string", "enum": enum_values },
                        "importance": { "type": "integer", "enum": [1, 2, 3, 4, 5] }
                    },
                    "required": ["summary", "category", "importance"],
                    "additionalProperties": false
                }
            }
        }
    });
    if model.server_fallback() {
        body["fallbacks"] = json!("default");
    }
    body
}

/// Sends `body`, retrying once on 429 (rate limited) or 529 (overloaded)
/// after the server's `retry-after` (capped), else two seconds.
async fn post_with_retry(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    model: SummaryModel,
    body: &Value,
) -> Result<reqwest::Response, String> {
    let send = || {
        let mut req = client
            .post(endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .timeout(Duration::from_secs(60))
            .json(body);
        if model.server_fallback() {
            req = req.header("anthropic-beta", FALLBACK_BETA);
        }
        req.send()
    };
    let resp = send().await.map_err(|e| format!("request: {e}"))?;
    let status = resp.status().as_u16();
    if status != 429 && status != 529 {
        return Ok(resp);
    }
    let wait = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(2))
        .min(MAX_RETRY_WAIT);
    tokio::time::sleep(wait).await;
    send().await.map_err(|e| format!("request: {e}"))
}

/// The answer, unless the model declined or ran out of tokens first.
fn parse_completion(completion: Completion) -> Result<Summary, String> {
    match completion.stop_reason.as_deref() {
        Some("refusal") => return Err("declined by Claude's safety filter".to_string()),
        Some("max_tokens") => return Err("response cut off (max_tokens)".to_string()),
        _ => {}
    }
    let text = completion
        .content
        .into_iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text),
            ContentBlock::Other => None,
        })
        .ok_or_else(|| "no text block in response".to_string())?;
    let parsed: Parsed = serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))?;
    Ok(Summary {
        summary: parsed.summary,
        category: parsed.category,
        importance: (1..=5)
            .contains(&parsed.importance)
            .then_some(parsed.importance as u8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn cats() -> Vec<(String, String)> {
        vec![("packaging".into(), "Packaging design".into())]
    }

    async fn run(server: &MockServer, model: SummaryModel) -> Result<Summary, String> {
        summarize_at(
            &server.url("/v1/messages"),
            &reqwest::Client::new(),
            "sk-test",
            model,
            "Oat milk cartons",
            "Dieline",
            "A studio rethinks the carton.",
            &cats(),
            None,
        )
        .await
    }

    #[test]
    fn body_uses_the_model_low_effort_and_the_category_enum() {
        let b = request_body(SummaryModel::Haiku55, "sys", "user", &["packaging".into()]);
        assert_eq!(b["model"], "claude-haiku-5-5");
        assert_eq!(b["max_tokens"], 2048);
        assert_eq!(b["output_config"]["effort"], "low");
        assert_eq!(
            b["output_config"]["format"]["schema"]["properties"]["category"]["enum"],
            json!(["packaging"])
        );
        assert!(b.get("thinking").is_none(), "thinking is left adaptive");
        assert!(b.get("temperature").is_none());
        // Haiku 5.5 has no server-side fallback.
        assert!(b.get("fallbacks").is_none());
    }

    #[test]
    fn sonnet_and_opus_opt_into_the_server_fallback() {
        for m in [SummaryModel::Sonnet55, SummaryModel::Opus55] {
            let b = request_body(m, "sys", "user", &["packaging".into()]);
            assert_eq!(b["model"], m.id());
            assert_eq!(b["fallbacks"], "default");
        }
    }

    #[test]
    fn system_prompt_lists_categories_and_the_lens() {
        let p = system_prompt(&cats(), Some("  brand systems  "));
        assert!(p.contains("- packaging: Packaging design"));
        assert!(p.contains("Reader context (lean into"));
        assert!(p.contains("brand systems"));
        assert!(!system_prompt(&cats(), Some("  ")).contains("Reader context (lean into"));
    }

    #[test]
    fn body_requires_an_importance_score() {
        let b = request_body(SummaryModel::Haiku55, "sys", "user", &["packaging".into()]);
        let schema = &b["output_config"]["format"]["schema"];
        assert_eq!(
            schema["properties"]["importance"]["enum"],
            json!([1, 2, 3, 4, 5])
        );
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("importance")));
    }

    #[test]
    fn system_prompt_explains_importance() {
        let p = system_prompt(&cats(), None);
        assert!(p.contains("importance"));
        assert!(p.contains("5 = must-read"));
    }

    #[tokio::test]
    async fn out_of_range_importance_keeps_the_summary() {
        for (score, want) in [(9, None), (0, None), (4, Some(4))] {
            let server = MockServer::start_async().await;
            let text =
                format!("{{\"summary\":\"S\",\"category\":\"packaging\",\"importance\":{score}}}");
            server
                .mock_async(|when, then| {
                    when.method(POST).path("/v1/messages");
                    then.status(200).json_body(json!({
                        "content": [{"type": "text", "text": text}],
                        "stop_reason": "end_turn"
                    }));
                })
                .await;
            let s = run(&server, SummaryModel::Haiku55).await.unwrap();
            assert_eq!(s.summary, "S");
            assert_eq!(s.importance, want, "score {score}");
        }
    }

    #[tokio::test]
    async fn reads_the_text_block_after_thinking() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/messages")
                    .header("x-api-key", "sk-test")
                    .header_missing("anthropic-beta");
                then.status(200).json_body(json!({
                    "content": [
                        {"type": "thinking", "thinking": "", "signature": "s"},
                        {"type": "text", "text": "{\"summary\":\"Short.\",\"category\":\"packaging\"}"}
                    ],
                    "stop_reason": "end_turn"
                }));
            })
            .await;
        let s = run(&server, SummaryModel::Haiku55).await.unwrap();
        assert_eq!(s.summary, "Short.");
        assert_eq!(s.category, "packaging");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fallback_models_send_the_beta_header() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/messages")
                    .header("anthropic-beta", FALLBACK_BETA);
                then.status(200).json_body(json!({
                    "content": [{"type": "text", "text": "{\"summary\":\"S\",\"category\":\"packaging\"}"}],
                    "stop_reason": "end_turn"
                }));
            })
            .await;
        run(&server, SummaryModel::Opus55).await.unwrap();
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn refusal_and_truncation_are_errors() {
        for (reason, needle) in [("refusal", "safety"), ("max_tokens", "max_tokens")] {
            let server = MockServer::start_async().await;
            server
                .mock_async(|when, then| {
                    when.method(POST).path("/v1/messages");
                    then.status(200).json_body(json!({
                        "content": [{"type": "text", "text": "{\"summary\":\"S\""}],
                        "stop_reason": reason
                    }));
                })
                .await;
            let err = run(&server, SummaryModel::Haiku55).await.err().unwrap();
            assert!(err.contains(needle), "{reason}: {err}");
        }
    }

    #[tokio::test]
    async fn rate_limit_is_retried_once() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/v1/messages");
                then.status(429)
                    .header("retry-after", "0")
                    .body("{\"type\":\"error\"}");
            })
            .await;
        let err = run(&server, SummaryModel::Haiku55).await.err().unwrap();
        assert!(err.contains("429"), "{err}");
        assert_eq!(mock.calls_async().await, 2);
    }
}
