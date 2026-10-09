use super::*;
use serde_json::json;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-read-state-{}", uuid::Uuid::new_v4())))
    }
    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.0.join("courses.db")).unwrap()
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn empty(db: &Database) {
    // Ensure tests never inspect or migrate the user's old read_items.json.
    db.save_data_cache(CACHE_KEY, r#"{"kgc":[],"luna":[],"kwic":[]}"#)
        .unwrap();
}

#[test]
fn borrowed_batches_filter_invalid_ids_and_duplicates_without_rewriting_known_state() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    empty(&db);
    let boundary = "月".repeat(170) + "ab"; // exactly 512 UTF-8 bytes
    let too_long = boundary.clone() + "c";
    for source in ["kgc", "luna", "kwic"] {
        mark_batch_read(
            &db,
            source,
            ["", "one", "one", "日本語 🌕", &boundary, &too_long],
        )
        .unwrap();
        let state = load_from(&db, || panic!("existing cache consulted legacy file")).unwrap();
        let set = match source {
            "kgc" => state.kgc,
            "luna" => state.luna,
            _ => state.kwic,
        };
        assert_eq!(
            set,
            ["one".to_owned(), "日本語 🌕".to_owned(), boundary.clone()]
                .into_iter()
                .collect()
        );
    }
    let before = db.get_data_cache(CACHE_KEY).unwrap();
    let conn = temporary.connection();
    conn.execute_batch("CREATE TABLE read_write_audit (attempt INTEGER); CREATE TRIGGER audit_read_write AFTER UPDATE OF data_json, updated_at ON data_cache WHEN OLD.cache_key='read_state' BEGIN INSERT INTO read_write_audit VALUES (1); END;").unwrap();
    for source in ["kgc", "luna", "kwic"] {
        mark_read(&db, source, "one").unwrap();
        mark_batch_read(&db, source, ["", "one", "one", &too_long]).unwrap();
    }
    mark_batch_read(&db, "unknown", ["new"]).unwrap();
    assert_eq!(db.get_data_cache(CACHE_KEY).unwrap(), before);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM read_write_audit", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    mark_batch_read(&db, "luna", ["new"]).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM read_write_audit", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(get_all_read_ids(&db)
        .unwrap()
        .luna
        .contains(&"new".to_owned()));
}

#[test]
fn source_caps_stay_bounded_and_other_sources_keep_every_id() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    empty(&db);
    mark_read(&db, "kgc", "keep kgc").unwrap();
    mark_read(&db, "kwic", "keep kwic").unwrap();
    let ids: Vec<String> = (0..700).map(|n| format!("luna-{n}")).collect();
    mark_batch_read(&db, "luna", ids.iter().map(String::as_str)).unwrap();
    let data = get_all_read_ids(&db).unwrap();
    assert_eq!(data.luna.len(), MAX_IDS_PER_SOURCE);
    assert!(data.luna.iter().all(|id| ids.contains(id)));
    assert_eq!(data.kgc, ["keep kgc"]);
    assert_eq!(data.kwic, ["keep kwic"]);
}

#[test]
fn failed_migration_keeps_the_source_file_and_successful_retry_commits_before_removal() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    let path = temporary.0.join("read_items.json");
    let text = serde_json::to_vec(
        &json!({"kgc":["existing kgc"],"luna":["日本語 🌕"],"kwic":["existing kwic"]}),
    )
    .unwrap();
    std::fs::write(&path, &text).unwrap();
    let conn = temporary.connection();
    conn.execute_batch("CREATE TRIGGER reject_migration BEFORE INSERT ON data_cache WHEN NEW.cache_key='read_state' BEGIN SELECT RAISE(ABORT,'migration failed'); END;").unwrap();
    let error = load_from(&db, || path.clone()).unwrap_err();
    assert!(error.contains("migration failed"));
    assert_eq!(std::fs::read(&path).unwrap(), text);
    assert!(db.cache_payload(CACHE_KEY).is_none());
    conn.execute_batch("DROP TRIGGER reject_migration;")
        .unwrap();
    let retried = load_from(&db, || path.clone()).unwrap();
    assert!(retried.kgc.contains("existing kgc"));
    assert!(retried.luna.contains("日本語 🌕"));
    assert!(retried.kwic.contains("existing kwic"));
    assert!(!path.exists());
    let reopened = Database::open(&temporary.0).unwrap();
    let saved = load_from(&reopened, || {
        panic!("committed migration re-read the source")
    })
    .unwrap();
    assert_eq!(saved.luna, retried.luna);
}

#[test]
fn malformed_legacy_files_are_retained_and_existing_cache_never_reads_the_file() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    let path = temporary.0.join("read_items.json");
    std::fs::write(&path, b"invalid old file").unwrap();
    let error = load_from(&db, || path.clone()).unwrap_err();
    assert!(error.starts_with("旧既読データの解析失敗:"));
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid old file");
    db.save_data_cache(CACHE_KEY, "invalid cache").unwrap();
    let error = load_from(&db, || {
        panic!("existing malformed cache changed migration behavior")
    })
    .unwrap_err();
    assert!(error.starts_with("既読データの解析失敗:"));
}

#[test]
fn failed_single_and_batch_updates_report_errors_without_losing_existing_ids() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    empty(&db);
    mark_batch_read(&db, "kgc", ["previous kgc 🌕"]).unwrap();
    mark_batch_read(&db, "luna", ["previous luna 🌕"]).unwrap();
    mark_batch_read(&db, "kwic", ["previous kwic 🌕"]).unwrap();
    let before = db.get_data_cache(CACHE_KEY).unwrap();
    let conn = temporary.connection();
    conn.execute_batch("CREATE TRIGGER reject_read_commit BEFORE UPDATE ON data_cache WHEN OLD.cache_key='read_state' BEGIN SELECT RAISE(ABORT,'original commit failure'); END;").unwrap();
    assert!(mark_read(&db, "luna", "single")
        .unwrap_err()
        .contains("original commit failure"));
    assert!(mark_batch_read(&db, "luna", ["batch one", "batch two"])
        .unwrap_err()
        .contains("original commit failure"));
    assert_eq!(db.get_data_cache(CACHE_KEY).unwrap(), before);
    // Known IDs still require no write, even while commits are rejected.
    mark_read(&db, "luna", "previous luna 🌕").unwrap();
    mark_batch_read(&db, "luna", ["previous luna 🌕", "previous luna 🌕"]).unwrap();
    conn.execute_batch("DROP TRIGGER reject_read_commit;")
        .unwrap();
    mark_read(&db, "luna", "single").unwrap();
    mark_batch_read(&db, "luna", ["batch one", "batch two"]).unwrap();
    let data = get_all_read_ids(&db).unwrap();
    assert_eq!(data.kgc, ["previous kgc 🌕"]);
    assert_eq!(data.kwic, ["previous kwic 🌕"]);
    assert_eq!(
        data.luna.into_iter().collect::<HashSet<_>>(),
        ["previous luna 🌕", "single", "batch one", "batch two"]
            .into_iter()
            .map(String::from)
            .collect()
    );
}

#[test]
fn corrupt_cache_rejects_reads_and_marks_without_replacing_the_original_bytes() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    for payload in [
        "invalid cached JSON",
        r#"{"kgc":["retained kgc"],"luna":[]}"#,
    ] {
        db.save_data_cache(CACHE_KEY, payload).unwrap();
        let before = db.get_data_cache(CACHE_KEY).unwrap();
        assert!(get_all_read_ids(&db)
            .unwrap_err()
            .starts_with("既読データの解析失敗:"));
        assert!(mark_read(&db, "luna", "new").is_err());
        assert!(mark_batch_read(&db, "luna", ["new"]).is_err());
        assert_eq!(db.get_data_cache(CACHE_KEY).unwrap(), before);
    }
    let conn = temporary.connection();
    let bytes = vec![0xff_u8; 4096];
    conn.execute(
        "UPDATE data_cache SET data_json=?1 WHERE cache_key='read_state'",
        rusqlite::params![&bytes],
    )
    .unwrap();
    assert!(get_all_read_ids(&db).is_err());
    assert!(mark_read(&db, "luna", "new").is_err());
    assert!(mark_batch_read(&db, "luna", ["new"]).is_err());
    let after: Vec<u8> = conn
        .query_row(
            "SELECT data_json FROM data_cache WHERE cache_key='read_state'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, bytes);
}

#[test]
fn read_failures_never_consult_legacy_files_and_only_not_found_means_no_prior_data() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    let missing = temporary.0.join("missing.json");
    let initial = load_from(&db, || missing).unwrap();
    assert!(initial.kgc.is_empty() && initial.luna.is_empty() && initial.kwic.is_empty());
    assert!(load_from(&db, || temporary.0.clone())
        .unwrap_err()
        .starts_with("旧既読データの読み取り失敗:"));
    temporary
        .connection()
        .execute_batch("DROP TABLE data_cache;")
        .unwrap();
    let error = load_from(&db, || {
        panic!("DB read failure consulted user's migration file")
    })
    .unwrap_err();
    assert!(error.starts_with("DB get cache:"));
    assert!(get_all_read_ids(&db).is_err());
    assert!(mark_read(&db, "luna", "new").is_err());
    assert!(mark_batch_read(&db, "luna", ["new"]).is_err());
    mark_read(&db, "luna", "").unwrap();
    mark_read(&db, "unknown", "valid").unwrap();
    mark_batch_read(&db, "unknown", ["valid"]).unwrap();
}
