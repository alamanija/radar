//! UI-free application state and its mutations. No IO and no clocks: time
//! is always a parameter.

use crate::ingest::{BriefingRequest, CategoryInput, KnownSummary, SourceInput};
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

/// Most top stories a briefing leads with.
pub const TOP_STORIES_MAX: usize = 3;
/// Lowest importance score that qualifies as a top story.
pub const TOP_STORY_MIN_IMPORTANCE: u8 = 4;

/// A briefing split for display ([`RadarModel::briefing_sections`]).
#[derive(Debug)]
pub struct Sections<'a> {
    pub top: Vec<&'a Article>,
    pub new: Vec<&'a Article>,
    pub earlier: Vec<&'a Article>,
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

    /// Sets the read flag; `None` when the article is unknown or already
    /// has it (nothing to save or PATCH).
    pub fn set_read(&mut self, id: u64, read: bool) -> Option<(String, ArticleState)> {
        if self.articles.iter().find(|a| a.id == id)?.read == read {
            return None;
        }
        self.toggle(id, |a| ArticleState {
            read,
            bookmarked: a.bookmarked,
        })
    }

    /// Marks every unread article in the active category read; returns the
    /// states that changed.
    pub fn mark_all_read(&mut self) -> Vec<(String, ArticleState)> {
        let ids: Vec<u64> = self
            .articles
            .iter()
            .filter(|a| !a.read && self.in_active_category(a))
            .map(|a| a.id)
            .collect();
        ids.into_iter()
            .filter_map(|id| self.set_read(id, true))
            .collect()
    }

    fn in_active_category(&self, a: &Article) -> bool {
        self.active_category == "all" || a.category == self.active_category
    }

    /// The briefing's articles under the category filter and, with
    /// `unread_only`, only unread ones plus `kept` (articles read while the
    /// filter was on, which stay until the caller resets the set).
    pub fn briefing_articles(&self, kept: Option<&HashSet<u64>>) -> Vec<&Article> {
        self.articles
            .iter()
            .filter(|a| self.in_active_category(a))
            .filter(|a| {
                !self.prefs.unread_only || !a.read || kept.is_some_and(|k| k.contains(&a.id))
            })
            .collect()
    }

    /// The filtered briefing ([`briefing_articles`](Self::briefing_articles))
    /// split for display: up to [`TOP_STORIES_MAX`] new articles scored at
    /// least [`TOP_STORY_MIN_IMPORTANCE`] (highest first, ties in live
    /// order), the other new articles, and the rest. New and Earlier keep
    /// the live order.
    pub fn briefing_sections(&self, kept: Option<&HashSet<u64>>) -> Sections<'_> {
        let new_ids = self.new_ids();
        let (fresh, earlier): (Vec<&Article>, Vec<&Article>) = self
            .briefing_articles(kept)
            .into_iter()
            .partition(|a| new_ids.contains(&a.id));
        let mut top: Vec<&Article> = fresh
            .iter()
            .copied()
            .filter(|a| a.importance.is_some_and(|i| i >= TOP_STORY_MIN_IMPORTANCE))
            .collect();
        top.sort_by_key(|a| std::cmp::Reverse(a.importance));
        top.truncate(TOP_STORIES_MAX);
        let new = fresh
            .into_iter()
            .filter(|a| !top.iter().any(|t| t.id == a.id))
            .collect();
        Sections { top, new, earlier }
    }

    /// A copy of `a` for display: the age from `publishedAt` at `now_ms`
    /// (the stored text was written at fetch time), and the label and color
    /// of its category as it is now. Without a timestamp, or for a category
    /// that no longer exists, the stored values stand.
    pub fn for_display(&self, a: &Article, now_ms: i64) -> Article {
        let mut d = a.clone();
        if let Some(t) = a.published_at {
            d.published = crate::text::relative_time_ms(Some(t.timestamp_millis()), now_ms);
        }
        if let Some(c) = self.categories.iter().find(|c| c.id == a.category) {
            d.category_label = c.label.clone();
            d.accent = c.accent.clone();
        }
        d
    }

    // ---- briefing ----

    pub fn begin_briefing(&mut self) {
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
            new_count: self.new_ids().len(),
            total,
            source_count,
        }
    }

    /// Live articles that appear in no earlier briefing run. The live run
    /// is the newest archive entry with exactly the live article ids
    /// (another device's later run may sit above it); earlier runs are the
    /// entries older than it. Without such an entry, every entry but the
    /// newest counts as earlier.
    pub fn new_ids(&self) -> HashSet<u64> {
        let live: HashSet<u64> = self.articles.iter().map(|a| a.id).collect();
        if live.is_empty() {
            return live;
        }
        let ids = |a: &Archive| a.articles.iter().map(|x| x.id).collect::<HashSet<_>>();
        let at = |a: &Archive| DateTime::parse_from_rfc3339(&a.run_at).ok();
        let earlier: Vec<&Archive> = match self.archives.iter().find(|a| ids(a) == live) {
            Some(run) => {
                let run_at = at(run);
                self.archives
                    .iter()
                    .filter(|a| !std::ptr::eq(*a, run) && at(a) < run_at)
                    .collect()
            }
            None => self.archives.iter().skip(1).collect(),
        };
        let seen: HashSet<u64> = earlier
            .iter()
            .flat_map(|a| a.articles.iter().map(|x| x.id))
            .collect();
        live.difference(&seen).copied().collect()
    }

    pub fn fail_briefing(&mut self, msg: &str) {
        self.briefing_errors = vec![SourceError {
            source_id: 0,
            source_name: "radar".into(),
            message: msg.to_string(),
        }];
        self.scanning = false;
    }

    /// Some source a briefing would fetch: enabled, with a feed URL.
    /// Run-at-launch and the scheduler only start a briefing when there is
    /// one.
    pub fn has_usable_source(&self) -> bool {
        self.sources.iter().any(is_fetched)
    }

    /// The next briefing's request, and the ids of the sources it will
    /// attempt (for the health stamps in [`finish_briefing`](Self::finish_briefing)).
    pub fn briefing_inputs(&self) -> (BriefingRequest, HashSet<u32>) {
        let enabled: Vec<&Source> = self.sources.iter().filter(|s| is_fetched(s)).collect();
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
        let req = BriefingRequest {
            sources,
            categories,
            lens,
            model: self.prefs.summary_model,
            known: self.known_summaries(),
            stale_after_days: self.prefs.stale_after_days,
        };
        (req, attempted)
    }

    /// Every summary Claude wrote that is still around: the live articles,
    /// then the archives newest first; the first copy of an id wins.
    fn known_summaries(&self) -> HashMap<u64, KnownSummary> {
        let mut known = HashMap::new();
        let all = self
            .articles
            .iter()
            .chain(self.archives.iter().flat_map(|a| a.articles.iter()));
        for a in all {
            if let Some(model) = &a.summarized_by {
                known.entry(a.id).or_insert_with(|| KnownSummary {
                    summary: a.summary.clone(),
                    category: a.category.clone(),
                    model: model.clone(),
                    importance: a.importance,
                });
            }
        }
        known
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

    /// Profile name, else account name, else the local part of the account
    /// email.
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

/// Enabled, with a feed URL: a source a briefing fetches.
fn is_fetched(s: &Source) -> bool {
    s.enabled && s.feed_url.as_deref().is_some_and(|u| !u.is_empty())
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
            summarized_by: None,
            importance: None,
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
    fn new_count_ignores_articles_from_earlier_runs() {
        let mut m = model();
        m.articles = vec![art(1, 1)];
        m.archives = vec![run(1, "1970-01-01T00:00:00.005Z", &[1])];
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
        let (req, att) = m.briefing_inputs();
        assert_eq!(req.sources.len(), 1);
        assert_eq!(req.lens, None);
        assert_eq!(att, HashSet::from([1]));
    }

    #[test]
    fn briefing_inputs_carry_model_age_limit_and_known_summaries() {
        let mut m = model();
        m.prefs.summary_model = SummaryModel::Opus55;
        m.prefs.stale_after_days = 7;
        let summarized = |id: u64, text: &str| Article {
            summary: text.into(),
            summarized_by: Some("claude-haiku-5-5".into()),
            ..art(id, 1)
        };
        m.articles = vec![
            Article {
                importance: Some(4),
                ..summarized(1, "current")
            },
            art(2, 1),
        ];
        m.archives = vec![Archive {
            id: 1,
            run_at: "2026-11-01T07:30:00.000Z".into(),
            // Article 1 again (the live copy wins), 3 only in the archive,
            // 4 never summarized.
            articles: vec![summarized(1, "archived"), summarized(3, "old"), art(4, 1)],
            errors: Vec::new(),
        }];

        let (req, _) = m.briefing_inputs();
        assert_eq!(req.model, SummaryModel::Opus55);
        assert_eq!(req.stale_after_days, 7);
        let mut ids: Vec<u64> = req.known.keys().copied().collect();
        ids.sort();
        assert_eq!(ids, [1, 3], "only summarized articles are known");
        assert_eq!(req.known[&1].summary, "current");
        assert_eq!(req.known[&1].importance, Some(4));
        assert_eq!(req.known[&3].importance, None);
        assert_eq!(req.known[&3].summary, "old");
        assert_eq!(req.known[&3].model, "claude-haiku-5-5");
    }

    #[test]
    fn has_usable_source_needs_an_enabled_feed() {
        let mut m = RadarModel::default();
        assert!(!m.has_usable_source());
        m.sources = vec![src(1, "a", false)];
        assert!(m.has_usable_source());
        m.sources[0].enabled = false;
        assert!(!m.has_usable_source(), "disabled");
        m.sources[0].enabled = true;
        m.sources[0].feed_url = Some(String::new());
        assert!(!m.has_usable_source(), "empty feed URL");
        m.sources[0].feed_url = None;
        assert!(!m.has_usable_source(), "no feed URL");
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut m = model();
        m.sources = vec![src(1, "a", false)];
        let snap = m.to_snapshot(vec![serde_json::json!(1)]);
        assert_eq!(snap.sync_queue.len(), 1);
        assert_eq!(RadarModel::from_snapshot(snap).sources.len(), 1);
    }

    fn in_cat(id: u64, category: &str, read: bool) -> Article {
        Article {
            category: category.into(),
            read,
            ..art(id, 1)
        }
    }

    #[test]
    fn set_read_reports_only_changes() {
        let mut m = model();
        m.articles = vec![in_cat(1, "c", false)];
        let (key, s) = m.set_read(1, true).unwrap();
        assert_eq!(key, "1");
        assert!(s.read && m.articles[0].read);
        assert_eq!(m.article_states["1"], s);
        assert_eq!(m.set_read(1, true), None, "already read");
        assert_eq!(m.set_read(9, true), None, "unknown id");
    }

    #[test]
    fn mark_all_read_follows_the_category_filter() {
        let mut m = model();
        m.articles = vec![
            in_cat(1, "a", false),
            in_cat(2, "a", true),
            in_cat(3, "b", false),
        ];
        m.active_category = "a".into();
        let changed = m.mark_all_read();
        assert_eq!(changed.len(), 1, "only the unread one in `a`");
        assert_eq!(changed[0].0, "1");
        assert!(!m.articles[2].read, "other categories are untouched");

        m.active_category = "all".into();
        let changed = m.mark_all_read();
        assert_eq!(
            changed.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["3"]
        );
        assert!(m.articles.iter().all(|a| a.read));
        assert!(m.mark_all_read().is_empty());
    }

    #[test]
    fn briefing_articles_filter_by_category_and_unread() {
        let mut m = model();
        m.articles = vec![
            in_cat(1, "a", false),
            in_cat(2, "a", true),
            in_cat(3, "b", false),
            in_cat(4, "a", true),
        ];
        let ids = |v: Vec<&Article>| v.iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(m.briefing_articles(None)), [1, 2, 3, 4]);
        m.active_category = "a".into();
        assert_eq!(ids(m.briefing_articles(None)), [1, 2, 4]);

        m.prefs.unread_only = true;
        assert_eq!(ids(m.briefing_articles(None)), [1]);
        // Opened while the filter was on: kept until the set is reset.
        let kept = HashSet::from([4]);
        assert_eq!(ids(m.briefing_articles(Some(&kept))), [1, 4]);
    }

    #[test]
    fn for_display_uses_live_time_and_current_category() {
        let mut m = model();
        m.categories = vec![Category {
            id: "c".into(),
            label: "Renamed".into(),
            description: String::new(),
            accent: "olive".into(),
        }];
        let now = DateTime::parse_from_rfc3339("2026-11-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let a = Article {
            published: "2h ago".into(),
            published_at: Some(now - chrono::Duration::days(3)),
            ..art(1, 1)
        };
        let d = m.for_display(&a, now.timestamp_millis());
        assert_eq!(
            d.published, "3d ago",
            "from the timestamp, not the stored text"
        );
        assert_eq!(d.category_label, "Renamed");
        assert_eq!(d.accent, "olive");
        assert_eq!(d.id, a.id);

        // No timestamp or an unknown category: the stored values stand.
        let b = Article {
            category: "gone".into(),
            published: "recent".into(),
            ..art(2, 1)
        };
        let d = m.for_display(&b, now.timestamp_millis());
        assert_eq!(d.published, "recent");
        assert_eq!(d.category_label, "C");
        assert_eq!(d.accent, "ink");
    }

    fn run(id: i64, run_at: &str, ids: &[u64]) -> Archive {
        Archive {
            id,
            run_at: run_at.into(),
            articles: ids.iter().map(|&i| art(i, 1)).collect(),
            errors: Vec::new(),
        }
    }

    #[test]
    fn new_ids_exclude_articles_seen_in_earlier_runs() {
        let mut m = model();
        m.articles = vec![art(1, 1), art(2, 1), art(3, 1)];
        m.archives = vec![
            run(2, "2026-11-05T07:30:00.000Z", &[1, 2, 3]),
            run(1, "2026-11-04T07:30:00.000Z", &[2]),
        ];
        assert_eq!(m.new_ids(), HashSet::from([1, 3]));
    }

    #[test]
    fn a_later_run_from_another_device_does_not_hide_new_articles() {
        let mut m = model();
        m.articles = vec![art(1, 1), art(2, 1), art(3, 1)];
        m.archives = vec![
            run(3, "2026-11-05T12:00:00.000Z", &[9]),
            run(2, "2026-11-05T07:30:00.000Z", &[1, 2, 3]),
            run(1, "2026-11-04T07:30:00.000Z", &[2]),
        ];
        assert_eq!(m.new_ids(), HashSet::from([1, 3]));
    }

    #[test]
    fn without_a_matching_run_all_but_the_newest_count_as_earlier() {
        let mut m = model();
        m.articles = vec![art(1, 1), art(2, 1)];
        m.archives = vec![
            run(2, "2026-11-05T07:30:00.000Z", &[5]),
            run(1, "2026-11-04T07:30:00.000Z", &[1]),
        ];
        assert_eq!(m.new_ids(), HashSet::from([2]));
        assert!(model().new_ids().is_empty(), "no articles, nothing new");
    }

    #[test]
    fn finish_briefing_counts_new_from_the_run_history() {
        let mut m = model();
        m.archives = vec![run(1, "2026-11-04T07:30:00.000Z", &[1])];
        m.begin_briefing();
        let out = m.finish_briefing(
            resp(vec![art(1, 1), art(2, 1)], vec![]),
            &HashSet::from([1]),
            1_794_300_000_000,
        );
        assert_eq!(out.new_count, 1);
        assert_eq!(m.new_ids(), HashSet::from([2]));
    }

    fn scored(id: u64, category: &str, importance: Option<u8>) -> Article {
        Article {
            category: category.into(),
            importance,
            ..art(id, 1)
        }
    }

    /// `articles` live, in order; `earlier` ids also in an older run.
    fn with_history(articles: Vec<Article>, earlier: &[u64]) -> RadarModel {
        let mut m = model();
        let live: Vec<u64> = articles.iter().map(|a| a.id).collect();
        m.articles = articles;
        m.archives = vec![
            run(2, "2026-11-05T07:30:00.000Z", &live),
            run(1, "2026-11-04T07:30:00.000Z", earlier),
        ];
        m
    }

    fn ids(v: &[&Article]) -> Vec<u64> {
        v.iter().map(|a| a.id).collect()
    }

    #[test]
    fn sections_lead_with_up_to_three_important_new_articles() {
        let m = with_history(
            vec![
                scored(1, "c", Some(5)),
                scored(2, "c", Some(4)),
                scored(3, "c", Some(4)),
                scored(4, "c", Some(5)),
                scored(5, "c", Some(3)),
                scored(6, "c", Some(5)),
                scored(7, "c", None),
            ],
            &[6, 7],
        );
        let s = m.briefing_sections(None);
        assert_eq!(ids(&s.top), [1, 4, 2], "score first, then live order");
        assert_eq!(ids(&s.new), [3, 5]);
        assert_eq!(ids(&s.earlier), [6, 7], "an earlier 5 is never top");
    }

    #[test]
    fn unscored_and_low_scored_articles_are_never_top() {
        let m = with_history(vec![scored(1, "c", None), scored(2, "c", Some(3))], &[]);
        let s = m.briefing_sections(None);
        assert!(s.top.is_empty());
        assert_eq!(ids(&s.new), [1, 2]);
        assert!(s.earlier.is_empty());
    }

    #[test]
    fn sections_respect_category_and_unread_filters() {
        let mut m = with_history(
            vec![
                scored(1, "a", Some(5)),
                scored(2, "b", Some(5)),
                Article {
                    read: true,
                    ..scored(3, "b", Some(4))
                },
                scored(4, "b", None),
            ],
            &[4],
        );
        m.active_category = "b".into();
        let s = m.briefing_sections(None);
        assert_eq!(ids(&s.top), [2, 3]);
        assert!(s.new.is_empty());
        assert_eq!(ids(&s.earlier), [4]);

        m.prefs.unread_only = true;
        assert_eq!(ids(&m.briefing_sections(None).top), [2], "read: dropped");
        let kept = HashSet::from([3]);
        assert_eq!(
            ids(&m.briefing_sections(Some(&kept)).top),
            [2, 3],
            "opened under the filter: stays"
        );
    }
}
