//! Headless UI tests for the shell (sidebar, header, tweaks, actions).
//! Run with `cargo test -p radar-app shell`.

use crate::app_model::{AppModel, SAVE_DEBOUNCE};
use crate::theme;
use crate::workspace::{self, OpenSearch, SourcesIntent, ToggleSidebar, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use radar_core::types::{Accent, Account, Article, Category, Density, Theme, View};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct Shell {
    _dir: tempfile::TempDir,
    pub(crate) path: PathBuf,
    pub(crate) window: AnyWindowHandle,
    pub(crate) ws: Entity<Workspace>,
    pub(crate) model: Entity<AppModel>,
}

pub(crate) fn boot(cx: &mut TestAppContext) -> Shell {
    // Work on the tokio runtime (briefing, sync PATCH) wakes GPUI tasks from
    // its own threads; without this the scheduler flags that as a race.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("radar.json");
    let (window, ws, model) = cx.update(|cx| {
        gpui_kit::init(cx);
        let model = AppModel::init(path.clone(), None, cx);
        workspace::init(cx);
        let (window, ws) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .unwrap();
        (window, ws, model)
    });
    cx.run_until_parked();
    Shell {
        _dir: dir,
        path,
        window,
        ws,
        model,
    }
}

impl Shell {
    pub(crate) fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        cx.update_window(self.window, |_, window, cx| window.click(id, cx))
            .unwrap();
        cx.run_until_parked();
    }

    pub(crate) fn click_dyn(&self, cx: &mut TestAppContext, id: String) {
        cx.update_window(self.window, |_, window, cx| {
            window.click(gpui_kit::SharedString::from(id), cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Accessible label of an element in a freshly rendered frame.
    pub(crate) fn label(
        &self,
        cx: &mut TestAppContext,
        id: impl Into<gpui_kit::ElementId>,
    ) -> Option<String> {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().map(str::to_owned)
        })
        .unwrap()
    }

    pub(crate) fn exists(
        &self,
        cx: &mut TestAppContext,
        id: impl Into<gpui_kit::ElementId>,
    ) -> bool {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    pub(crate) fn edit(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut radar_core::model::RadarModel),
    ) {
        self.model.update(cx, |m, cx| m.update_model(cx, f));
        cx.run_until_parked();
    }

    pub(crate) fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&AppModel) -> R) -> R {
        self.model.read_with(cx, |m, _| f(m))
    }
}

pub(crate) fn article(id: u64, category: &str, read: bool) -> Article {
    Article {
        id,
        title: format!("Article {id}"),
        source: "Feed".into(),
        source_id: 1,
        category: category.into(),
        category_label: category.into(),
        summary: String::new(),
        published: "1h ago".into(),
        published_at: None,
        read,
        bookmarked: false,
        accent: "olive".into(),
        url: format!("https://x.test/{id}"),
        image_url: None,
    }
}

pub(crate) fn category(id: &str) -> Category {
    Category {
        id: id.into(),
        label: id.to_uppercase(),
        description: String::new(),
        accent: "olive".into(),
    }
}

#[gpui_kit::test]
fn sidebar_click_sets_view(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.click(cx, "nav-saved");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Saved);
    assert_eq!(s.label(cx, "content-title").as_deref(), Some("Saved"));

    cx.executor().advance_clock(SAVE_DEBOUNCE);
    cx.run_until_parked();
    assert_eq!(radar_core::store::load(&s.path).prefs.view, View::Saved);
}

#[gpui_kit::test]
fn sidebar_category_filter_sets_active_category(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.categories = vec![category("packaging"), category("tools")];
        m.prefs.view = View::Archive;
    });
    s.click_dyn(cx, "sidebar-category-tools".into());
    s.read(cx, |m| {
        assert_eq!(m.model.prefs.view, View::Briefing);
        assert_eq!(m.model.active_category, "tools");
    });
}

#[gpui_kit::test]
fn sidebar_counts(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| {
        m.categories = vec![category("packaging")];
        m.articles = vec![
            article(1, "packaging", false),
            article(2, "packaging", true),
            article(3, "tools", false),
        ];
        m.articles[0].bookmarked = true;
    });
    let unread = s.read(cx, |m| m.model.unread_count());
    assert_eq!(unread, 2);
    assert_eq!(s.label(cx, "nav-briefing-count"), Some(unread.to_string()));
    assert_eq!(s.label(cx, "nav-saved-count").as_deref(), Some("1"));
    assert_eq!(s.label(cx, "nav-categories-count").as_deref(), Some("1"));
}

#[gpui_kit::test]
fn sidebar_add_source_opens_sources_with_intent(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.edit(cx, |m| m.categories = vec![category("tools")]);
    s.click(cx, "sidebar-add-source");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Sources);
    // The Sources view consumes the intent (and opens its add row).
    assert!(s.exists(cx, "source-edit-name"));
    let intent = s.ws.read_with(cx, |w, _| w.sources_intent);
    assert_eq!(intent, None::<SourcesIntent>);
}

#[gpui_kit::test]
fn sidebar_source_row_toggles_source(cx: &mut TestAppContext) {
    let s = boot(cx);
    let id = s.model.update(cx, |m, cx| {
        m.update_model(cx, |r| {
            r.add_source("A".into(), "tools".into(), Some("https://a.test".into()))
        })
    });
    cx.run_until_parked();
    s.click_dyn(cx, format!("sidebar-source-{id}"));
    let enabled = s.read(cx, |m| m.model.sources[0].enabled);
    assert!(!enabled);
}

#[gpui_kit::test]
fn header_theme_toggle_flips_and_reapplies_tokens(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert_eq!(s.read(cx, |m| m.model.prefs.theme), Theme::Light);
    s.click(cx, "header-theme-toggle");
    assert_eq!(s.read(cx, |m| m.model.prefs.theme), Theme::Dark);
    let want = theme::tokens(Theme::Dark, Accent::Camel, Density::Cozy);
    cx.read(|cx| {
        assert_eq!(*theme::t(cx), want);
        let kit = gpui_kit::component::Theme::global(cx);
        assert_eq!(kit.mode, gpui_kit::component::ThemeMode::Dark);
        assert_eq!(kit.background, want.bg);
    });
    s.click(cx, "header-theme-toggle");
    assert_eq!(s.read(cx, |m| m.model.prefs.theme), Theme::Light);
}

#[gpui_kit::test]
fn header_view_mode_segmented_only_on_briefing(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert!(s.exists(cx, "header-view-list"));
    s.click(cx, "header-view-list");
    assert_eq!(
        s.read(cx, |m| m.model.prefs.view_mode),
        radar_core::types::ViewMode::List
    );
    s.click(cx, "nav-saved");
    assert!(!s.exists(cx, "header-view-list"));
    assert_eq!(s.label(cx, "header-title").as_deref(), Some("Saved"));
}

#[gpui_kit::test]
fn header_search_opens_search(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.click(cx, "header-search");
    assert!(s.ws.read_with(cx, |w, _| w.search_open));
}

#[gpui_kit::test]
fn ctrl_k_opens_search(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert!(!s.ws.read_with(cx, |w, _| w.search_open));
    cx.dispatch_action(s.window, OpenSearch);
    assert!(s.ws.read_with(cx, |w, _| w.search_open));
}

/// The real chain: the workspace holds focus from window open, and the
/// `secondary-k` binding in context "Workspace" resolves to OpenSearch.
#[gpui_kit::test]
fn ctrl_k_keystroke_opens_search(cx: &mut TestAppContext) {
    let s = boot(cx);
    cx.update_window(s.window, |_, window, cx| window.press("secondary-k", cx))
        .unwrap();
    assert!(s.ws.read_with(cx, |w, _| w.search_open));
}

#[gpui_kit::test]
fn toggle_sidebar_collapses(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert!(s.exists(cx, "sidebar"));
    s.click(cx, "header-sidebar-toggle");
    assert!(s.read(cx, |m| m.model.prefs.sidebar_collapsed));
    assert!(!s.exists(cx, "sidebar"));

    cx.dispatch_action(s.window, ToggleSidebar);
    cx.run_until_parked();
    assert!(!s.read(cx, |m| m.model.prefs.sidebar_collapsed));
    assert!(s.exists(cx, "sidebar"));
}

#[gpui_kit::test]
fn tweaks_accent_swatch_updates_prefs_and_tokens(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert!(!s.exists(cx, "tweaks-panel"));
    s.click(cx, "tweaks-fab");
    assert!(s.exists(cx, "tweaks-panel"));
    assert!(
        !s.exists(cx, "tweaks-fab"),
        "FAB hides while the panel is open"
    );

    s.click(cx, "tweaks-accent-Olive");
    assert_eq!(s.read(cx, |m| m.model.prefs.accent), Accent::Olive);
    cx.read(|cx| {
        assert_eq!(
            theme::t(cx).accent,
            theme::tokens(Theme::Light, Accent::Olive, Density::Cozy).accent
        );
    });

    s.click(cx, "tweaks-close");
    assert!(!s.exists(cx, "tweaks-panel"));
}

#[gpui_kit::test]
fn tweaks_controls_update_prefs(cx: &mut TestAppContext) {
    let s = boot(cx);
    s.click(cx, "tweaks-fab");
    s.click(cx, "tweaks-theme-dark");
    s.click(cx, "tweaks-view-list");
    s.click(cx, "tweaks-density-compact");
    s.click(cx, "tweaks-serif");
    s.click(cx, "tweaks-accent-bars");
    s.click(cx, "tweaks-sidebar-collapsed");
    let p = s.read(cx, |m| m.model.prefs.clone());
    assert_eq!(p.theme, Theme::Dark);
    assert_eq!(p.view_mode, radar_core::types::ViewMode::List);
    assert_eq!(p.density, Density::Compact);
    assert!(!p.use_serif);
    assert!(!p.show_accent);
    assert!(p.sidebar_collapsed);
    cx.read(|cx| assert_eq!(theme::t(cx).row_h, gpui_kit::px(36.)));
}

#[gpui_kit::test]
fn sidebar_footer_shows_profile_name_or_account(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert_eq!(
        s.label(cx, "sidebar-profile-name").as_deref(),
        Some("Set your name")
    );
    assert_eq!(
        s.label(cx, "sidebar-profile-role").as_deref(),
        Some("Settings → Profile")
    );

    s.model.update(cx, |m, cx| {
        m.account = Some(Account {
            id: "user_1".into(),
            email: "grace@navy.mil".into(),
            name: String::new(),
            image_url: None,
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        s.label(cx, "sidebar-profile-name").as_deref(),
        Some("grace")
    );
    assert_eq!(
        s.label(cx, "sidebar-profile-role").as_deref(),
        Some("grace@navy.mil")
    );

    s.edit(cx, |m| {
        m.profile.name = "Ada".into();
        m.profile.role = "Designer".into();
    });
    assert_eq!(s.label(cx, "sidebar-profile-name").as_deref(), Some("Ada"));
    assert_eq!(
        s.label(cx, "sidebar-profile-role").as_deref(),
        Some("Designer")
    );

    s.click(cx, "sidebar-profile");
    assert_eq!(s.read(cx, |m| m.model.prefs.view), View::Settings);
}

#[gpui_kit::test]
fn header_run_briefing_fetches_and_finishes(cx: &mut TestAppContext) {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/feed.xml");
        then.status(200)
            .header("content-type", "application/rss+xml")
            .body(
                r#"<?xml version="1.0"?><rss version="2.0"><channel><title>T</title>
                <item><title>Hello</title><link>https://x.test/1</link>
                <guid>g1</guid><pubDate>Mon, 02 Nov 2026 10:00:00 GMT</pubDate>
                <description>Body</description></item></channel></rss>"#,
            );
    });
    let s = boot(cx);
    s.model.update(cx, |m, cx| {
        m.update_model(cx, |r| {
            r.add_source("Feed".into(), "tools".into(), Some(server.url("/feed.xml")))
        })
    });
    s.click(cx, "header-run-briefing");
    assert!(s.read(cx, |m| m.model.scanning));
    // A second click while scanning is a no-op.
    s.click(cx, "header-run-briefing");

    // The fetch runs on the tokio runtime; wait for it in real time.
    cx.executor().allow_parking();
    for _ in 0..500 {
        cx.run_until_parked();
        if !s.read(cx, |m| m.model.scanning) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    s.read(cx, |m| {
        assert!(!m.model.scanning, "briefing never finished");
        assert_eq!(m.model.articles.len(), 1, "{:?}", m.model.briefing_errors);
        assert_eq!(m.model.archives.len(), 1, "exactly one run happened");
        assert!(m.model.sources[0].last_fetch_at.is_some());
    });
}

#[gpui_kit::test]
fn content_routes_on_view(cx: &mut TestAppContext) {
    let s = boot(cx);
    for (view, title) in [
        (View::Archive, "Archive"),
        (View::Sources, "Sources"),
        (View::Categories, "Categories"),
        (View::Settings, "Settings"),
    ] {
        s.edit(cx, |m| m.prefs.view = view);
        assert_eq!(s.label(cx, "content-title").as_deref(), Some(title));
    }
    // The briefing page titles itself with the greeting, not the header's
    // "Morning Briefing".
    s.edit(cx, |m| m.prefs.view = View::Briefing);
    let greeting = s.label(cx, "content-title").unwrap();
    assert!(greeting.ends_with('.'), "{greeting}");
    assert!(s.exists(cx, "briefing-subtitle"));
}

#[gpui_kit::test]
fn search_placeholder_closes_on_scrim_click_and_escape(cx: &mut TestAppContext) {
    let s = boot(cx);
    assert!(!s.exists(cx, "search-scrim"));
    cx.dispatch_action(s.window, OpenSearch);
    assert!(s.exists(cx, "search-scrim"));
    s.click(cx, "search-scrim");
    assert!(!s.ws.read_with(cx, |w, _| w.search_open));
    assert!(!s.exists(cx, "search-scrim"));

    cx.dispatch_action(s.window, OpenSearch);
    cx.update_window(s.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert!(!s.ws.read_with(cx, |w, _| w.search_open));
}

#[gpui_kit::test]
fn theme_and_briefing_actions_reach_the_model(cx: &mut TestAppContext) {
    let s = boot(cx);
    cx.dispatch_action(s.window, workspace::ToggleTheme);
    assert_eq!(s.read(cx, |m| m.model.prefs.theme), Theme::Dark);

    cx.dispatch_action(s.window, workspace::RunBriefing);
    assert!(s.read(cx, |m| m.model.scanning));
    // No sources: the run finishes at once with nothing to archive.
    cx.executor().allow_parking();
    for _ in 0..500 {
        cx.run_until_parked();
        if !s.read(cx, |m| m.model.scanning) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    s.read(cx, |m| {
        assert!(!m.model.scanning, "briefing never finished");
        assert!(m.model.archives.is_empty());
    });
}
