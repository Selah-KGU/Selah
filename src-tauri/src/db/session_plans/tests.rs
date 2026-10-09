use super::*;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn database() -> (Temporary, Database) {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-course-plans-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    (temporary, db)
}
fn plan(num: i32) -> SessionPlanRow {
    SessionPlanRow {
        session_num: num,
        th_header: format!("第{num}回\n概要 🌕"),
        topic: format!("{num}: 引用 \"\\\n👩🏽‍💻 学修内容"),
        delivery_mode: if num % 2 == 0 { "online" } else { "offline" }.into(),
        study_outside: "完全な授業外学修　\n提出資料 🌕".repeat(num.max(1) as usize),
    }
}
fn legacy(db: &Database, code: &str) -> Option<Vec<SessionPlanRow>> {
    db.get_all_session_plans()
        .unwrap()
        .into_iter()
        .find(|(stored, _)| stored.trim() == code.trim())
        .map(|(_, rows)| rows)
}
fn value(rows: Option<Vec<SessionPlanRow>>) -> serde_json::Value {
    serde_json::to_value(rows).unwrap()
}

#[test]
fn targeted_read_matches_full_lookup_with_order_updates_full_text_and_parameterized_codes() {
    let (_temporary, db) = database();
    let target = "CODE' ; DROP TABLE session_plans; -- 🌕";
    let mut plans = (0..35).rev().map(plan).collect::<Vec<_>>();
    plans[0].topic = "全文を残す 👩🏽‍💻".repeat(5000);
    db.upsert_session_plans(target, &plans).unwrap();
    db.upsert_session_plans("unrelated", &[plan(999)]).unwrap();
    assert_eq!(
        value(db.get_session_plans_for_course(target).unwrap()),
        value(legacy(&db, target))
    );
    let first = db.get_session_plans_for_course(target).unwrap().unwrap();
    assert_eq!(first.len(), 35);
    assert_eq!(first[0].session_num, 0);
    assert_eq!(first[34].topic, plans[0].topic);
    db.upsert_session_plans(
        target,
        &[SessionPlanRow {
            topic: "updated topic".into(),
            ..plan(10)
        }],
    )
    .unwrap();
    assert_eq!(
        value(db.get_session_plans_for_course(target).unwrap()),
        value(legacy(&db, target))
    );
    assert!(db
        .get_session_plans_for_course("missing")
        .unwrap()
        .is_none());
    assert!(db
        .get_session_plans_for_course(" \t　\u{2003}")
        .unwrap()
        .is_none());
    assert_eq!(db.get_all_session_plans().unwrap().len(), 2);
}

#[test]
fn unicode_trim_aliases_are_retained_and_ambiguous_codes_have_a_stable_preference() {
    let (_temporary, db) = database();
    for (index, whitespace) in [
        " ", "\t", "\n", "\r", "\u{85}", "\u{a0}", "\u{2003}", "\u{202f}", "　",
    ]
    .into_iter()
    .enumerate()
    {
        let code = format!("ALIAS{index}");
        let stored = format!("{whitespace}{code}{whitespace}");
        db.upsert_session_plans(&stored, &[plan(index as i32)])
            .unwrap();
        let input = format!("　\t{code}\n");
        assert_eq!(
            value(db.get_session_plans_for_course(&input).unwrap()),
            value(legacy(&db, &input))
        );
        assert!(db
            .get_session_plans_for_course(&code.to_lowercase())
            .unwrap()
            .is_none());
    }
    for (code, num) in [("  DUP ", 1), ("\tDUP\n", 2), ("　DUP　", 3)] {
        db.upsert_session_plans(code, &[plan(num)]).unwrap();
    }
    for _ in 0..20 {
        assert_eq!(
            db.get_session_plans_for_course("DUP").unwrap().unwrap()[0].session_num,
            2
        );
    }
    db.upsert_session_plans("DUP", &[plan(4)]).unwrap();
    assert_eq!(
        db.get_session_plans_for_course("　DUP　").unwrap().unwrap()[0].session_num,
        4
    );
}

#[test]
fn malformed_target_rows_report_failure_while_unrelated_plan_bodies_are_not_decoded() {
    let (_temporary, db) = database();
    db.upsert_session_plans("target", &[plan(1), plan(2)])
        .unwrap();
    db.upsert_session_plans("unrelated", &[plan(1)]).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE session_plans SET topic=x'ff' WHERE kgc_code='unrelated'",
            [],
        )
        .unwrap();
    assert_eq!(
        db.get_session_plans_for_course("target")
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    db.conn.lock().unwrap().execute("UPDATE session_plans SET delivery_mode=x'ff' WHERE kgc_code='target' AND session_num=2", []).unwrap();
    assert!(db
        .get_session_plans_for_course("target")
        .unwrap_err()
        .starts_with("DB read course plan:"));
    db.upsert_session_plans("target", &[plan(2)]).unwrap();
    assert_eq!(
        db.get_session_plans_for_course("target")
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE session_plans;")
        .unwrap();
    // A cached statement can discover a dropped table when it is executed,
    // rather than when it is prepared. Both paths must propagate the failure.
    let error = db.get_session_plans_for_course("target").unwrap_err();
    assert!(error.starts_with("DB "), "{error}");
    assert!(error.contains("no such table: session_plans"), "{error}");
}

#[test]
fn course_query_uses_existing_index_without_a_sort_and_reopening_preserves_legacy_rows() {
    let (temporary, db) = database();
    db.upsert_session_plans("　CODE\t", &[plan(4), plan(2)])
        .unwrap();
    let before = value(db.get_session_plans_for_course("CODE").unwrap());
    {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {COURSE_PLANS}"))
            .unwrap();
        let details = stmt
            .query_map(params!["CODE"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .any(|s| s.contains("SEARCH session_plans") && s.contains("kgc_code=?")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .all(|s| !s.contains("TEMP B-TREE") && !s.contains("SCAN session_plans")),
            "{details:?}"
        );
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {PLAN_CODES}"))
            .unwrap();
        let details = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details.iter().any(|s| s.contains("COVERING INDEX")),
            "{details:?}"
        );
        assert!(
            details.iter().all(|s| !s.contains("TEMP B-TREE")),
            "{details:?}"
        );
    }
    drop(db);
    let db = Database::open(&temporary.0).unwrap();
    assert_eq!(
        before,
        value(db.get_session_plans_for_course("CODE").unwrap())
    );
    assert_eq!(db.get_all_session_plans().unwrap()[0].0, "　CODE\t");
}

#[test]
fn alias_lookup_and_rows_can_share_a_read_snapshot_across_an_external_write() {
    let (temporary, db) = database();
    db.upsert_session_plans("　CODE　", &[plan(1)]).unwrap();
    let external = Database::open(&temporary.0).unwrap();
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().unwrap();
    assert!(Database::query_course_session_plans(&tx, "CODE")
        .unwrap()
        .is_empty());
    external
        .upsert_session_plans(
            "　CODE　",
            &[SessionPlanRow {
                topic: "new alias topic".into(),
                ..plan(1)
            }],
        )
        .unwrap();
    external.upsert_session_plans("CODE", &[plan(9)]).unwrap();
    let alias: String = tx.query_row(PLAN_CODES, [], |row| row.get(0)).unwrap();
    assert_eq!(alias, "　CODE　");
    let old = Database::query_course_session_plans(&tx, &alias).unwrap();
    assert_eq!(old[0].topic, plan(1).topic);
    tx.commit().unwrap();
    drop(conn);
    assert_eq!(
        db.get_session_plans_for_course("CODE").unwrap().unwrap()[0].session_num,
        9
    );
}

#[test]
#[ignore = "manual comparison of complete DB reads, no UI/model work"]
fn benchmark_one_course_plan_read() {
    use std::time::Instant;
    for course_count in [16, 256] {
        let (_temporary, db) = database();
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            {
                let mut stmt = tx.prepare("INSERT INTO session_plans (kgc_code, session_num, th_header, topic, delivery_mode, study_outside, updated_at) VALUES (?1,?2,?3,?4,?5,?6,0)").unwrap();
                for course in 0..course_count {
                    for num in 1..=18 {
                        stmt.execute(params![
                            format!("CODE-{course:04}"),
                            num,
                            format!("第{num}回概要"),
                            "講義内容と課題の説明 🌕".repeat(10),
                            "offline",
                            "授業外の準備と提出方法".repeat(12)
                        ])
                        .unwrap();
                    }
                }
            }
            tx.commit().unwrap();
        }
        let input = format!("CODE-{:04}", course_count / 2);
        for alias in [false, true] {
            if alias {
                db.conn
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE session_plans SET kgc_code=?1 WHERE kgc_code=?2",
                        params![format!("　{input}　"), input],
                    )
                    .unwrap();
            }
            assert_eq!(
                value(legacy(&db, &input)),
                value(db.get_session_plans_for_course(&input).unwrap())
            );
            let mut old_times = Vec::new();
            let mut new_times = Vec::new();
            for turn in 0..9 {
                for old in if turn % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let start = Instant::now();
                    let rows = if old {
                        legacy(&db, &input)
                    } else {
                        db.get_session_plans_for_course(&input).unwrap()
                    };
                    assert_eq!(rows.as_ref().unwrap().len(), 18);
                    std::hint::black_box(rows);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if old {
                        old_times.push(elapsed);
                    } else {
                        new_times.push(elapsed);
                    }
                }
            }
            old_times.sort_by(f64::total_cmp);
            new_times.sort_by(f64::total_cmp);
            eprintln!("course_count={course_count}, unicode_alias={alias}: all plans {:.3}ms -> one course {:.3}ms", old_times[4], new_times[4]);
        }
    }
}
