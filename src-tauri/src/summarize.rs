use serde::Deserialize;
use std::time::Duration;

const MODEL: &str = "claude-opus-4-7";
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

pub struct Summary {
    pub summary: String,
    pub category: String,
}

#[derive(Deserialize)]
struct Completion {
    content: Vec<ContentBlock>,
}

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
}

/// `categories` must be non-empty — an empty JSON-schema `enum` is invalid and
/// the request would 400. Callers guard this before invoking.
pub async fn summarize(
    client: &reqwest::Client,
    api_key: &str,
    title: &str,
    source: &str,
    raw_excerpt: &str,
    categories: &[(String, String)],
    lens: Option<&str>,
) -> Result<Summary, String> {
    if categories.is_empty() {
        return Err("no categories configured".to_string());
    }

    let cat_list = categories
        .iter()
        .map(|(id, desc)| format!("- {id}: {desc}"))
        .collect::<Vec<_>>()
        .join("\n");

    let enum_values: Vec<String> = categories.iter().map(|(id, _)| id.clone()).collect();

    let lens_block = lens
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| format!("\n\nReader context (lean into what this person cares about):\n{s}"))
        .unwrap_or_default();

    let system_prompt = format!(
        "You summarize and categorize articles for a morning briefing. \
         The reader is scanning ~20 articles in under 2 minutes. \
         Given an article's source, title, and raw excerpt, return a JSON object:\n\
         - summary: 2-3 sentences, under 280 characters, in the voice of a journalist. \
         Do NOT restate the title. Lead with the most concrete detail.\n\
         - category: exactly one ID from:\n{cat_list}{lens_block}\n\n\
         Return only the JSON object."
    );

    let user = format!("Source: {source}\nTitle: {title}\n\nExcerpt:\n{raw_excerpt}");

    let body = serde_json::json!({
        "model": MODEL,
        "max_tokens": 1024,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user }],
        "output_config": {
            "format": {
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": {
                        "summary": { "type": "string" },
                        "category": {
                            "type": "string",
                            "enum": enum_values,
                        }
                    },
                    "required": ["summary", "category"],
                    "additionalProperties": false
                }
            }
        }
    });

    let resp = client
        .post(ENDPOINT)
        .header("x-api-key", api_key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .timeout(Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;

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

    let text = completion
        .content
        .into_iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        })
        .ok_or_else(|| "no text block in response".to_string())?;

    let parsed: Parsed =
        serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))?;

    Ok(Summary {
        summary: parsed.summary,
        category: parsed.category,
    })
}
