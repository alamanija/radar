//! Port of `src/components/SyncIndicator.jsx` (and `useSyncStatus.js`): the
//! one-line sync status above the sidebar footer, shown while signed in.
//! Quiet when idle, amber while changes are queued or in flight, red when
//! the queue reports a failure and nothing is left to retry.

use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    div, px, rgb, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled,
};
use radar_core::sync::SyncStatus;
use radar_core::text::relative_time_ms;

pub const ID: &str = "sync-indicator";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Idle,
    Busy,
    Error,
}

/// What the row says: its dot tone, label and hover title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indicator {
    pub tone: Tone,
    pub label: String,
    pub title: String,
}

/// The JSX rules: an error with nothing pending wins, then pending or
/// in-flight work, else idle. Idle adds the last successful sync time to
/// the title when one is known.
pub fn indicator(st: &SyncStatus, now_ms: i64) -> Indicator {
    if let (Some(err), 0) = (&st.last_error, st.pending) {
        return Indicator {
            tone: Tone::Error,
            label: "Sync issue".into(),
            title: err.clone(),
        };
    }
    if st.pending > 0 || st.inflight {
        let label = match st.pending {
            0 => "Syncing…".to_string(),
            1 => "Syncing 1 change…".to_string(),
            n => format!("Syncing {n} changes…"),
        };
        return Indicator {
            tone: Tone::Busy,
            label,
            title: "Changes will keep retrying until they reach the server".into(),
        };
    }
    let mut title = String::from("All changes saved to Radar");
    if st.last_success_at.is_some() {
        title.push_str(" · ");
        title.push_str(&relative_time_ms(st.last_success_at, now_ms));
    }
    Indicator {
        tone: Tone::Idle,
        label: "Synced".into(),
        title,
    }
}

fn dot(tone: Tone, t: &Tokens) -> Hsla {
    match tone {
        Tone::Idle => t.text_3,
        Tone::Busy => rgb(0xC48A2A).into(),
        Tone::Error => rgb(0xB3513A).into(),
    }
}

/// `styles.row`: 6/12 padding, 11 px text-3 on surface-2 between two
/// borders; a 7 px dot and the label (one line, ellipsis, tabular figures).
/// The title is the hover tooltip and the accessible description.
pub fn render(st: &SyncStatus, t: &Tokens) -> impl IntoElement {
    let now = chrono::Utc::now().timestamp_millis();
    let ind = indicator(st, now);
    let title = SharedString::from(ind.title);
    let tip = title.clone();
    div()
        .id(ID)
        .test_support()
        .aria_label(ind.label.clone())
        .aria_description(title)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .py(px(6.))
        .px(px(12.))
        .text_size(px(11.))
        .text_color(t.text_3)
        .border_t_1()
        .border_b_1()
        .border_color(t.border)
        .bg(t.surface_2)
        .child(
            div()
                .flex_none()
                .size(px(7.))
                .rounded_full()
                .bg(dot(ind.tone, t)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_features(theme::font_features(true))
                .child(ind.label),
        )
}
