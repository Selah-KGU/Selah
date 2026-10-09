use super::*;
use serde_json::{json, Value};
use std::sync::Mutex;

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn database() -> (Database, Temporary) {
    let dir =
        Temporary(std::env::temp_dir().join(format!("selah-raw-scope-{}", uuid::Uuid::new_v4())));
    (Database::open(&dir.0).unwrap(), dir)
}
fn canonical(mut raw: ScheduleRawData) -> Value {
    // The legacy HashMap has no group order. Keep every other array in its
    // original order, including session numbers, activity ties and detail rows.
    raw.session_plans.sort_by(|a, b| a.0.cmp(&b.0));
    serde_json::to_value(raw).unwrap()
}
fn compare(db: &Database, current: &str, next: &str) {
    let communities = vec![crate::luna_parser::LunaCommunity {
        idnumber: "9999CM9901659902".into(),
        name: "社区 🌙".into(),
    }];
    assert_eq!(
        canonical(
            db.build_raw_data(current, next, communities.clone())
                .unwrap()
        ),
        canonical(
            db.build_raw_data_before(current, next, communities)
                .unwrap()
        )
    );
}
fn seed(db: &Database, history: usize, repeats: usize) -> (String, String) {
    db.save_snapshot_state(&SnapshotState {
        luna_year: "2026".into(),
        luna_term: "03".into(),
        ..Default::default()
    })
    .unwrap();
    let mut conn = db.conn.lock().unwrap();
    let (year, term) = Database::effective_luna_scope(&conn);
    let current_id = format!("{year}12345678{term}01");
    let past_year = if year == "2000" { "2001" } else { "2000" };
    let text = "全文\n\"引用\" \\ 👩🏽‍💻 中文 日本語".repeat(repeats);
    let tx = conn.transaction().unwrap();
    for index in (0..history + 2).rev() {
        let code = match index {
            0 => "课程'🌙".into(),
            1 => "　 target　".into(),
            _ => format!("old-{index:06}"),
        };
        let label = if index < 2 { "current" } else { "old-week" };
        tx.execute(
            "INSERT INTO kgc_courses (kgc_code,name,day,period,week_label) VALUES (?1,?2,1,1,?3)",
            params![code, text, label],
        )
        .unwrap();
        for number in [3, 1, 0, 2] {
            tx.execute("INSERT INTO session_plans (kgc_code,session_num,th_header,topic,delivery_mode,study_outside) VALUES (?1,?2,?3,?4,'offline',?4)", params![code, number, format!("第{number}回"), text]).unwrap();
        }
        let fields = serde_json::to_string(&vec![("授業概要", format!("{code}: {text}"))]).unwrap();
        let textbooks = json!([{"category":"教科書","title":text,"author":"作者 🌙","publisher":"出版社","year":"2026","isbn":"123","text":text}]).to_string();
        tx.execute("INSERT INTO kgc_course_details (kgc_code,fields_json,delivery_mode,textbooks_json) VALUES (?1,?2,'online',?3)", params![code, fields, textbooks]).unwrap();
        let id = if index < 2 {
            format!("{year}{index:08}{term}01")
        } else {
            format!("{past_year}{index:08}0301")
        };
        tx.execute(
            "INSERT INTO luna_counts (luna_id,announcements,reports) VALUES (?1,?2,?2)",
            params![id, index as i32],
        )
        .unwrap();
        for kind in ["report", "announcement", "report"] {
            tx.execute("INSERT INTO luna_activities (luna_id,activity_type,title,period,status,detail_path) VALUES (?1,?2,?3,'明日','未提出',?4)", params![id, kind, text, format!("/contents/{index}'🌙")]).unwrap();
        }
    }
    tx.execute("INSERT INTO kgc_courses (kgc_code,name,day,period,week_label) VALUES ('课程''🌙','次週',2,2,'next')", []).unwrap();
    // Same code also appears in another period without duplicating its plans.
    tx.execute("INSERT INTO kgc_courses (kgc_code,name,day,period,week_label) VALUES ('课程''🌙','別コマ',3,3,'current')", []).unwrap();
    for id in [
        &current_id,
        "not-a-luna-id",
        "9999CM9901659902",
        &format!("{year}123456780101"),
        &format!("{year}👩🏽‍💻0301"),
        "课程\0🌙",
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO luna_counts (luna_id,reports) VALUES (?1,9)",
            params![id],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO luna_activities (luna_id,activity_type,title) VALUES (?1,'report',?2)",
            params![id, text],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    (year, term)
}

#[test]
fn scoped_rows_match_complete_legacy_data_with_unicode_ids_week_changes_and_empty_labels() {
    let (db, _dir) = database();
    seed(&db, 48, 128);
    for (current, next) in [
        ("current", "next"),
        ("next", "current"),
        ("current", "current"),
        ("old-week", "next"),
        ("", "current"),
        ("current", ""),
        ("", ""),
        ("　\t", "\n"),
        ("missing'🌙", "none"),
    ] {
        compare(&db, current, next);
    }
}

#[test]
fn chunked_key_reads_preserve_row_order_and_all_unknown_luna_ids() {
    let (db, _dir) = database();
    seed(&db, 520, 1);
    let conn = db.conn.lock().unwrap();
    // More than two parameter batches for both KGC and Luna, with reversed
    // insertion order and activity ties to detect ordering changes.
    for index in (0..530).rev() {
        conn.execute(
            "INSERT INTO luna_counts (luna_id,reports) VALUES (?1,?2)",
            params![format!("legacy-{index:06}"), index],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO luna_activities (luna_id,activity_type,title) VALUES (?1,'report',?2)",
            params![format!("legacy-{index:06}"), format!("{index} 🌙")],
        )
        .unwrap();
    }
    drop(conn);
    compare(&db, "old-week", "current");
    let raw = db.build_raw_data("old-week", "current", vec![]).unwrap();
    assert_eq!(raw.session_plans.len(), 522);
    assert!(raw.luna_counts.len() >= 530);
    assert_eq!(raw.kgc_course_details.len(), 522);
}

#[test]
fn malformed_visible_and_hidden_rows_keep_legacy_fallbacks_without_losing_valid_neighbors() {
    let (db, _dir) = database();
    seed(&db, 12, 2);
    db.conn.lock().unwrap().execute_batch("UPDATE session_plans SET topic=x'ff' WHERE session_num=1;
        UPDATE kgc_course_details SET fields_json='{broken', textbooks_json='null' WHERE kgc_code='课程''🌙';
        UPDATE kgc_course_details SET delivery_mode=x'ff' WHERE kgc_code='　 target　';
        UPDATE luna_counts SET reports='bad' WHERE announcements=1;
        UPDATE luna_activities SET title=x'ff' WHERE id=1;
        INSERT INTO kgc_courses (kgc_code,name,day,period,week_label) VALUES ('invalid-course-row',x'ff',1,1,'current');
        INSERT INTO session_plans (kgc_code,session_num,topic) VALUES ('invalid-course-row',1,'hidden because its timetable row fails decoding');").unwrap();
    compare(&db, "current", "next");
    compare(&db, "old-week", "current");
}

#[test]
fn empty_key_reads_still_report_missing_tables_and_projection_columns() {
    for table in [
        "session_plans",
        "luna_counts",
        "luna_activities",
        "kgc_course_details",
    ] {
        let (db, _dir) = database();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(&format!("DROP TABLE {table}"))
            .unwrap();
        assert_eq!(
            db.build_raw_data("", "", vec![]).unwrap_err(),
            db.build_raw_data_before("", "", vec![]).unwrap_err()
        );
    }
    let (db, _dir) = database();
    db.conn
        .lock()
        .unwrap()
        .execute_batch("ALTER TABLE luna_activities DROP COLUMN title")
        .unwrap();
    // SQLite includes the actual SQL and character offset in this error. The
    // new projection has a different offset but must preserve its cause.
    for error in [
        db.build_raw_data("", "", vec![]).unwrap_err(),
        db.build_raw_data_before("", "", vec![]).unwrap_err(),
    ] {
        assert!(
            error.starts_with("DB query: no such column: title"),
            "{error}"
        );
        assert!(error.contains("luna_activities"));
    }
}

#[test]
fn luna_scope_readers_keep_empty_terms_unknown_years_and_byte_indexed_unicode_ids() {
    let (db, _dir) = database();
    let (year, term) = seed(&db, 16, 2);
    let conn = db.conn.lock().unwrap();
    let all_counts = Database::query_all_luna_counts(&conn).unwrap();
    let all_activities = Database::query_all_luna_activities(&conn).unwrap();
    for year in ["", "2000", "9999", year.as_str()] {
        for term in ["", "02", "03", "01", term.as_str()] {
            let expected_counts: Vec<_> = all_counts
                .iter()
                .filter(|(id, _)| luna_course_matches_snapshot(id, year, term))
                .collect();
            let expected_activities: Vec<_> = all_activities
                .iter()
                .filter(|row| luna_course_matches_snapshot(&row.luna_id, year, term))
                .collect();
            assert_eq!(
                serde_json::to_value(
                    Database::query_visible_luna_counts(&conn, year, term).unwrap()
                )
                .unwrap(),
                serde_json::to_value(expected_counts).unwrap()
            );
            assert_eq!(
                serde_json::to_value(
                    Database::query_visible_luna_activities(&conn, year, term).unwrap()
                )
                .unwrap(),
                serde_json::to_value(expected_activities).unwrap()
            );
        }
    }
}

struct Profile<'a> {
    db: &'a Database,
    rows: Box<Mutex<Vec<(String, i32)>>>,
}
impl<'a> Profile<'a> {
    fn observe(db: &'a Database) -> Self {
        let mut profile = Self {
            db,
            rows: Box::default(),
        };
        let conn = db.conn.lock().unwrap();
        // SAFETY: callback storage is boxed and stable; Drop unregisters it
        // under the connection mutex before freeing it.
        let status = unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                conn.handle(),
                rusqlite::ffi::SQLITE_TRACE_PROFILE as u32,
                Some(Self::record),
                profile.rows.as_mut() as *mut _ as *mut std::ffi::c_void,
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        profile
    }
    unsafe extern "C" fn record(
        _: u32,
        context: *mut std::ffi::c_void,
        statement: *mut std::ffi::c_void,
        _: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        if !context.is_null() && !statement.is_null() {
            // SAFETY: SQLite supplies a live statement during PROFILE callbacks.
            let statement = statement.cast::<rusqlite::ffi::sqlite3_stmt>();
            let sql = unsafe { rusqlite::ffi::sqlite3_sql(statement) };
            if !sql.is_null() {
                let sql = unsafe { std::ffi::CStr::from_ptr(sql) }.to_string_lossy();
                if sql.starts_with("SELECT ")
                    && ["topic", "fields_json", "announcements", "title"]
                        .iter()
                        .any(|column| sql.contains(column))
                {
                    let steps = unsafe {
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,
                            0,
                        )
                    };
                    let storage = unsafe { &*context.cast::<Mutex<Vec<(String, i32)>>>() };
                    if let Ok(mut rows) = storage.lock() {
                        rows.push((sql.into_owned(), steps));
                    }
                }
            }
        }
        0
    }
    fn clear(&self) {
        self.rows.lock().unwrap().clear();
    }
    fn rows(&self) -> Vec<(String, i32)> {
        self.rows.lock().unwrap().clone()
    }
}
impl Drop for Profile<'_> {
    fn drop(&mut self) {
        let conn = self.db.conn.lock().unwrap();
        // SAFETY: connection exclusion ensures no callback outlives its storage.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(conn.handle(), 0, None, std::ptr::null_mut());
        }
    }
}

#[test]
fn body_queries_seek_visible_keys_and_id_metadata_uses_covering_indexes() {
    let (db, _dir) = database();
    seed(&db, 192, 4);
    let profile = Profile::observe(&db);
    db.build_raw_data_before("current", "next", vec![]).unwrap();
    let old = profile.rows();
    assert_eq!(old.len(), 4);
    assert!(old.iter().map(|(_, steps)| *steps).sum::<i32>() > 1000);
    profile.clear();
    db.build_raw_data("current", "next", vec![]).unwrap();
    let new = profile.rows();
    assert_eq!(new.len(), 4);
    assert_eq!(
        new.iter().map(|(_, steps)| *steps).sum::<i32>(),
        0,
        "{new:?}"
    );
    let conn = db.conn.lock().unwrap();
    for table in ["luna_counts", "luna_activities"] {
        let mut stmt = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT DISTINCT luna_id FROM {table}"
            ))
            .unwrap();
        let details: Vec<String> = stmt
            .query_map([], |row| row.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("COVERING INDEX")),
            "{table}: {details:?}"
        );
    }
}

#[test]
#[ignore = "isolated synthetic raw-data timing, run explicitly"]
fn benchmark_raw_scope_with_historical_bodies() {
    fn median(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    }
    let mut reports = Vec::new();
    for history in [0, 32, 256] {
        let (db, _dir) = database();
        seed(&db, history, 64);
        let expected = canonical(db.build_raw_data_before("current", "next", vec![]).unwrap());
        let mut before = Vec::new();
        let mut after = Vec::new();
        for index in 0..25 {
            for old in if index % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                let started = std::time::Instant::now();
                let raw = if old {
                    db.build_raw_data_before("current", "next", vec![]).unwrap()
                } else {
                    db.build_raw_data("current", "next", vec![]).unwrap()
                };
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(canonical(raw), expected);
                if index >= 4 {
                    if old {
                        before.push(elapsed);
                    } else {
                        after.push(elapsed);
                    }
                }
            }
        }
        reports.push(json!({"historical_courses":history,"visible_kgc_courses":2,"before_ms":median(before),"after_ms":median(after)}));
    }
    let report = json!({"scope":"isolated temporary SQLite DB; raw-data read/parse/filter/assembly only; outer JSON encoding and DTO disposal excluded; no real app RSS, WebKit or GPU", "profile":"debug", "warmups":4,"samples":21,"results":reports});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("SELAH_RAW_SCOPE_BENCHMARK") {
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap() + "\n").unwrap();
    }
}
