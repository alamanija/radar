//! Pure text helpers: greeting, relative time and tray tooltip. Time is
//! always a parameter.

use crate::types::Article;
use chrono::{DateTime, Datelike, Local, Timelike};

const TOOLTIP_ARTICLE_COUNT: usize = 3;
const TOOLTIP_LINE_MAX: usize = 72;

fn greeting_group(hour: u32) -> [&'static str; 3] {
    match hour {
        0..=4 => ["Still up", "Night owl mode", "Burning the midnight oil"],
        5..=8 => ["Rise and shine", "Early start", "Up with the sun"],
        9..=11 => ["Good morning", "Morning", "Top of the morning"],
        12..=13 => ["Good afternoon", "High noon", "Afternoon already"],
        14..=17 => ["Good afternoon", "Afternoon", "Back at it"],
        18..=21 => ["Good evening", "Evening", "Winding down"],
        _ => ["Late check-in", "Night owl mode", "One more scroll"],
    }
}

/// Time-of-day greeting; phrase rotates by local date.
pub fn greeting(name: &str, now: DateTime<Local>) -> String {
    let group = greeting_group(now.hour());
    let seed = now.year() as u32 * 10000 + now.month() * 100 + now.day();
    let phrase = group[(seed as usize) % group.len()];
    if name.is_empty() {
        format!("{phrase}.")
    } else {
        format!("{phrase}, {name}.")
    }
}

/// "3m ago" / "2h ago" / "never".
pub fn relative_time_ms(ms: Option<i64>, now_ms: i64) -> String {
    let Some(ms) = ms else {
        return "never".into();
    };
    let s = (now_ms - ms).max(0) / 1000;
    if s < 60 {
        return "just now".into();
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    let d = h / 24;
    if d < 7 {
        return format!("{d}d ago");
    }
    if d < 30 {
        return format!("{}w ago", d / 7);
    }
    format!("{}mo ago", d / 30)
}

/// Archive row/heading date:
/// `Today · 9:05 AM`, `Yesterday · 9:05 AM`, else
/// `Wednesday, November 5 · 9:05 AM`, in local time relative to `now`.
/// An unparseable `iso` is returned unchanged (the raw text at least says
/// what the archive holds).
pub fn format_run_at(iso: &str, now: DateTime<Local>) -> String {
    let Ok(d) = DateTime::parse_from_rfc3339(iso) else {
        return iso.to_string();
    };
    let d = d.with_timezone(&Local);
    let time = d.format("%-I:%M %p");
    let day = d.date_naive();
    let today = now.date_naive();
    if day == today {
        format!("Today · {time}")
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday · {time}")
    } else {
        format!("{} · {time}", d.format("%A, %B %-d"))
    }
}

/// Truncate to `max` chars, ending with an ellipsis when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

pub fn tray_tooltip(
    unread: usize,
    last_run_ms: Option<i64>,
    articles: &[Article],
    now_ms: i64,
) -> String {
    let last_run = relative_time_ms(last_run_ms, now_ms);
    let header = if unread > 0 {
        format!("Radar — {unread} unread · last run {last_run}")
    } else {
        format!("Radar — all caught up · last run {last_run}")
    };
    let lines: Vec<String> = articles
        .iter()
        .take(TOOLTIP_ARTICLE_COUNT)
        .filter(|a| !a.title.is_empty())
        .map(|a| {
            let line = if a.source.is_empty() {
                a.title.clone()
            } else {
                format!("{} — {}", a.source, a.title)
            };
            truncate(&line, TOOLTIP_LINE_MAX)
        })
        .collect();
    if lines.is_empty() {
        header
    } else {
        format!("{header}\n\n{}", lines.join("\n"))
    }
}

pub fn tray_title(unread: usize) -> Option<String> {
    (unread > 0).then(|| unread.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Article;
    use chrono::{Local, TimeZone};

    fn art(source: &str, title: &str) -> Article {
        Article {
            id: 1,
            title: title.into(),
            source: source.into(),
            source_id: 1,
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

    #[test]
    fn greeting_buckets_and_daily_seed() {
        let now = Local.with_ymd_and_hms(2026, 11, 4, 7, 0, 0).unwrap();
        // 20261104 % 3 == 1
        assert_eq!(greeting("Maya", now), "Early start, Maya.");
        assert_eq!(greeting("", now), "Early start.");
        let late = Local.with_ymd_and_hms(2026, 11, 4, 23, 0, 0).unwrap();
        assert_eq!(greeting("", late), "Night owl mode.");
    }

    #[test]
    fn relative_time_ms_cases() {
        let now = 1_000_000_000_000i64;
        assert_eq!(relative_time_ms(None, now), "never");
        assert_eq!(relative_time_ms(Some(now - 30_000), now), "just now");
        assert_eq!(relative_time_ms(Some(now - 5 * 60_000), now), "5m ago");
        assert_eq!(relative_time_ms(Some(now - 3 * 3_600_000), now), "3h ago");
        assert_eq!(relative_time_ms(Some(now - 10 * 86_400_000), now), "1w ago");
        assert_eq!(
            relative_time_ms(Some(now - 65 * 86_400_000), now),
            "2mo ago"
        );
        assert_eq!(relative_time_ms(Some(now + 5_000), now), "just now");
    }

    #[test]
    fn tray_tooltip_format() {
        let now = 1_000_000_000_000i64;
        let long = "x".repeat(100);
        let arts = vec![
            art("Dezeen", "Title"),
            art("A", &long),
            art("B", "T3"),
            art("C", "T4"),
        ];
        let t = tray_tooltip(2, Some(now - 300_000), &arts, now);
        let mut lines = t.split('\n');
        assert_eq!(lines.next().unwrap(), "Radar — 2 unread · last run 5m ago");
        assert_eq!(lines.next().unwrap(), "");
        assert_eq!(lines.next().unwrap(), "Dezeen — Title");
        let l2 = lines.next().unwrap();
        assert_eq!(l2.chars().count(), 72);
        assert!(l2.ends_with('…'));
        assert_eq!(lines.next().unwrap(), "B — T3");
        assert!(lines.next().is_none());
        let z = tray_tooltip(0, None, &[], now);
        assert_eq!(z, "Radar — all caught up · last run never");
        assert_eq!(tray_title(0), None);
        assert_eq!(tray_title(3), Some("3".into()));
    }

    #[test]
    fn truncate_basic() {
        assert_eq!(truncate("", 5), "");
        assert_eq!(truncate("abcde", 5), "abcde");
        assert_eq!(truncate("abcdef", 5), "abcd…");
    }

    #[test]
    fn format_run_at_today_yesterday_older_and_invalid() {
        let now = Local.with_ymd_and_hms(2026, 11, 5, 15, 0, 0).unwrap();
        let at = |y, mo, d, h, mi| {
            Local
                .with_ymd_and_hms(y, mo, d, h, mi, 0)
                .unwrap()
                .to_rfc3339()
        };
        assert_eq!(
            format_run_at(&at(2026, 11, 5, 9, 5), now),
            "Today · 9:05 AM"
        );
        assert_eq!(
            format_run_at(&at(2026, 11, 5, 0, 0), now),
            "Today · 12:00 AM"
        );
        assert_eq!(
            format_run_at(&at(2026, 11, 4, 23, 30), now),
            "Yesterday · 11:30 PM"
        );
        assert_eq!(
            format_run_at(&at(2026, 11, 3, 13, 0), now),
            "Tuesday, November 3 · 1:00 PM"
        );
        assert_eq!(
            format_run_at(&at(2025, 12, 31, 8, 9), now),
            "Wednesday, December 31 · 8:09 AM"
        );
        assert_eq!(format_run_at("not a date", now), "not a date");
    }
}
