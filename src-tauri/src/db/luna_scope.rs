pub fn epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Luna timetable ids are 16 digits. The academic year is the first four
/// digits and the term code is at indexes 12..14 (02 spring, 03 fall).
pub(crate) fn luna_id_year(luna_id: &str) -> Option<&str> {
    let year = luna_id.get(..4)?;
    year.chars().all(|c| c.is_ascii_digit()).then_some(year)
}

pub(crate) fn luna_id_term_code(luna_id: &str) -> Option<&str> {
    let code = luna_id.get(12..14)?;
    code.chars().all(|c| c.is_ascii_digit()).then_some(code)
}

fn plausible_academic_year(year: &str) -> bool {
    matches!(year.parse::<i32>(), Ok(value) if (2000..=2100).contains(&value))
}

/// Hide a stored row only when its id positively belongs to another semester.
/// Unrecognized ids, sentinel years such as 9999, and year-long codes stay.
/// An empty snapshot term must not hide the whole table.
pub(crate) fn luna_course_matches_snapshot(luna_id: &str, year: &str, term: &str) -> bool {
    if !year.is_empty() {
        if let Some(id_year) = luna_id_year(luna_id) {
            if plausible_academic_year(id_year) && id_year != year {
                return false;
            }
        }
    }
    if term.is_empty() {
        return true;
    }
    match luna_id_term_code(luna_id) {
        Some(code) if code == "02" || code == "03" => code == term,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_spring_luna_ids_when_snapshot_is_fall() {
        assert!(!luna_course_matches_snapshot(
            "2026340640010201",
            "2026",
            "03"
        ));
        assert!(luna_course_matches_snapshot(
            "2026340650010301",
            "2026",
            "03"
        ));
    }

    #[test]
    fn keeps_unrecognized_and_year_long_luna_ids() {
        assert!(luna_course_matches_snapshot(
            "2026340640010101",
            "2026",
            "03"
        ));
        assert!(luna_course_matches_snapshot("not-a-luna-id", "2026", "03"));
        assert!(luna_course_matches_snapshot("2026340640010201", "2026", ""));
    }

    #[test]
    fn hides_a_different_academic_year() {
        assert!(!luna_course_matches_snapshot(
            "2025340650010301",
            "2026",
            "03"
        ));
    }

    #[test]
    fn hides_fall_luna_ids_when_snapshot_is_spring() {
        assert!(!luna_course_matches_snapshot(
            "2026340650010301",
            "2026",
            "02"
        ));
        assert!(luna_course_matches_snapshot(
            "2026340640010201",
            "2026",
            "02"
        ));
        assert!(!luna_course_matches_snapshot(
            "2026340650010301",
            "2027",
            "02"
        ));
    }

    #[test]
    fn keeps_sentinel_community_ids_but_hides_previous_year() {
        assert!(luna_course_matches_snapshot(
            "9999CM9901659902",
            "2027",
            "02"
        ));
        assert!(!luna_course_matches_snapshot(
            "2026CM2600090102",
            "2027",
            "02"
        ));
    }
}
