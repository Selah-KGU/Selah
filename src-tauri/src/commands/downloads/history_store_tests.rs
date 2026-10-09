use super::*;
use crate::commands::downloads::{history, migrate, scan};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    store: HistoryStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("selah-history-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let store = HistoryStore::new(root.join("download_history.json"));
        Self { root, store }
    }
    fn record(&self, id: &str) -> DownloadRecord {
        DownloadRecord {
            id: id.into(),
            filename: format!("授業 {id} 👩🏽‍💻.md"),
            path: self
                .root
                .join(format!("{id}.md"))
                .to_string_lossy()
                .into_owned(),
            course_name: "政治学基礎".into(),
            source: "luna".into(),
            size_bytes: 987,
            downloaded_at: 123456789,
            file_exists: true,
            subfolder: "第01回/教材".into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// The previous load/save boundary, with only its global path replaced by a
// fixture parameter. This deliberately retains its fallback and overwrite.
fn previous_load(path: &std::path::Path) -> Vec<DownloadRecord> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
fn previous_save(path: &std::path::Path, records: &[DownloadRecord]) {
    std::fs::write(path, serde_json::to_string(records).unwrap()).unwrap();
}

#[test]
fn previous_stale_snapshots_lose_updates_and_restore_removed_rows() {
    let f = Fixture::new();
    let old = f.record("old");
    let a = f.record("a");
    let b = f.record("b");
    previous_save(&f.store.path, std::slice::from_ref(&old));
    let mut first = previous_load(&f.store.path);
    let mut second = previous_load(&f.store.path);
    first.push(a.clone());
    second.push(b.clone());
    previous_save(&f.store.path, &first);
    previous_save(&f.store.path, &second);
    assert_eq!(previous_load(&f.store.path), vec![old.clone(), b.clone()]);
    // A read captured before deletion also resurrected its removed row.
    let mut stale_scan = previous_load(&f.store.path);
    previous_save(&f.store.path, std::slice::from_ref(&b));
    stale_scan.push(a.clone());
    previous_save(&f.store.path, &stale_scan);
    assert!(previous_load(&f.store.path).contains(&old));

    f.store.clear().unwrap();
    for record in [old.clone(), a.clone(), b.clone()] {
        f.store
            .update(|records| {
                history::upsert_download_record(records, record);
                true
            })
            .unwrap();
    }
    f.store
        .update(|records| history::retain_download_records(records, |r| r.id != old.id))
        .unwrap();
    let c = f.record("c");
    f.store
        .update(|records| scan::merge_discovered_records(records, vec![c.clone()]))
        .unwrap();
    assert_eq!(f.store.read().unwrap(), vec![a, b, c]);
}

#[test]
fn independent_thread_writers_keep_every_full_record_and_atomic_readers_never_see_partial_json() {
    let f = Fixture::new();
    f.store.clear().unwrap();
    let gate = Arc::new(Barrier::new(9));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut expected = Vec::new();
    let mut writers = Vec::new();
    for worker in 0..8 {
        let rows: Vec<_> = (0..25)
            .map(|i| f.record(&format!("{worker}-{i}")))
            .collect();
        expected.extend(rows.iter().cloned());
        let path = f.store.path.clone();
        let gate = gate.clone();
        let finished = finished.clone();
        writers.push(std::thread::spawn(move || {
            gate.wait();
            for row in rows {
                HistoryStore::new(path.clone())
                    .update(|records| {
                        history::upsert_download_record(records, row);
                        true
                    })
                    .unwrap();
            }
            finished.fetch_add(1, Ordering::Release);
        }));
    }
    let path = f.store.path.clone();
    let reader = std::thread::spawn(move || {
        gate.wait();
        let mut reads = 0;
        loop {
            // An external reader without the sidecar lock must still see a
            // complete JSON file. Shared-lock reads exercise a separate handle.
            let raw: Vec<DownloadRecord> =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert!(raw.len() <= 200);
            assert!(HistoryStore::new(path.clone()).read().unwrap().len() <= 200);
            reads += 1;
            if finished.load(Ordering::Acquire) == 8 {
                break;
            }
        }
        reads
    });
    for worker in writers {
        worker.join().unwrap();
    }
    assert!(reader.join().unwrap() > 0);
    let mut actual = f.store.read().unwrap();
    actual.sort_by(|a, b| a.id.cmp(&b.id));
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(actual, expected);
}

#[test]
fn missing_history_is_empty_but_corrupt_or_unreadable_history_is_never_overwritten() {
    let f = Fixture::new();
    assert!(f.store.read().unwrap().is_empty());
    assert!(!f.store.path.exists());
    for bytes in [
        b"[".as_slice(),
        b"",
        b"{}",
        b"[{\"id\":\"missing fields\"}]",
        b"not JSON",
    ] {
        std::fs::write(&f.store.path, bytes).unwrap();
        assert!(f.store.read().is_err());
        assert!(f
            .store
            .update(|_| panic!("invalid JSON must stop before mutation"))
            .is_err());
        assert_eq!(std::fs::read(&f.store.path).unwrap(), bytes);
    }
    // Explicit user clear is the sole recovery operation that discards history.
    f.store.clear().unwrap();
    assert!(f.store.read().unwrap().is_empty());
    std::fs::remove_file(&f.store.path).unwrap();
    std::fs::create_dir(&f.store.path).unwrap();
    std::fs::write(f.store.path.join("keep"), b"keep").unwrap();
    assert!(f.store.read().is_err());
    assert!(f.store.update(|_| true).is_err());
    assert!(f.store.clear().is_err());
    assert_eq!(std::fs::read(f.store.path.join("keep")).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 2); // history + stable lock
}

#[test]
fn no_op_removal_and_migration_preserve_the_file_and_do_not_create_history() {
    let f = Fixture::new();
    f.store
        .update(|r| history::retain_download_records(r, |_| true))
        .unwrap();
    assert!(!f.store.path.exists());
    let row = f.record("keep");
    f.store
        .update(|r| {
            r.push(row.clone());
            true
        })
        .unwrap();
    let before = std::fs::metadata(&f.store.path).unwrap();
    let bytes = std::fs::read(&f.store.path).unwrap();
    for _ in 0..20 {
        f.store
            .update(|r| history::retain_download_records(r, |row| row.id != "absent"))
            .unwrap();
        f.store.update(migrate::normalize_course_names).unwrap();
    }
    let after = std::fs::metadata(&f.store.path).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(std::fs::read(&f.store.path).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 2);
}

#[test]
fn panicked_transaction_keeps_prior_history_and_releases_the_lock() {
    let f = Fixture::new();
    let original = f.record("original");
    f.store
        .update(|r| {
            r.push(original.clone());
            true
        })
        .unwrap();
    let failed = std::panic::catch_unwind(|| {
        f.store.update(|r| {
            r.clear();
            panic!("fixture mutation failed");
        })
    });
    assert!(failed.is_err());
    assert_eq!(f.store.read().unwrap(), vec![original.clone()]);
    let path = f.store.path.clone();
    let next = f.record("next");
    let expected = vec![original, next.clone()];
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        tx.send(HistoryStore::new(path).update(|r| {
            r.push(next);
            true
        }))
        .unwrap();
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
        expected
    );
    worker.join().unwrap();
}

#[test]
fn scan_merge_keeps_completed_download_metadata_and_only_removes_matching_missing_rows() {
    let f = Fixture::new();
    let real = f.record("completed");
    let mut same_course = f.record("missing-same");
    same_course.filename = "moved.md".into();
    let mut other_course = same_course.clone();
    other_course.id = "missing-other".into();
    other_course.path = f.root.join("other.md").to_string_lossy().into_owned();
    other_course.course_name = "別の授業".into();
    let mut still_exists = same_course.clone();
    still_exists.id = "still-exists".into();
    still_exists.path = f.root.join("exists.md").to_string_lossy().into_owned();
    std::fs::write(&still_exists.path, b"file").unwrap();
    f.store
        .update(|r| {
            r.extend([
                real.clone(),
                same_course,
                other_course.clone(),
                still_exists.clone(),
            ]);
            true
        })
        .unwrap();
    // Captured before completion; a path collision must keep the real record.
    let mut old_scan = real.clone();
    old_scan.id = "scan_old".into();
    old_scan.source = "scan".into();
    old_scan.size_bytes = 1;
    let mut moved = f.record("moved");
    moved.filename = "moved.md".into();
    let result = f
        .store
        .update(|r| scan::merge_discovered_records(r, vec![old_scan, moved.clone(), moved.clone()]))
        .unwrap();
    assert_eq!(
        result,
        vec![real, other_course, still_exists, moved.clone()]
    );
    let before = std::fs::metadata(&f.store.path)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        f.store
            .update(|r| scan::merge_discovered_records(r, vec![moved]))
            .unwrap(),
        result
    );
    assert_eq!(
        std::fs::metadata(&f.store.path)
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn upsert_preserves_position_full_metadata_and_the_existing_500_record_limit() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 0..510 {
        history::upsert_download_record(&mut rows, f.record(&n.to_string()));
    }
    assert_eq!(rows.len(), 500);
    assert_eq!(rows.first().unwrap().id, "10");
    assert_eq!(rows.last().unwrap().id, "509");
    let mut completed = rows[100].clone();
    completed.id = "downloaded".into();
    completed.source = "live".into();
    completed.size_bytes = u64::MAX;
    history::upsert_download_record(&mut rows, completed.clone());
    assert_eq!(rows.len(), 500);
    assert_eq!(rows[100], completed);
    assert_eq!(rows[101].id, "111");
    f.store
        .update(|r| {
            *r = rows.clone();
            true
        })
        .unwrap();
    assert_eq!(f.store.read().unwrap(), rows);
}

#[test]
fn migration_transforms_latest_rows_preserves_courses_and_prefers_existing_then_recent_files() {
    let f = Fixture::new();
    let mut old = f.record("old");
    old.course_name = "水４・金２ 日本語I ４".into();
    let mut newer = f.record("newer");
    newer.filename = old.filename.clone();
    newer.course_name = old.course_name.clone();
    newer.downloaded_at += 1;
    let mut other = newer.clone();
    other.id = "other-course".into();
    other.course_name = "別の授業".into();
    let mut loose = f.record("loose");
    loose.course_name.clear();
    std::fs::write(&old.path, b"existing").unwrap();
    f.store
        .update(|r| {
            r.extend([old.clone(), newer.clone(), other.clone(), loose.clone()]);
            true
        })
        .unwrap();
    let new_path = f.root.join("new-place.md").to_string_lossy().into_owned();
    let map = std::collections::HashMap::from([(loose.path.clone(), new_path.clone())]);
    f.store
        .update(|r| migrate::apply_uncategorized_paths(r, &map))
        .unwrap();
    f.store.update(migrate::normalize_course_names).unwrap();
    let rows = f
        .store
        .update(migrate::deduplicate_history_records)
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].id, old.id); // Live old beats a newer missing file.
    assert_eq!(rows[0].course_name, "日本語I ４");
    assert_eq!(rows[1], other);
    assert_eq!(rows[2].path, new_path);
    assert_eq!(rows[2].course_name, crate::commands::OTHER_CATEGORY);
    // Both live: latest timestamp wins, and unrelated courses survive.
    newer.course_name = "日本語I ４".into();
    std::fs::write(&newer.path, b"existing").unwrap();
    f.store
        .update(|r| {
            r.push(newer.clone());
            true
        })
        .unwrap();
    let rows = f
        .store
        .update(migrate::deduplicate_history_records)
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| &r.id).collect::<Vec<_>>(),
        vec![&other.id, &loose.id, &newer.id]
    );
    let renamed = f.root.join("renamed.md").to_string_lossy().into_owned();
    let map = std::collections::HashMap::from([(newer.path.clone(), renamed.clone())]);
    let rows = f
        .store
        .update(|r| migrate::apply_renamed_course_paths(r, &map))
        .unwrap();
    assert_eq!(rows.last().unwrap().path, renamed);
    assert_eq!(rows.last().unwrap().size_bytes, newer.size_bytes);
}

#[test]
fn downloaded_checks_preserve_aliases_case_matching_and_existing_file_preference() {
    let f = Fixture::new();
    let mut real = f.record("real");
    real.filename = "資料.PDF".into();
    real.course_name = "日本語I ４".into();
    std::fs::write(&real.path, b"real").unwrap();
    let mut missing = real.clone();
    missing.path = f.root.join("missing.pdf").to_string_lossy().into_owned();
    missing.id = "missing".into();
    missing.downloaded_at += 1;
    let rows = vec![real.clone(), missing.clone()];
    let found = history::check_downloaded_records(
        &rows,
        vec![" ".into(), "資料.PDF".into(), "unknown.pdf".into()],
        Some("水４・金２ 日本語I ４"),
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found["資料.PDF"], real);
    assert_eq!(found["資料.pdf"], real);
    assert!(
        history::check_downloaded_records(&rows, vec!["資料.PDF".into()], Some("別の授業"))
            .is_empty()
    );
    std::fs::remove_file(&real.path).unwrap();
    missing.file_exists = false;
    assert_eq!(
        history::check_downloaded_records(&rows, vec!["資料.PDF".into()], None)["資料.PDF"],
        missing
    );
}

#[test]
fn real_directory_scan_reuses_the_transaction_and_keeps_the_completed_source() {
    let f = Fixture::new();
    let base = f.root.join("files");
    let course = base.join("水４・金２ 日本語I ４");
    let theme = course.join("第01回");
    std::fs::create_dir_all(&theme).unwrap();
    let path = theme.join("資料.md");
    std::fs::write(&path, "# 全文\nUnicode 👩🏽‍💻").unwrap();
    let rows = scan::scan_download_history(&f.store, &base).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].course_name, "日本語I ４");
    let mut completed = rows[0].clone();
    completed.id = "actual-download".into();
    completed.source = "live".into();
    completed.downloaded_at += 1;
    f.store
        .update(|r| {
            history::upsert_download_record(r, completed.clone());
            true
        })
        .unwrap();
    let before = std::fs::metadata(&f.store.path)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        scan::scan_download_history(&f.store, &base).unwrap(),
        [completed]
    );
    assert_eq!(
        std::fs::metadata(&f.store.path)
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ipc_worker_waits_for_history_lock_without_blocking_async_tasks_and_keeps_full_json() {
    use tauri::ipc::IpcResponse;
    let f = Fixture::new();
    let mut row = f.record("large");
    row.filename = "全文 👩🏽‍💻 \"引用\"\n".repeat(10000);
    row.size_bytes = u64::MAX;
    f.store
        .update(|r| {
            r.push(row.clone());
            true
        })
        .unwrap();
    let expected = serde_json::to_value(vec![row]).unwrap();
    let lock = f.store.lock(true).unwrap();
    let path = f.store.path.clone();
    let origin = std::thread::current().id();
    let (entered, received) = tokio::sync::oneshot::channel();
    let response = tokio::spawn(async move {
        crate::background_ipc::respond(
            "History fixture worker",
            "History fixture JSON",
            move || {
                entered.send(std::thread::current().id()).unwrap();
                HistoryStore::new(path).read()
            },
        )
        .await
    });
    assert_ne!(
        tokio::time::timeout(Duration::from_secs(5), received)
            .await
            .unwrap()
            .unwrap(),
        origin
    );
    tokio::task::yield_now().await;
    assert!(!response.is_finished()); // Reader is blocked on the OS lock, not this executor.
    drop(lock);
    let result = tokio::time::timeout(Duration::from_secs(5), response)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        result
            .body()
            .unwrap()
            .deserialize::<serde_json::Value>()
            .unwrap(),
        expected
    );
}

struct Process(std::process::Child);
impl Process {
    fn wait_success(&mut self) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < until, "history child process timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn separate_processes_share_the_stable_lock_and_merge_every_record() {
    let f = Fixture::new();
    let lock = f.store.lock(true).unwrap();
    let mut children = Vec::new();
    for label in ["child-a", "child-b"] {
        children.push(Process(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::downloads::history_store::tests::history_process_worker",
                    "--ignored",
                ])
                .env("SELAH_HISTORY_FIXTURE_ROOT", &f.root)
                .env("SELAH_HISTORY_FIXTURE_LABEL", label)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        ));
    }
    let until = Instant::now() + Duration::from_secs(15);
    while !["child-a.blocked", "child-b.blocked"]
        .iter()
        .all(|name| f.root.join(name).exists())
    {
        assert!(
            Instant::now() < until,
            "children failed to observe the parent's lock"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Neither process can commit while the parent owns the stable sidecar.
    assert!(!f.store.path.exists());
    let parent = f.record("parent");
    f.store.write_locked(std::slice::from_ref(&parent)).unwrap();
    drop(lock);
    for child in &mut children {
        child.wait_success();
    }
    let rows = f.store.read().unwrap();
    assert_eq!(rows.len(), 25);
    assert_eq!(rows[0], parent);
    let ids: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
    for label in ["child-a", "child-b"] {
        for n in 0..12 {
            assert!(ids.contains(format!("{label}-{n}").as_str()));
        }
    }
}

#[test]
#[ignore = "subprocess fixture used by separate_processes_share_the_stable_lock_and_merge_every_record"]
fn history_process_worker() {
    let root = PathBuf::from(std::env::var_os("SELAH_HISTORY_FIXTURE_ROOT").expect("fixture root"));
    let label = std::env::var("SELAH_HISTORY_FIXTURE_LABEL").expect("fixture label");
    let store = HistoryStore::new(root.join("download_history.json"));
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.path.with_extension("lock"))
        .unwrap();
    assert!(matches!(
        probe.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    std::fs::write(root.join(format!("{label}.blocked")), b"blocked").unwrap();
    drop(probe);
    for n in 0..12 {
        let id = format!("{label}-{n}");
        store
            .update(|rows| {
                rows.push(DownloadRecord {
                    id: id.clone(),
                    filename: format!("授業 {id}.md"),
                    path: root.join(format!("{id}.md")).to_string_lossy().into_owned(),
                    course_name: "歴史".into(),
                    source: "live".into(),
                    size_bytes: 31,
                    downloaded_at: n,
                    file_exists: true,
                    subfolder: String::new(),
                });
                true
            })
            .unwrap();
    }
}
