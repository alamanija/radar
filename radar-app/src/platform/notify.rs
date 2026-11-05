//! System notification when a briefing brings new articles.
//!
//! The model decides when to post: new articles, and no Radar window has
//! focus (if the user is already looking, the re-rendered briefing is the
//! notification). macOS asks for permission on first use and only delivers
//! from an app bundle; Windows needs the app identity that `main.rs` sets.

use gpui_kit::{App, SystemNotification};
use radar_core::model::BriefingOutcome;

/// Every briefing notification shares this tag, so a new one replaces the
/// last.
pub const TAG: &str = "briefing";
pub const TITLE: &str = "Radar briefing ready";

/// `"{n} new · {total} total across {s} source(s)"`.
fn body(o: &BriefingOutcome) -> String {
    let plural = if o.source_count == 1 { "" } else { "s" };
    format!(
        "{} new · {} total across {} source{plural}",
        o.new_count, o.total, o.source_count
    )
}

/// Posts "Radar briefing ready".
pub fn briefing_ready(cx: &mut App, outcome: BriefingOutcome) {
    log::info!("[notify] briefing ready: {} new", outcome.new_count);
    cx.show_system_notification(SystemNotification {
        tag: TAG.into(),
        title: TITLE.into(),
        body: body(&outcome).into(),
        actions: vec![],
    });
}

/// Clicking a Radar notification opens (or focuses) the main window. Call
/// once at startup.
pub fn init(cx: &mut App) {
    cx.on_system_notification_response(|response, cx| {
        log::info!("[notify] {} notification clicked", response.tag);
        crate::open_main_window(cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_model::AppModel;
    use gpui_kit::{SystemNotificationResponse, TestAppContext};

    fn outcome(new_count: usize, total: usize, source_count: usize) -> BriefingOutcome {
        BriefingOutcome {
            new_count,
            total,
            source_count,
        }
    }

    /// The test platform records notifications instead of posting them.
    #[gpui_kit::test]
    fn briefing_ready_posts_the_expected_copy(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_app_identity("com.radar.test", "Radar");
            briefing_ready(cx, outcome(3, 10, 1));
            briefing_ready(cx, outcome(1, 4, 2));
        });
        let shown = cx.shown_system_notifications();
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].tag.as_ref(), "briefing");
        assert_eq!(shown[0].title.as_ref(), "Radar briefing ready");
        assert_eq!(shown[0].body.as_ref(), "3 new · 10 total across 1 source");
        assert!(shown[0].actions.is_empty());
        assert_eq!(shown[1].body.as_ref(), "1 new · 4 total across 2 sources");
        assert_eq!(
            cx.delivered_system_notifications().len(),
            1,
            "the newer banner replaces the older one"
        );
    }

    #[gpui_kit::test]
    fn clicking_the_notification_opens_the_main_window(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppModel::init(dir.path().join("radar.json"), None, cx);
            crate::workspace::init(cx);
            init(cx);
        });
        assert!(cx.windows().is_empty());
        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: TAG.into(),
            action_id: None,
        });
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
    }
}
