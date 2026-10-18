use super::*;
use crate::db::{AiScheduleItem, KgcCourseRow, SnapshotState};

#[test]
fn offline_snapshot_advances_on_saturday_without_refreshing_courses_or_reusing_old_week_status() {
    let directory =
        std::env::temp_dir().join(format!("selah-week-rollover-{}", uuid::Uuid::new_v4()));
    let db = Database::open(&directory).unwrap();
    let old = "2026/10/12～2026/10/17";
    let upcoming = "2026/10/19～2026/10/24";
    db.save_snapshot_state(&SnapshotState {
        current_week_label: old.into(),
        next_week_label: upcoming.into(),
        luna_year: "2026".into(),
        luna_term: "03".into(),
        ..Default::default()
    })
    .unwrap();
    db.replace_luna_courses(&[luna_parser::LunaCourse {
        idnumber: "local-course".into(),
        name: "Luna course".into(),
        teacher: "Teacher".into(),
        day: 1,
        period: 1,
    }])
    .unwrap();
    for (label, cancelled, room) in [(old, true, "Old room"), (upcoming, false, "Next room")] {
        db.replace_kgc_week(
            label,
            &[KgcCourseRow {
                id: 0,
                kgc_code: "code".into(),
                name: "Luna course".into(),
                day: 1,
                period: 1,
                room: room.into(),
                detail_path: String::new(),
                is_cancelled: cancelled,
                is_makeup: false,
                is_room_changed: false,
                week_label: label.into(),
            }],
        )
        .unwrap();
    }
    db.save_ai_schedule_cache(&AiScheduleResult {
        current_week_label: old.into(),
        next_week_label: upcoming.into(),
        current_week: vec![AiScheduleItem {
            course_name: "Luna course".into(),
            is_cancelled: true,
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    let conn = rusqlite::Connection::open(directory.join("courses.db")).unwrap();
    conn.execute_batch("UPDATE schedule_snapshot_state SET updated_at = 123; UPDATE luna_courses SET updated_at = 100;").unwrap();
    let saved = serde_json::to_value(db.get_snapshot_state().unwrap()).unwrap();
    let friday = chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    let response = build_schedule_snapshot_on(&db, friday).unwrap();
    assert_eq!(response.raw.current_week_label, old);
    assert!(response.raw.kgc_entries_current[0].is_cancelled);
    assert!(response.ai_result.is_some());
    for day in [17, 18, 19] {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
        let response = build_schedule_snapshot_on(&db, today).unwrap();
        assert_eq!(response.raw.current_week_label, upcoming);
        assert_eq!(
            response.raw.next_week_label,
            "2026年10月26日～2026年10月31日"
        );
        assert_eq!(response.raw.luna_courses[0].name, "Luna course");
        assert_eq!(response.raw.kgc_entries_current.len(), 1);
        assert_eq!(response.raw.kgc_entries_current[0].room, "Next room");
        assert!(!response.raw.kgc_entries_current[0].is_cancelled);
        assert!(response.raw.kgc_entries_next.is_empty());
        assert!(response.ai_result.is_none());
        assert_eq!(response.snapshot_updated_at, 123);
        assert_eq!(
            serde_json::to_value(db.get_snapshot_state().unwrap()).unwrap(),
            saved
        );
    }
    // Several missed refreshes still advance from today's date, rather than
    // advancing just once from the last cached next week.
    let later = build_schedule_snapshot_on(&db, friday + chrono::Duration::days(15)).unwrap();
    assert_eq!(
        later.raw.current_week_label,
        "2026年11月02日～2026年11月07日"
    );
    assert!(later.raw.kgc_entries_current.is_empty());
    assert_eq!(later.raw.luna_courses.len(), 1);
    assert_eq!(later.snapshot_updated_at, 123);
    let course_timestamp: i64 = conn
        .query_row("SELECT updated_at FROM luna_courses LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(course_timestamp, 100);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_dir_all(directory);
}
