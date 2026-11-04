//! Shared wire/domain types. Field names are camelCase on the wire to match
//! what the old JS frontend stored.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
    pub image_url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceError {
    pub source_id: u32,
    pub source_name: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BriefingResponse {
    pub articles: Vec<Article>,
    pub errors: Vec<SourceError>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SourceHealth {
    #[default]
    Ok,
    Warn,
    Stale,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: u32,
    pub name: String,
    pub category: String,
    pub feed_url: Option<String>,
    pub enabled: bool,
    pub is_default: bool,
    pub last_fetch_at: Option<i64>,
    #[serde(default)]
    pub health: SourceHealth,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub label: String,
    pub description: String,
    pub accent: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Archive {
    pub id: i64,
    /// RFC3339 timestamp.
    pub run_at: String,
    pub articles: Vec<Article>,
    pub errors: Vec<SourceError>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Cozy,
    Compact,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Camel,
    Olive,
    Rust,
    Slate,
    Ink,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    Briefing,
    Saved,
    Archive,
    Sources,
    Categories,
    Settings,
}

fn d_true() -> bool {
    true
}

fn d_time() -> String {
    "07:30".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub view_mode: ViewMode,
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub accent: Accent,
    #[serde(default = "d_true")]
    pub use_serif: bool,
    #[serde(default = "d_true")]
    pub show_accent: bool,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub view: View,
    #[serde(default)]
    pub run_at_launch: bool,
    #[serde(default)]
    pub schedule_enabled: bool,
    #[serde(default = "d_time")]
    pub schedule_time: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            view_mode: ViewMode::default(),
            density: Density::default(),
            accent: Accent::default(),
            use_serif: true,
            show_accent: true,
            sidebar_collapsed: false,
            view: View::default(),
            run_at_launch: false,
            schedule_enabled: false,
            schedule_time: d_time(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub lens: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn article_roundtrips_camel_case() {
        let v = json!({
            "id": 12345678901234u64,
            "title": "A title",
            "source": "Dieline",
            "sourceId": 3,
            "category": "packaging",
            "categoryLabel": "Packaging",
            "summary": "Short.",
            "published": "2h ago",
            "publishedAt": "2026-05-01T10:00:00Z",
            "read": false,
            "bookmarked": true,
            "accent": "camel",
            "url": "https://example.com/a",
            "imageUrl": "https://example.com/a.png"
        });
        let a: Article = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(a.source_id, 3);
        assert_eq!(a.category_label, "Packaging");
        assert_eq!(serde_json::to_value(&a).unwrap(), v);
    }

    #[test]
    fn prefs_defaults_fill_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.schedule_time, "07:30");
        assert_eq!(p.accent, Accent::Camel);
        assert!(p.use_serif);
        assert_eq!(p, Prefs::default());
    }

    #[test]
    fn prefs_enums_use_js_strings() {
        let p = Prefs {
            theme: Theme::Dark,
            view_mode: ViewMode::List,
            density: Density::Compact,
            accent: Accent::Slate,
            view: View::Categories,
            ..Prefs::default()
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["viewMode"], "list");
        assert_eq!(v["density"], "compact");
        assert_eq!(v["accent"], "Slate");
        assert_eq!(v["view"], "categories");
        assert_eq!(v["scheduleTime"], "07:30");
    }

    #[test]
    fn source_without_health_defaults_ok() {
        let s: Source = serde_json::from_value(json!({
            "id": 1, "name": "N", "category": "tools", "feedUrl": null,
            "enabled": true, "isDefault": false, "lastFetchAt": null
        }))
        .unwrap();
        assert_eq!(s.health, SourceHealth::Ok);
        let warn: SourceHealth = serde_json::from_value(json!("warn")).unwrap();
        assert_eq!(warn, SourceHealth::Warn);
    }
}
