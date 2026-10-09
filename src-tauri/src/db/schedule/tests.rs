use super::*;
use serde_json::json;

struct Temporary(PathBuf);
impl Temporary {
    fn open() -> (Database, Self) {
        let dir = Self(
            std::env::temp_dir().join(format!("selah-schedule-read-{}", uuid::Uuid::new_v4())),
        );
        (Database::open(&dir.0).unwrap(), dir)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state() -> SnapshotState {
    serde_json::from_value(json!({
        "current_week_label":"今週", "next_week_label":"来週",
        "luna_year":"2026", "luna_term":"03",
        "luna_communities":[],
        "luna_year_options":[{"value":"2026","label":"2026年度 🌙","selected":true}],
        "luna_term_options":[{"value":"03","label":"秋学期","selected":true}],
        "updated_at":0
    }))
    .unwrap()
}

#[test]
fn absent_rows_are_empty_and_saved_snapshot_and_ai_round_trip() {
    let (db, _dir) = Temporary::open();
    assert!(db.get_snapshot_state().unwrap().is_none());
    assert!(db.get_ai_schedule_cache().unwrap().is_none());
    let mut expected = state();
    db.save_snapshot_state(&expected).unwrap();
    let loaded = db.get_snapshot_state().unwrap().unwrap();
    assert!(loaded.updated_at > 0);
    expected.updated_at = loaded.updated_at;
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let ai = AiScheduleResult {
        weekly_summary: "全文\n\"引用\" 🌙".into(),
        ..Default::default()
    };
    db.save_ai_schedule_cache(&ai).unwrap();
    let (loaded, stamp) = db.get_ai_schedule_cache().unwrap().unwrap();
    assert!(stamp > 0);
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::to_value(ai).unwrap()
    );
}

#[test]
fn missing_tables_are_read_errors_instead_of_missing_rows() {
    let (db, _dir) = Temporary::open();
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE schedule_snapshot_state; DROP TABLE ai_schedule_cache;")
        .unwrap();
    assert!(db
        .get_snapshot_state()
        .unwrap_err()
        .starts_with("DB snapshot read:"));
    assert!(db
        .get_ai_schedule_cache()
        .unwrap_err()
        .starts_with("DB AI cache read:"));
}

#[test]
fn invalid_sql_column_types_are_read_errors_and_retained() {
    let (db, _dir) = Temporary::open();
    db.save_snapshot_state(&state()).unwrap();
    db.save_ai_schedule_cache(&AiScheduleResult::default())
        .unwrap();
    db.conn.lock().unwrap().execute_batch("UPDATE schedule_snapshot_state SET updated_at = 'broken'; UPDATE ai_schedule_cache SET updated_at = 'broken';").unwrap();
    assert!(db
        .get_snapshot_state()
        .unwrap_err()
        .starts_with("DB snapshot read:"));
    assert!(db
        .get_ai_schedule_cache()
        .unwrap_err()
        .starts_with("DB AI cache read:"));
    for table in ["schedule_snapshot_state", "ai_schedule_cache"] {
        let raw: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT updated_at FROM {table} WHERE id = 1"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "broken");
    }
}

#[test]
fn invalid_snapshot_collections_keep_compatible_fallbacks_for_network_refresh() {
    let (db, _dir) = Temporary::open();
    for (column, field) in [
        ("luna_communities_json", "communities"),
        ("luna_year_options_json", "year_options"),
        ("luna_term_options_json", "term_options"),
    ] {
        for bad in ["{unfinished", "null", "{}", "[{}]"] {
            db.save_snapshot_state(&state()).unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    &format!("UPDATE schedule_snapshot_state SET {column} = ?1 WHERE id = 1"),
                    params![bad],
                )
                .unwrap();
            let loaded = db.get_snapshot_state().unwrap().unwrap();
            let value = serde_json::to_value(&loaded).unwrap();
            assert_eq!(value[format!("luna_{field}")], json!([]));
            assert_eq!(loaded.current_week_label, "今週");
            assert_eq!(loaded.next_week_label, "来週");
            assert_eq!(loaded.luna_year, "2026");
            let raw: String = db
                .conn
                .lock()
                .unwrap()
                .query_row(
                    &format!("SELECT {column} FROM schedule_snapshot_state WHERE id = 1"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(raw, bad);
        }
    }
    db.save_snapshot_state(&state()).unwrap();
    assert!(db.get_snapshot_state().unwrap().is_some());
}

// This observer records real SQLite executions without adding a production hook.
struct SnapshotReads<'a> {
    db: &'a Database,
    count: Box<std::sync::atomic::AtomicUsize>,
}
impl<'a> SnapshotReads<'a> {
    fn observe(db: &'a Database) -> Self {
        let mut observer = Self {
            db,
            count: Box::default(),
        };
        let conn = db.conn.lock().unwrap();
        // SAFETY: the boxed counter has a stable address. Drop removes the callback
        // under the same connection mutex before that allocation can be freed.
        let status = unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                conn.handle(),
                rusqlite::ffi::SQLITE_TRACE_STMT as u32,
                Some(Self::record),
                observer.count.as_mut() as *mut _ as *mut std::ffi::c_void,
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        observer
    }
    unsafe extern "C" fn record(
        kind: u32,
        context: *mut std::ffi::c_void,
        _statement: *mut std::ffi::c_void,
        sql: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        if kind == rusqlite::ffi::SQLITE_TRACE_STMT as u32 && !context.is_null() && !sql.is_null() {
            // SAFETY: SQLite supplies the statement SQL for this callback only;
            // context refers to the live counter registered by observe().
            let text = unsafe { std::ffi::CStr::from_ptr(sql.cast()) }.to_bytes();
            if text
                .starts_with(b"SELECT current_week_label, next_week_label, luna_year, luna_term,")
            {
                unsafe { &*context.cast::<std::sync::atomic::AtomicUsize>() }
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        0
    }
    fn count(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Drop for SnapshotReads<'_> {
    fn drop(&mut self) {
        let conn = self.db.conn.lock().unwrap();
        // SAFETY: removing the callback while holding the connection mutex also
        // rules out a concurrent callback that could outlive the boxed counter.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(conn.handle(), 0, None, std::ptr::null_mut());
        }
    }
}

#[test]
fn schedule_builder_reads_and_decodes_saved_metadata_once() {
    use tauri::Manager;
    for saved in [false, true] {
        let (db, _dir) = Temporary::open();
        if saved {
            db.save_snapshot_state(&state()).unwrap();
        }
        db.save_ai_schedule_cache(&AiScheduleResult {
            current_week: vec![AiScheduleItem {
                course_name: "全文 🌙".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let app = tauri::test::mock_builder()
            .manage(db)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let db = app.state::<Database>().scope();
        let observer = SnapshotReads::observe(&db);
        let response = crate::timetable::build_schedule_snapshot(app.handle()).unwrap();
        assert_eq!(observer.count(), 1, "saved={saved}");
        // No saved metadata means no week filtering, unlike a saved default row.
        if !saved {
            assert!(response.ai_result.is_some());
        }
    }
}
