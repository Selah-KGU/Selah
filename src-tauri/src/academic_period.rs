use chrono::{Datelike, NaiveDate};
use regex::Regex;
use std::sync::LazyLock;

/// The semester the timetable should show. `end` is exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcademicPeriod {
    pub year: String,
    pub term: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

pub(crate) struct VisibleWeeks {
    pub current: String,
    pub next: String,
    pub year: String,
    pub term: String,
    pub from_calendar: bool,
    pub hid_current: bool,
}

pub(crate) fn calendar_academic_period(today: NaiveDate) -> Option<AcademicPeriod> {
    let calendar = crate::commands::load_calendar_config();
    expected_academic_period(today, &calendar.spring_start, &calendar.fall_start)
}

/// Calendar month-days recur each year. Fall runs until the next spring start,
/// not a fixed 200-day window, so a new spring does not keep the previous fall.
pub(crate) fn expected_academic_period(
    today: NaiveDate,
    spring_start: &str,
    fall_start: &str,
) -> Option<AcademicPeriod> {
    let spring = NaiveDate::parse_from_str(spring_start, "%Y-%m-%d").ok();
    let fall = NaiveDate::parse_from_str(fall_start, "%Y-%m-%d").ok();
    match (spring, fall) {
        (Some(spring), Some(fall)) if spring <= fall => {
            period_from_recurring_bounds(today, spring, fall)
        }
        (Some(spring), None) if today >= spring && today < spring + chrono::Duration::days(150) => {
            Some(AcademicPeriod {
                year: spring.format("%Y").to_string(),
                term: "02".to_string(),
                start: spring,
                end: spring + chrono::Duration::days(150),
            })
        }
        (None, Some(fall)) if today >= fall && today < fall + chrono::Duration::days(200) => {
            Some(AcademicPeriod {
                year: fall.format("%Y").to_string(),
                term: "03".to_string(),
                start: fall,
                end: fall + chrono::Duration::days(200),
            })
        }
        _ => None,
    }
}

fn period_from_recurring_bounds(
    today: NaiveDate,
    spring: NaiveDate,
    fall: NaiveDate,
) -> Option<AcademicPeriod> {
    let spring_month = spring.month();
    let spring_day = spring.day();
    let fall_month = fall.month();
    let fall_day = fall.day();
    let last_year = (spring.year() + 6).max(today.year() + 1);
    for year in spring.year()..=last_year {
        let Some(spring_y) = NaiveDate::from_ymd_opt(year, spring_month, spring_day) else {
            continue;
        };
        let Some(fall_y) = NaiveDate::from_ymd_opt(year, fall_month, fall_day) else {
            continue;
        };
        if spring_y > fall_y {
            continue;
        }
        let Some(next_spring) = NaiveDate::from_ymd_opt(year + 1, spring_month, spring_day) else {
            continue;
        };
        if today >= spring_y && today < fall_y {
            return Some(AcademicPeriod {
                year: year.to_string(),
                term: "02".to_string(),
                start: spring_y,
                end: fall_y,
            });
        }
        if today >= fall_y && today < next_spring {
            return Some(AcademicPeriod {
                year: year.to_string(),
                term: "03".to_string(),
                start: fall_y,
                end: next_spring,
            });
        }
        if today < spring_y {
            break;
        }
    }
    None
}

pub(crate) fn visible_weeks(
    current: &str,
    next: &str,
    snapshot_year: &str,
    snapshot_term: &str,
    today: NaiveDate,
) -> VisibleWeeks {
    let Some(period) = calendar_academic_period(today) else {
        return VisibleWeeks {
            current: current.to_string(),
            next: next.to_string(),
            year: snapshot_year.to_string(),
            term: snapshot_term.to_string(),
            from_calendar: false,
            hid_current: false,
        };
    };
    let current_visible =
        visible_week_label(current, Some(&period), snapshot_year, snapshot_term, today);
    let next_visible = visible_week_label(next, Some(&period), snapshot_year, snapshot_term, today);
    VisibleWeeks {
        current: current_visible.clone(),
        next: next_visible,
        year: period.year,
        term: period.term,
        from_calendar: true,
        hid_current: !current.trim().is_empty() && current_visible.is_empty(),
    }
}

pub(crate) fn visible_week_label(
    label: &str,
    period: Option<&AcademicPeriod>,
    snapshot_year: &str,
    snapshot_term: &str,
    today: NaiveDate,
) -> String {
    let Some(period) = period else {
        return label.to_string();
    };
    if label.trim().is_empty() {
        return String::new();
    }
    if let Some((start, end)) = week_label_range(label) {
        if end < period.start || start >= period.end {
            return String::new();
        }
        // A week that began before this semester and has already finished is
        // the previous semester's trailing week, not this semester's cache.
        if start < period.start && end < today {
            return String::new();
        }
        return label.to_string();
    }
    if snapshot_year == period.year && snapshot_term == period.term {
        return label.to_string();
    }
    String::new()
}

/// Cached analysis belongs on screen only when both labels are present and equal.
pub(crate) fn week_belongs_to_visible_label(shown: &str, cached: &str) -> bool {
    let shown = shown.trim();
    let cached = cached.trim();
    !shown.is_empty() && !cached.is_empty() && shown == cached
}

pub(crate) fn week_label_range(label: &str) -> Option<(NaiveDate, NaiveDate)> {
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    for sep in ['～', '〜', '~'] {
        let Some((left, right)) = label.split_once(sep) else {
            continue;
        };
        let start = parse_flexible_date(left)?;
        let end = parse_flexible_date(right)?;
        if end < start {
            return None;
        }
        return Some((start, end));
    }
    None
}

fn parse_flexible_date(text: &str) -> Option<NaiveDate> {
    static DATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\d{4})\D*(\d{1,2})\D*(\d{1,2})").unwrap());
    let caps = DATE_RE.captures(text.trim())?;
    let year: i32 = caps.get(1)?.as_str().parse().ok()?;
    let month: u32 = caps.get(2)?.as_str().parse().ok()?;
    let day: u32 = caps.get(3)?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(today: NaiveDate) -> AcademicPeriod {
        expected_academic_period(today, "2026-04-03", "2026-09-21").unwrap()
    }

    #[test]
    fn fall_after_configured_start_is_term_03() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2026", "03"));
    }

    #[test]
    fn spring_before_fall_is_term_02() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2026", "02"));
    }

    #[test]
    fn missing_calendar_does_not_guess() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        assert!(expected_academic_period(today, "", "").is_none());
    }

    #[test]
    fn before_configured_spring_does_not_invent_a_previous_year() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        assert!(expected_academic_period(today, "2026-04-03", "2026-09-21").is_none());
    }

    #[test]
    fn day_before_next_spring_stays_previous_fall() {
        let today = NaiveDate::from_ymd_opt(2027, 4, 2).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2026", "03"));
    }

    #[test]
    fn next_spring_with_last_year_calendar_is_term_02() {
        let today = NaiveDate::from_ymd_opt(2027, 4, 10).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2027", "02"));
        assert_eq!(got.start, NaiveDate::from_ymd_opt(2027, 4, 3).unwrap());
    }

    #[test]
    fn first_week_of_next_spring_is_not_still_fall() {
        let today = NaiveDate::from_ymd_opt(2027, 4, 8).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2027", "02"));
    }

    #[test]
    fn winter_break_stays_previous_fall() {
        let today = NaiveDate::from_ymd_opt(2027, 1, 20).unwrap();
        let got = period(today);
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2026", "03"));
    }

    #[test]
    fn updated_calendar_still_selects_that_spring() {
        let today = NaiveDate::from_ymd_opt(2027, 5, 1).unwrap();
        let got = expected_academic_period(today, "2027-04-05", "2027-09-21").unwrap();
        assert_eq!((got.year.as_str(), got.term.as_str()), ("2027", "02"));
    }

    #[test]
    fn fall_week_is_hidden_once_spring_starts() {
        let spring = period(NaiveDate::from_ymd_opt(2027, 4, 10).unwrap());
        let today = NaiveDate::from_ymd_opt(2027, 4, 10).unwrap();
        assert!(
            visible_week_label("2027/01/18～2027/01/24", Some(&spring), "2026", "03", today)
                .is_empty()
        );
        assert_eq!(
            visible_week_label("2027/04/06～2027/04/11", Some(&spring), "2026", "03", today),
            "2027/04/06～2027/04/11"
        );
        assert!(visible_week_label(
            "2027年01月18日～2027年01月24日",
            Some(&spring),
            "2026",
            "03",
            today
        )
        .is_empty());
    }

    #[test]
    fn finished_boundary_week_does_not_stay_into_spring() {
        let during = NaiveDate::from_ymd_opt(2027, 4, 4).unwrap();
        let spring = period(during);
        assert_eq!(
            visible_week_label(
                "2027/03/30～2027/04/04",
                Some(&spring),
                "2026",
                "03",
                during
            ),
            "2027/03/30～2027/04/04"
        );
        let later = NaiveDate::from_ymd_opt(2027, 4, 10).unwrap();
        let spring = period(later);
        assert!(
            visible_week_label("2027/03/30～2027/04/04", Some(&spring), "2026", "03", later)
                .is_empty()
        );
    }

    #[test]
    fn ai_week_must_match_the_visible_label() {
        assert!(!week_belongs_to_visible_label("", "2026/09/28～2026/10/03"));
        assert!(!week_belongs_to_visible_label("", ""));
        assert!(!week_belongs_to_visible_label("2027/04/06～2027/04/11", ""));
        assert!(week_belongs_to_visible_label(
            "2027/04/06～2027/04/11",
            "2027/04/06～2027/04/11"
        ));
    }

    #[test]
    fn same_semester_week_stays_when_calendar_cannot_tell() {
        assert_eq!(
            visible_week_label(
                "2026/09/28～2026/10/03",
                None,
                "2026",
                "03",
                NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
            ),
            "2026/09/28～2026/10/03"
        );
    }
}
