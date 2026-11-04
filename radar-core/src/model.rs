//! UI-free application state and its mutations. Ports of the state logic in
//! `src/App.jsx`. No IO and no clocks: time is always a parameter.

use crate::ingest::{CategoryInput, SourceInput};
use crate::store::Snapshot;
use crate::types::{
    Account, Archive, Article, BriefingResponse, Category, Prefs, Profile, Source, SourceError,
    SourceHealth,
};
use chrono::{DateTime, SecondsFormat};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const ARCHIVE_CAP: usize = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArticleState {
    pub read: bool,
    pub bookmarked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SourcePatch {
    pub name: Option<String>,
    pub category: Option<String>,
    pub feed_url: Option<Option<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct CategoryPatch {
    pub label: Option<String>,
    pub description: Option<String>,
    pub accent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingOutcome {
    pub new_count: usize,
    pub total: usize,
    pub source_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RadarModel {
    pub prefs: Prefs,
    pub profile: Profile,
    pub sources: Vec<Source>,
    pub categories: Vec<Category>,
    pub articles: Vec<Article>,
    pub archives: Vec<Archive>,
    pub briefing_errors: Vec<SourceError>,
    pub scanning: bool,
    pub active_category: String,
    /// Keyed by `article.id.to_string()`.
    pub article_states: HashMap<String, ArticleState>,
    /// Article ids present when the current briefing began (for `new_count`).
    prev_ids: Option<HashSet<u64>>,
}

fn slugify_base(label: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in label.trim().to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out
}

impl RadarModel {
    pub fn from_snapshot(s: Snapshot) -> Self {
        let mut m = Self {
            prefs: s.prefs,
            profile: s.profile,
            sources: s.sources,
            categories: s.categories,
            articles: s.articles,
            archives: s.archives,
            active_category: "all".into(),
            ..Default::default()
        };
        // Seed the state map from persisted per-article flags.
        for a in &m.articles {
            if a.read || a.bookmarked {
                m.article_states.insert(
                    a.id.to_string(),
                    ArticleState {
                        read: a.read,
                        bookmarked: a.bookmarked,
                    },
                );
            }
        }
        m
    }

    pub fn to_snapshot(&self, sync_queue: Vec<Value>) -> Snapshot {
        Snapshot {
            prefs: self.prefs.clone(),
            sources: self.sources.clone(),
            articles: self.articles.clone(),
            archives: self.archives.clone(),
            categories: self.categories.clone(),
            profile: self.profile.clone(),
            sync_queue,
        }
    }

    // ---- sources ----

    pub fn add_source(&mut self, name: String, category: String, feed_url: Option<String>) -> u32 {
        let id = self.sources.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        self.sources.push(Source {
            id,
            name,
            category,
            feed_url: feed_url.filter(|u| !u.is_empty()),
            enabled: true,
            is_default: false,
            last_fetch_at: None,
            health: SourceHealth::Ok,
        });
        id
    }

    pub fn update_source(&mut self, id: u32, p: SourcePatch) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
            if let Some(v) = p.name {
                s.name = v;
            }
            if let Some(v) = p.category {
                s.category = v;
            }
            if let Some(v) = p.feed_url {
                s.feed_url = v;
            }
            if let Some(v) = p.enabled {
                s.enabled = v;
            }
        }
    }

    pub fn delete_source(&mut self, id: u32) -> Result<(), &'static str> {
        if self.sources.iter().any(|s| s.id == id && s.is_default) {
            return Err("default sources cannot be deleted");
        }
        self.sources.retain(|s| s.id != id);
        Ok(())
    }

    pub fn toggle_source(&mut self, id: u32) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
            s.enabled = !s.enabled;
        }
    }

    // ---- categories ----

    fn slugify(&self, label: &str) -> String {
        let base = slugify_base(label);
        if base.is_empty() {
            return "category".into();
        }
        let exists = |id: &str| self.categories.iter().any(|c| c.id == id);
        if !exists(&base) {
            return base;
        }
        let mut n = 2;
        while exists(&format!("{base}-{n}")) {
            n += 1;
        }
        format!("{base}-{n}")
    }

    pub fn add_category(&mut self, label: String, description: String, accent: String) -> String {
        let id = self.slugify(&label);
        self.categories.push(Category {
            id: id.clone(),
            label: label.trim().to_string(),
            description: description.trim().to_string(),
            accent,
        });
        id
    }

    pub fn update_category(&mut self, id: &str, p: CategoryPatch) {
        if let Some(c) = self.categories.iter_mut().find(|c| c.id == id) {
            if let Some(v) = p.label {
                c.label = v;
            }
            if let Some(v) = p.description {
                c.description = v;
            }
            if let Some(v) = p.accent {
                c.accent = v;
            }
        }
    }

    /// Err carries the number of sources still using the category.
    pub fn delete_category(&mut self, id: &str) -> Result<(), usize> {
        let in_use = self.sources.iter().filter(|s| s.category == id).count();
        if in_use > 0 {
            return Err(in_use);
        }
        self.categories.retain(|c| c.id != id);
        Ok(())
    }

    // ---- article state ----

    pub fn apply_article_states(&mut self) {
        for a in &mut self.articles {
            if let Some(s) = self.article_states.get(&a.id.to_string()) {
                a.read = s.read;
                a.bookmarked = s.bookmarked;
            }
        }
    }

    fn toggle(
        &mut self,
        id: u64,
        f: impl FnOnce(&Article) -> ArticleState,
    ) -> Option<(String, ArticleState)> {
        let a = self.articles.iter_mut().find(|a| a.id == id)?;
        let next = f(a);
        a.read = next.read;
        a.bookmarked = next.bookmarked;
        let key = id.to_string();
        self.article_states.insert(key.clone(), next);
        Some((key, next))
    }

    pub fn toggle_read(&mut self, id: u64) -> Option<(String, ArticleState)> {
        self.toggle(id, |a| ArticleState {
            read: !a.read,
            bookmarked: a.bookmarked,
        })
    }

    pub fn toggle_bookmark(&mut self, id: u64) -> Option<(String, ArticleState)> {
        self.toggle(id, |a| ArticleState {
            read: a.read,
            bookmarked: !a.bookmarked,
        })
    }

    // ---- briefing ----

    pub fn begin_briefing(&mut self) {
        self.prev_ids = Some(self.articles.iter().map(|a| a.id).collect());
        self.scanning = true;
        self.articles.clear();
        self.briefing_errors.clear();
    }

    pub fn finish_briefing(
        &mut self,
        resp: BriefingResponse,
        attempted: &HashSet<u32>,
        now_ms: i64,
    ) -> BriefingOutcome {
        let prev = self
            .prev_ids
            .take()
            .unwrap_or_else(|| self.articles.iter().map(|a| a.id).collect());
        let new_count = resp
            .articles
            .iter()
            .filter(|a| !prev.contains(&a.id))
            .count();
        let total = resp.articles.len();
        let source_count = resp
            .articles
            .iter()
            .map(|a| a.source.as_str())
            .collect::<HashSet<_>>()
            .len();

        if !resp.articles.is_empty() {
            let run_at = DateTime::from_timestamp_millis(now_ms)
                .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
                .unwrap_or_default();
            self.archives.insert(
                0,
                Archive {
                    id: now_ms,
                    run_at,
                    articles: resp.articles.clone(),
                    errors: resp.errors.clone(),
                },
            );
            self.archives.truncate(ARCHIVE_CAP);
        }

        let failed: HashSet<u32> = resp
            .errors
            .iter()
            .map(|e| e.source_id)
            .filter(|&id| id != 0)
            .collect();
        for s in &mut self.sources {
            if !attempted.contains(&s.id) {
                continue;
            }
            if failed.contains(&s.id) {
                s.health = SourceHealth::Warn;
            } else {
                s.health = SourceHealth::Ok;
                s.last_fetch_at = Some(now_ms);
            }
        }

        self.articles = resp.articles;
        self.briefing_errors = resp.errors;
        self.apply_article_states();
        self.scanning = false;
        BriefingOutcome {
            new_count,
            total,
            source_count,
        }
    }

    pub fn fail_briefing(&mut self, msg: &str) {
        self.briefing_errors = vec![SourceError {
            source_id: 0,
            source_name: "radar".into(),
            message: msg.to_string(),
        }];
        self.prev_ids = None;
        self.scanning = false;
    }

    pub fn briefing_inputs(
        &self,
    ) -> (
        Vec<SourceInput>,
        Vec<CategoryInput>,
        Option<String>,
        HashSet<u32>,
    ) {
        let enabled: Vec<&Source> = self
            .sources
            .iter()
            .filter(|s| s.enabled && s.feed_url.as_deref().is_some_and(|u| !u.is_empty()))
            .collect();
        let sources = enabled
            .iter()
            .map(|s| SourceInput {
                id: s.id,
                name: s.name.clone(),
                category: s.category.clone(),
                feed_url: s.feed_url.clone(),
            })
            .collect();
        let attempted = enabled.iter().map(|s| s.id).collect();
        let categories = self
            .categories
            .iter()
            .map(|c| CategoryInput {
                id: c.id.clone(),
                label: c.label.clone(),
                description: c.description.clone(),
                accent: c.accent.clone(),
            })
            .collect();
        let lens = self.profile.lens.trim();
        let lens = (!lens.is_empty()).then(|| lens.to_string());
        (sources, categories, lens, attempted)
    }

    // ---- derived ----

    pub fn unread_count(&self) -> usize {
        self.articles.iter().filter(|a| !a.read).count()
    }

    pub fn saved(&self) -> Vec<&Article> {
        self.articles.iter().filter(|a| a.bookmarked).collect()
    }

    pub fn last_run_at_ms(&self) -> Option<i64> {
        self.archives
            .first()
            .and_then(|a| DateTime::parse_from_rfc3339(&a.run_at).ok())
            .map(|d| d.timestamp_millis())
    }

    /// Port of the `profileName` expression: profile name, else account name,
    /// else the local part of the account email.
    pub fn display_name(&self, account: Option<&Account>) -> String {
        let p = self.profile.name.trim();
        if !p.is_empty() {
            return p.to_string();
        }
        let Some(acc) = account else {
            return String::new();
        };
        let n = acc.name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        if acc.email.is_empty() {
            return String::new();
        }
        acc.email.split('@').next().unwrap_or("").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Snapshot;
    use crate::types::*;
    use chrono::Utc;
    use std::collections::HashSet;

    fn art(id: u64, source_id: u32) -> Article {
        Article {
            id,
            title: format!("t{id}"),
            source: format!("s{source_id}"),
            source_id,
            category: "c".into(),
            category_label: "C".into(),
            summary: String::new(),
            published: String::new(),
            published_at: None,
            read: false,
            bookmarked: false,
            accent: "ink".into(),
            url: String::new(),
            image_url: None,
        }
    }
    fn src(id: u32, cat: &str, is_default: bool) -> Source {
        Source {
            id,
            name: format!("s{id}"),
            category: cat.into(),
            feed_url: Some("https://x".into()),
            enabled: true,
            is_default,
            last_fetch_at: Some(5),
            health: SourceHealth::Ok,
        }
    }
    fn resp(arts: Vec<Article>, errs: Vec<SourceError>) -> BriefingResponse {
        BriefingResponse {
            articles: arts,
            errors: errs,
            fetched_at: Utc::now(),
        }
    }
    fn err(id: u32) -> SourceError {
        SourceError {
            source_id: id,
            source_name: "n".into(),
            message: "m".into(),
        }
    }
    fn model() -> RadarModel {
        RadarModel::from_snapshot(Snapshot::default())
    }

    #[test]
    fn slugify_dedupes() {
        let mut m = model();
        assert_eq!(
            m.add_category("Packaging".into(), " d ".into(), "camel".into()),
            "packaging"
        );
        assert_eq!(
            m.add_category("Packaging".into(), "d".into(), "camel".into()),
            "packaging-2"
        );
        assert_eq!(
            m.add_category("!!!".into(), "d".into(), "camel".into()),
            "category"
        );
        assert_eq!(m.categories[0].description, "d");
    }

    #[test]
    fn delete_category_blocked_when_in_use() {
        let mut m = model();
        m.add_category("Tools".into(), "d".into(), "ink".into());
        m.sources.push(src(1, "tools", false));
        m.sources.push(src(2, "tools", false));
        assert_eq!(m.delete_category("tools"), Err(2));
        m.sources.clear();
        assert_eq!(m.delete_category("tools"), Ok(()));
        assert!(m.categories.is_empty());
    }

    #[test]
    fn update_category_patches() {
        let mut m = model();
        m.add_category("Tools".into(), "d".into(), "ink".into());
        m.update_category(
            "tools",
            CategoryPatch {
                label: Some("T".into()),
                ..Default::default()
            },
        );
        assert_eq!(m.categories[0].label, "T");
        assert_eq!(m.categories[0].accent, "ink");
    }

    #[test]
    fn delete_default_source_refused() {
        let mut m = model();
        m.sources = vec![src(1, "a", true), src(2, "a", false)];
        assert!(m.delete_source(1).is_err());
        assert!(m.delete_source(2).is_ok());
        assert_eq!(m.sources.len(), 1);
        let id = m.add_source("N".into(), "a".into(), Some(String::new()));
        assert_eq!(id, 2);
        assert_eq!(m.sources[1].feed_url, None);
        m.toggle_source(2);
        assert!(!m.sources[1].enabled);
        m.update_source(
            2,
            SourcePatch {
                name: Some("Z".into()),
                feed_url: Some(Some("https://y".into())),
                ..Default::default()
            },
        );
        assert_eq!(m.sources[1].name, "Z");
        assert_eq!(m.sources[1].feed_url.as_deref(), Some("https://y"));
    }

    #[test]
    fn finish_briefing_stamps_health() {
        let mut m = model();
        m.sources = vec![src(1, "a", false), src(2, "a", false), src(3, "a", false)];
        let attempted: HashSet<u32> = [1, 2].into();
        m.begin_briefing();
        assert!(m.scanning);
        let out = m.finish_briefing(resp(vec![art(1, 1)], vec![err(2), err(0)]), &attempted, 999);
        assert_eq!(m.sources[0].health, SourceHealth::Ok);
        assert_eq!(m.sources[0].last_fetch_at, Some(999));
        assert_eq!(m.sources[1].health, SourceHealth::Warn);
        assert_eq!(m.sources[1].last_fetch_at, Some(5));
        assert_eq!(m.sources[2].last_fetch_at, Some(5));
        assert!(!m.scanning);
        assert_eq!(out.new_count, 1);
        assert_eq!(out.total, 1);
        assert_eq!(out.source_count, 1);
        assert_eq!(m.briefing_errors.len(), 2);
    }

    #[test]
    fn new_count_uses_articles_before_begin() {
        let mut m = model();
        m.articles = vec![art(1, 1)];
        m.begin_briefing();
        assert!(m.articles.is_empty());
        let out = m.finish_briefing(
            resp(vec![art(1, 1), art(2, 1)], vec![]),
            &HashSet::new(),
            10,
        );
        assert_eq!(out.new_count, 1);
        assert_eq!(out.total, 2);
    }

    #[test]
    fn finish_briefing_overlays_states_and_archives() {
        let mut m = model();
        m.article_states.insert(
            "7".into(),
            ArticleState {
                read: true,
                bookmarked: true,
            },
        );
        m.finish_briefing(resp(vec![art(7, 1)], vec![]), &HashSet::new(), 1000);
        assert!(m.articles[0].read && m.articles[0].bookmarked);
        assert_eq!(m.archives.len(), 1);
        assert_eq!(m.archives[0].id, 1000);
        assert_eq!(m.archives[0].run_at, "1970-01-01T00:00:01.000Z");
        assert!(!m.archives[0].articles[0].read);
        m.finish_briefing(resp(vec![], vec![]), &HashSet::new(), 2000);
        assert_eq!(m.archives.len(), 1);
        assert_eq!(m.last_run_at_ms(), Some(1000));
    }

    #[test]
    fn archive_cap_90() {
        let mut m = model();
        for i in 0..95 {
            m.finish_briefing(
                resp(vec![art(i, 1)], vec![]),
                &HashSet::new(),
                1000 + i as i64,
            );
        }
        assert_eq!(m.archives.len(), 90);
        assert_eq!(m.archives[0].id, 1094);
    }

    #[test]
    fn toggles_and_counts() {
        let mut m = model();
        m.articles = vec![art(1, 1), art(2, 1)];
        let (k, s) = m.toggle_read(1).unwrap();
        assert_eq!(k, "1");
        assert_eq!(
            s,
            ArticleState {
                read: true,
                bookmarked: false
            }
        );
        let (_, s) = m.toggle_bookmark(1).unwrap();
        assert_eq!(
            s,
            ArticleState {
                read: true,
                bookmarked: true
            }
        );
        assert!(m.toggle_read(99).is_none());
        assert_eq!(m.unread_count(), 1);
        assert_eq!(m.saved().len(), 1);
        assert_eq!(m.article_states["1"], s);
    }

    #[test]
    fn display_name_and_inputs() {
        let mut m = model();
        let acc = Account {
            id: "u".into(),
            email: "maya@x.com".into(),
            name: " ".into(),
            image_url: None,
        };
        assert_eq!(m.display_name(Some(&acc)), "maya");
        assert_eq!(m.display_name(None), "");
        m.profile.name = " Jo ".into();
        assert_eq!(m.display_name(Some(&acc)), "Jo");
        m.profile.lens = "  ".into();
        let mut s2 = src(2, "a", false);
        s2.enabled = false;
        let mut s3 = src(3, "a", false);
        s3.feed_url = None;
        m.sources = vec![src(1, "a", false), s2, s3];
        let (si, _ci, lens, att) = m.briefing_inputs();
        assert_eq!(si.len(), 1);
        assert_eq!(lens, None);
        assert_eq!(att, HashSet::from([1]));
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut m = model();
        m.sources = vec![src(1, "a", false)];
        let snap = m.to_snapshot(vec![serde_json::json!(1)]);
        assert_eq!(snap.sync_queue.len(), 1);
        assert_eq!(RadarModel::from_snapshot(snap).sources.len(), 1);
    }
}
