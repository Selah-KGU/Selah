use super::Timing;
use crate::ai::config::read_live_summary_interval;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

struct File(PathBuf);

impl File {
    fn new(minutes: u32) -> Self {
        let file = Self(
            std::env::temp_dir().join(format!("selah-timing-cache-{}.json", uuid::Uuid::new_v4())),
        );
        file.write(minutes).unwrap();
        file
    }

    fn write(&self, minutes: u32) -> std::io::Result<()> {
        std::fs::write(
            &self.0,
            format!(r#"{{"live_summary_interval_minutes":{minutes}}}"#),
        )
    }

    fn minutes(&self) -> u32 {
        read_live_summary_interval(&self.0) as u32
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn hot_reads_keep_cached_timing_until_an_explicit_refresh() {
    let timing = Timing::new();
    let file = File::new(15);
    assert_eq!(timing.minutes(), 5);
    timing.refresh(|| file.minutes());
    file.write(30).unwrap();
    for _ in 0..10_000 {
        assert_eq!(timing.minutes(), 15);
    }
    timing.refresh(|| file.minutes());
    assert_eq!(timing.minutes(), 30);
    std::fs::remove_file(&file.0).unwrap();
    assert_eq!(timing.minutes(), 30);
    timing.refresh(|| file.minutes());
    assert_eq!(timing.minutes(), 5);
}

#[test]
fn failed_save_preserves_timing_and_does_not_invalidate_a_captured_read() {
    let timing = Timing::new();
    let file = File::new(12);
    timing.refresh(|| file.minutes());
    let (revision, old) = timing.read(|| file.minutes());
    // A real IO error, without reading or modifying the user's configuration.
    let missing_parent = file.0.join("missing");
    let expected_kind = std::fs::write(&missing_parent, "30").unwrap_err().kind();
    let error = timing
        .commit(30, || std::fs::write(&missing_parent, "30"))
        .unwrap_err();
    assert_eq!(error.kind(), expected_kind);
    assert_eq!(timing.minutes(), 12);
    assert_eq!(file.minutes(), 12);
    assert_eq!(timing.read(|| ()).0, revision);
    timing.observe(revision, old);
    assert_eq!(timing.minutes(), 12);
    timing.commit(30, || file.write(30)).unwrap();
    assert_eq!(timing.minutes(), 30);
    assert_eq!(file.minutes(), 30);
}

#[test]
fn a_late_config_read_and_migration_cannot_overwrite_a_newer_save() {
    let timing = Timing::new();
    let file = File::new(10);
    timing.refresh(|| file.minutes());
    let (old_revision, old_minutes) = timing.read(|| file.minutes());
    timing.commit(25, || file.write(25)).unwrap();
    timing.observe(old_revision, old_minutes);
    let migrated = timing
        .commit_if(Some(old_revision), old_minutes, || file.write(old_minutes))
        .unwrap();
    assert!(!migrated);
    assert_eq!(file.minutes(), 25);
    assert_eq!(timing.minutes(), 25);
    let (current, _) = timing.read(|| ());
    assert!(timing
        .commit_if(Some(current), 18, || file.write(18))
        .unwrap());
    assert!(!timing
        .commit_if(Some(current), 40, || file.write(40))
        .unwrap());
    assert_eq!(file.minutes(), 18);
    assert_eq!(timing.minutes(), 18);
}

#[test]
fn recording_start_refresh_also_invalidates_older_full_config_reads() {
    let timing = Timing::new();
    let file = File::new(10);
    let (before, minutes) = timing.read(|| file.minutes());
    file.write(22).unwrap();
    timing.refresh(|| file.minutes());
    timing.observe(before, minutes);
    assert!(!timing
        .commit_if(Some(before), minutes, || file.write(minutes))
        .unwrap());
    assert_eq!(timing.minutes(), 22);
    assert_eq!(file.minutes(), 22);
}

#[test]
fn a_delayed_start_refresh_cannot_publish_after_a_newer_successful_save() {
    let timing = Arc::new(Timing::new());
    let file = Arc::new(File::new(10));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reader_timing = timing.clone();
    let reader_file = file.clone();
    let reader = std::thread::spawn(move || {
        reader_timing.refresh(|| {
            let minutes = reader_file.minutes();
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            minutes
        });
    });
    entered_rx.recv_timeout(WAIT).unwrap();
    let writer_timing = timing.clone();
    let writer_file = file.clone();
    let writer = std::thread::spawn(move || {
        writer_timing.commit(15, || writer_file.write(15)).unwrap();
    });
    // This getter runs while the file reader owns the IO gate.
    assert_eq!(timing.minutes(), 5);
    release_tx.send(()).unwrap();
    reader.join().unwrap();
    writer.join().unwrap();
    assert_eq!(timing.minutes(), 15);
    assert_eq!(file.minutes(), 15);
}

#[test]
fn full_config_reads_wait_for_a_complete_write_and_cache_matches_final_disk() {
    let timing = Arc::new(Timing::new());
    let file = Arc::new(File::new(10));
    timing.refresh(|| file.minutes());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer_timing = timing.clone();
    let writer_file = file.clone();
    let writer = std::thread::spawn(move || {
        writer_timing
            .commit(30, || {
                std::fs::write(&writer_file.0, "partial JSON")?;
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(WAIT).unwrap();
                writer_file.write(30)
            })
            .unwrap();
    });
    entered_rx.recv_timeout(WAIT).unwrap();
    let reader_timing = timing.clone();
    let reader_file = file.clone();
    let reader = std::thread::spawn(move || reader_timing.read(|| reader_file.minutes()));
    let second_timing = timing.clone();
    let second_file = file.clone();
    let second_writer = std::thread::spawn(move || {
        second_timing.commit(40, || second_file.write(40)).unwrap();
    });
    assert_eq!(timing.minutes(), 10);
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    let (revision, minutes) = reader.join().unwrap();
    second_writer.join().unwrap();
    assert!(matches!(minutes, 30 | 40));
    timing.observe(revision, minutes);
    assert_eq!(file.minutes(), 40);
    assert_eq!(timing.minutes(), 40);
}

#[test]
fn failed_or_panicking_io_can_be_followed_by_a_successful_save() {
    let timing = Timing::new();
    timing.commit(12, || Ok::<_, ()>(())).unwrap();
    let panic = std::panic::catch_unwind(|| {
        let _ = timing.commit(20, || -> Result<(), ()> { panic!("IO interrupted") });
    });
    assert!(panic.is_err());
    assert_eq!(timing.minutes(), 12);
    timing.commit(1, || Ok::<_, ()>(())).unwrap();
    assert_eq!(timing.minutes(), 5);
    timing.commit(u32::MAX, || Ok::<_, ()>(())).unwrap();
    assert_eq!(timing.minutes(), i64::from(u32::MAX));
}
