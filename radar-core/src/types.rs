//! Shared wire/domain types. Field names are camelCase on the wire to match
//! what earlier releases stored.

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

/// Per-field lenient deserialize: a wrong type, unknown enum string or null
/// falls back to the field default so one bad field never wipes the rest.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

fn lenient_true<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or(true))
}

fn lenient_time<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_else(|_| d_time()))
}

fn d_time() -> String {
    "07:30".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default, deserialize_with = "lenient")]
    pub theme: Theme,
    #[serde(default, deserialize_with = "lenient")]
    pub view_mode: ViewMode,
    #[serde(default, deserialize_with = "lenient")]
    pub density: Density,
    #[serde(default, deserialize_with = "lenient")]
    pub accent: Accent,
    #[serde(default = "d_true", deserialize_with = "lenient_true")]
    pub use_serif: bool,
    #[serde(default = "d_true", deserialize_with = "lenient_true")]
    pub show_accent: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub sidebar_collapsed: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub view: View,
    #[serde(default, deserialize_with = "lenient")]
    pub run_at_launch: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub schedule_enabled: bool,
    #[serde(default = "d_time", deserialize_with = "lenient_time")]
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
    #[serde(default, deserialize_with = "lenient")]
    pub name: String,
    #[serde(default, deserialize_with = "lenient")]
    pub role: String,
    #[serde(default, deserialize_with = "lenient")]
    pub lens: String,
}

/// Signed-in account (Clerk identity). Wire shape is camelCase.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub name: String,
    pub image_url: Option<String>,
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
    fn prefs_unknown_enum_falls_back_per_field() {
        let p: Prefs = serde_json::from_value(json!({
            "theme": "sepia", "viewMode": "list", "density": null,
            "useSerif": "yes", "scheduleTime": 5, "scheduleEnabled": true,
            "accent": "Rust", "view": "nope"
        }))
        .unwrap();
        assert_eq!(p.theme, Theme::Light);
        assert_eq!(p.view_mode, ViewMode::List);
        assert_eq!(p.density, Density::Cozy);
        assert!(p.use_serif);
        assert_eq!(p.schedule_time, "07:30");
        assert!(p.schedule_enabled);
        assert_eq!(p.accent, Accent::Rust);
        assert_eq!(p.view, View::Briefing);
    }

    #[test]
    fn profile_null_fields_default() {
        let p: Profile = serde_json::from_value(json!({
            "name": null, "role": "Designer", "lens": 7
        }))
        .unwrap();
        assert_eq!(p.name, "");
        assert_eq!(p.role, "Designer");
        assert_eq!(p.lens, "");
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
