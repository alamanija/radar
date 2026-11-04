//! Pure schedule logic: decides whether the daily briefing slot is due.

use crate::types::Prefs;
use chrono::{DateTime, Local, NaiveTime, TimeZone};

/// Parse `HH:MM` (24h) into `(hour, minute)`.
pub fn parse_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

fn slot_for(now: DateTime<Local>, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    Local
        .from_local_datetime(&now.date_naive().and_time(time))
        .single()
}

/// True when scheduling is enabled, today's slot has passed, and no run has
/// happened at or after that slot.
pub fn should_fire(
    prefs: &Prefs,
    last_run_at: Option<DateTime<Local>>,
    now: DateTime<Local>,
) -> bool {
    if !prefs.schedule_enabled {
        return false;
    }
    let Some((h, m)) = parse_time(&prefs.schedule_time) else {
        return false;
    };
    let Some(slot) = slot_for(now, h, m) else {
        return false;
    };
    if now < slot {
        return false;
    }
    !last_run_at.is_some_and(|t| t >= slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        Local
            .from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap())
            .single()
            .unwrap()
    }

    fn prefs(enabled: bool, time: &str) -> Prefs {
        Prefs {
            schedule_enabled: enabled,
            schedule_time: time.to_string(),
            ..Prefs::default()
        }
    }

    #[test]
    fn should_fire_disabled_is_false() {
        assert!(!should_fire(&prefs(false, "07:30"), None, at(9, 0)));
    }

    #[test]
    fn should_fire_before_slot_is_false() {
        assert!(!should_fire(&prefs(true, "07:30"), None, at(7, 0)));
    }

    #[test]
    fn should_fire_after_slot_with_stale_last_run_is_true() {
        let last = at(7, 30) - Duration::hours(1);
        assert!(should_fire(&prefs(true, "07:30"), Some(last), at(9, 0)));
        assert!(should_fire(&prefs(true, "07:30"), None, at(9, 0)));
    }

    #[test]
    fn should_fire_after_slot_with_fresh_last_run_is_false() {
        assert!(!should_fire(&prefs(true, "07:30"), Some(at(8, 0)), at(9, 0)));
    }

    #[test]
    fn should_fire_bad_time_is_false() {
        assert!(!should_fire(&prefs(true, "25:00"), None, at(9, 0)));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("07:30"), Some((7, 30)));
    }
}
