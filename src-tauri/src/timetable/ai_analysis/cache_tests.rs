use super::*;
use crate::db::{AiScheduleItem, SnapshotState};

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn open() -> (Database, Self) {
        let dir = Self(
            std::env::temp_dir().join(format!("selah-ai-cache-reuse-{}", uuid::Uuid::new_v4())),
        );
        (Database::open(&dir.0).unwrap(), dir)
    }
    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.0.join("courses.db"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snapshot() -> SnapshotState {
    let today = chrono::Local::now().date_naive();
    let period = crate::academic_period::calendar_academic_period(today);
    SnapshotState {
        current_week_label: format!("{}～{}", today.format("%Y/%m/%d"), today.format("%Y/%m/%d")),
        next_week_label: format!(
            "{}～{}",
            (today + chrono::Duration::days(7)).format("%Y/%m/%d"),
            (today + chrono::Duration::days(13)).format("%Y/%m/%d")
        ),
        luna_year: period.as_ref().map(|p| p.year.clone()).unwrap_or_default(),
        luna_term: period.as_ref().map(|p| p.term.clone()).unwrap_or_default(),
        ..Default::default()
    }
}

#[test]
fn reused_snapshot_preserves_complete_legacy_ai_output_for_missing_empty_and_mismatched_weeks() {
    let (db, dir) = Temporary::open();
    let populated = snapshot();
    let scope = crate::academic_period::visible_weeks(
        &populated.current_week_label,
        &populated.next_week_label,
        &populated.luna_year,
        &populated.luna_term,
        chrono::Local::now().date_naive(),
    );
    let item = AiScheduleItem {
        day: 1,
        period: 2,
        course_name: "全文\n\"引用\" 👩🏽‍💻".repeat(128),
        delivery_mode: "対面".into(),
        room: "B201".into(),
        teacher: "先生".into(),
        session_topic: "全ての授業内容".repeat(64),
        is_cancelled: true,
        notifications: vec!["通知".repeat(64)],
        assignments: vec!["課題".repeat(64)],
        exams: vec!["試験".repeat(64)],
    };
    for saved in [None, Some(SnapshotState::default()), Some(populated)] {
        dir.sql("DELETE FROM schedule_snapshot_state; DELETE FROM ai_schedule_cache;");
        if let Some(state) = saved.as_ref() {
            db.save_snapshot_state(state).unwrap();
        }
        let snapshot = db.get_snapshot_state().unwrap();
        assert_eq!(
            serde_json::to_value(load_ai_cache_with_snapshot(&db, snapshot.as_ref()).unwrap())
                .unwrap(),
            serde_json::to_value(super::legacy_cache::load_ai_cache_before(&db).unwrap()).unwrap()
        );
        for (current, next) in [
            (scope.current.as_str(), scope.next.as_str()),
            ("2000/01/01～2000/01/07", scope.next.as_str()),
            (scope.current.as_str(), "2000/01/08～2000/01/14"),
            ("2000/01/01～2000/01/07", "2000/01/08～2000/01/14"),
            ("", ""),
        ] {
            for empty in [false, true] {
                for expired in [false, true] {
                    let ai = AiScheduleResult {
                        current_week_label: current.into(),
                        next_week_label: next.into(),
                        current_week: if empty { vec![] } else { vec![item.clone()] },
                        next_week: if empty { vec![] } else { vec![item.clone()] },
                        weekly_summary: "全体要約\n🌙".repeat(128),
                        cross_week_insights: "跨週の洞察".repeat(128),
                    };
                    db.save_ai_schedule_cache(&ai).unwrap();
                    if expired {
                        dir.sql("UPDATE ai_schedule_cache SET updated_at = 1;");
                    }
                    let before = super::legacy_cache::load_ai_cache_before(&db).unwrap();
                    let after = load_ai_cache_with_snapshot(&db, snapshot.as_ref()).unwrap();
                    assert_eq!(
                        serde_json::to_string(&after).unwrap(),
                        serde_json::to_string(&before).unwrap()
                    );
                    assert_eq!(
                        serde_json::to_string(&load_ai_cache(&db).unwrap()).unwrap(),
                        serde_json::to_string(&before).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn reused_snapshot_week_filter_uses_the_metadata_already_loaded_for_the_response() {
    let (db, _dir) = Temporary::open();
    let snapshot = snapshot();
    db.save_snapshot_state(&snapshot).unwrap();
    let scope = crate::academic_period::visible_weeks(
        &snapshot.current_week_label,
        &snapshot.next_week_label,
        &snapshot.luna_year,
        &snapshot.luna_term,
        chrono::Local::now().date_naive(),
    );
    let ai = AiScheduleResult {
        current_week_label: scope.current.clone(),
        next_week_label: scope.next.clone(),
        current_week: vec![AiScheduleItem {
            course_name: "元の週".into(),
            ..Default::default()
        }],
        next_week: vec![AiScheduleItem {
            course_name: "次週".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    db.save_ai_schedule_cache(&ai).unwrap();
    let saved = db.get_snapshot_state().unwrap();
    let expected = super::legacy_cache::load_ai_cache_before(&db).unwrap();
    assert!(expected.0.is_some());
    // A concurrent sync changes the database metadata after this response took
    // its snapshot. The AI filter must still use the response's original weeks.
    db.save_snapshot_state(&SnapshotState::default()).unwrap();
    let response = load_ai_cache_with_snapshot(&db, saved.as_ref()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert!(load_ai_cache(&db).unwrap().0.is_none());
}

#[test]
fn ai_parse_and_metadata_sql_errors_keep_the_existing_failure_paths() {
    let (db, dir) = Temporary::open();
    db.save_ai_schedule_cache(&AiScheduleResult::default())
        .unwrap();
    dir.sql("UPDATE ai_schedule_cache SET result_json = '{broken';");
    assert_eq!(
        load_ai_cache_with_snapshot(&db, None).unwrap_err(),
        super::legacy_cache::load_ai_cache_before(&db).unwrap_err()
    );
    dir.sql("DROP TABLE schedule_snapshot_state;");
    let before = super::legacy_cache::load_ai_cache_before(&db).unwrap_err();
    assert!(before.starts_with("DB snapshot read:"));
    assert_eq!(load_ai_cache(&db).unwrap_err(), before);
}
