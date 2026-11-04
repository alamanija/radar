//! Search palette semantics (port of `SearchPalette.jsx`).

use crate::types::{Archive, Article, Source};
use std::collections::HashSet;

const MAX_ARTICLE_HITS: usize = 12;
const MAX_SOURCE_HITS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Article(Article),
    Source(Source),
}

fn matches(hay: &str, q: &str) -> bool {
    hay.to_lowercase().contains(q)
}

/// Case-insensitive substring search. Articles come from `articles` plus all
/// archive articles, deduped by id with the current copy winning. Article hits
/// precede source hits.
pub fn search(
    query: &str,
    articles: &[Article],
    archives: &[Archive],
    sources: &[Source],
) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let all = articles
        .iter()
        .chain(archives.iter().flat_map(|b| b.articles.iter()))
        .filter(|a| seen.insert(a.id));
    let mut hits: Vec<Hit> = all
        .filter(|a| matches(&a.title, &q) || matches(&a.source, &q) || matches(&a.summary, &q))
        .take(MAX_ARTICLE_HITS)
        .map(|a| Hit::Article(a.clone()))
        .collect();
    hits.extend(
        sources
            .iter()
            .filter(|s| {
                matches(&s.name, &q)
                    || matches(&s.category, &q)
                    || s.feed_url.as_deref().is_some_and(|u| matches(u, &q))
            })
            .take(MAX_SOURCE_HITS)
            .map(|s| Hit::Source(s.clone())),
    );
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Archive, Article, Source, SourceHealth};

    fn art(id: u64, read: bool) -> Article {
        Article {
            id,
            title: "Bauhaus rebrand".into(),
            source: "Dezeen".into(),
            source_id: 1,
            category: "c".into(),
            category_label: "C".into(),
            summary: String::new(),
            published: String::new(),
            published_at: None,
            read,
            bookmarked: false,
            accent: "ink".into(),
            url: String::new(),
            image_url: None,
        }
    }

    #[test]
    fn search_dedups_current_wins() {
        let arch = vec![Archive {
            id: 1,
            run_at: String::new(),
            articles: vec![art(7, false), art(8, false)],
            errors: vec![],
        }];
        let hits = search("BAUHAUS", &[art(7, true)], &arch, &[]);
        assert_eq!(hits.len(), 2);
        match &hits[0] {
            Hit::Article(a) => {
                assert_eq!(a.id, 7);
                assert!(a.read);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn search_sources_and_empty() {
        let s = Source {
            id: 1,
            name: "Dieline".into(),
            category: "packaging".into(),
            feed_url: Some("https://x/feed".into()),
            enabled: true,
            is_default: false,
            last_fetch_at: None,
            health: SourceHealth::Ok,
        };
        assert!(search("  ", &[], &[], std::slice::from_ref(&s)).is_empty());
        assert_eq!(
            search("PACKAG", &[], &[], std::slice::from_ref(&s)),
            vec![Hit::Source(s.clone())]
        );
        assert_eq!(search("x/feed", &[], &[], &[s]).len(), 1);
    }
}
