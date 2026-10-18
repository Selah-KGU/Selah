use crate::db::{KgcCourseRow, LunaCourseRow};
use unicode_normalization::UnicodeNormalization;

pub(crate) struct PrimaryCourse<'a> {
    pub day: i32,
    pub period: i32,
    pub name: &'a str,
    pub room: &'a str,
    pub is_cancelled: bool,
}

fn names_match(left: &str, right: &str) -> bool {
    let normalize = |name: &str| {
        name.nfkc()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    let name = normalize(left);
    !name.is_empty() && name == normalize(right)
}

/// Resolve native consumers using the same Luna-first rule as the timetable UI.
pub(crate) fn primary_courses<'a>(
    luna: &'a [LunaCourseRow],
    kgc: &'a [KgcCourseRow],
) -> Vec<PrimaryCourse<'a>> {
    let mut slots = std::collections::BTreeSet::new();
    let mut courses = Vec::new();
    for course in luna {
        if !slots.insert((course.day, course.period)) {
            continue;
        }
        let extra = kgc.iter().find(|extra| {
            extra.day == course.day
                && extra.period == course.period
                && names_match(&extra.name, &course.name)
        });
        courses.push(PrimaryCourse {
            day: course.day,
            period: course.period,
            name: &course.name,
            room: extra.map_or("", |extra| extra.room.as_str()),
            is_cancelled: extra.is_some_and(|extra| extra.is_cancelled),
        });
    }
    for course in kgc {
        if slots.contains(&(course.day, course.period))
            || (!course.is_makeup
                && luna
                    .iter()
                    .any(|primary| names_match(&primary.name, &course.name)))
        {
            continue;
        }
        slots.insert((course.day, course.period));
        courses.push(PrimaryCourse {
            day: course.day,
            period: course.period,
            name: &course.name,
            room: &course.room,
            is_cancelled: course.is_cancelled,
        });
    }
    courses.sort_by_key(|course| (course.day, course.period));
    courses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_use_the_same_unicode_normalization_as_the_frontend() {
        assert!(names_match("ＡＩ　入門", "AI入門"));
        assert!(names_match("ﾃﾞｰﾀ分析", "データ分析"));
        assert!(!names_match("", ""));
    }

    #[test]
    fn primary_courses_keep_luna_and_only_apply_matching_secondary_metadata() {
        let luna: Vec<LunaCourseRow> = serde_json::from_value(serde_json::json!([
            {"id":1,"luna_id":"current","name":"Current","teacher":"Teacher","day":1,"period":1},
            {"id":2,"luna_id":"moved","name":"Moved","teacher":"Teacher","day":2,"period":2}
        ]))
        .unwrap();
        let kgc: Vec<KgcCourseRow> = serde_json::from_value(serde_json::json!([
            {"id":1,"kgc_code":"old","name":"Old","day":1,"period":1,"room":"Wrong","detail_path":"","is_cancelled":true,"is_makeup":false,"is_room_changed":false,"week_label":"current"},
            {"id":2,"kgc_code":"moved","name":"Moved","day":1,"period":2,"room":"Old","detail_path":"","is_cancelled":false,"is_makeup":false,"is_room_changed":false,"week_label":"current"},
            {"id":3,"kgc_code":"moved","name":"Moved","day":2,"period":2,"room":"Correct","detail_path":"","is_cancelled":true,"is_makeup":false,"is_room_changed":false,"week_label":"current"}
        ])).unwrap();
        let courses = primary_courses(&luna, &kgc);
        assert_eq!(courses.len(), 2);
        assert_eq!(courses[0].name, "Current");
        assert_eq!(courses[0].room, "");
        assert!(!courses[0].is_cancelled);
        assert_eq!(courses[1].room, "Correct");
        assert!(courses[1].is_cancelled);
    }
}
