//! Actual file recovery and its frozen predecessor; temporary synthetic files
//! only. No app, microphone, model, user files, IPC, or UI.
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/markdown/fixtures.rs"]
mod fixtures;

mod live {
    use super::*;
    #[allow(dead_code)]
    mod schema {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/types.rs"
        ));
    }
    use schema::{LiveDayCache, LiveLineDeltaBorrowed, LiveLineDeltaRef};
    mod current {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/recovery.rs"
        ));
    }
    #[allow(dead_code)]
    mod before {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/recovery/before.rs"
        ));
    }

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn elapsed(load: impl FnOnce() -> LiveDayCache) -> Duration {
        let start = Instant::now();
        let value = black_box(load());
        let elapsed = start.elapsed();
        black_box(&value);
        drop(value); // Output destruction and comparisons are outside elapsed.
        elapsed
    }
    #[derive(Clone, Copy, Debug)]
    pub(super) enum Case {
        Folded,
        MixedEscaped,
        MixedPlain,
        StalePlain,
    }
    pub(super) fn run(count: usize, case: Case) {
        let mixed = matches!(case, Case::MixedEscaped | Case::MixedPlain);
        let stale = matches!(case, Case::StalePlain);
        let base = if mixed { count / 3 } else { count };
        let mut lines = fixtures::lines(count);
        if matches!(case, Case::MixedPlain | Case::StalePlain) {
            // Typical STT lines without JSON escapes; a folded snapshot whose
            // previous journal could not be removed contains only stale rows.
            for (i, line) in lines.iter_mut().enumerate() {
                std::sync::Arc::make_mut(line).text = format!("{i}: 完整な発話 中文 한국어 👩🏽‍💻");
            }
        }
        let course = fixtures::course(false);
        let cache = LiveDayCache {
            date: "2026-10-08".into(),
            course_name: course.course_name,
            started_at: "2026-10-08 10:00:00".into(),
            transcript_lines: lines[..base].to_vec(),
            summaries: fixtures::summaries((count / 200).max(1)),
        };
        let dir = Directory(
            std::env::temp_dir().join(format!("selah-recovery-benchmark-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir(&dir.0).unwrap();
        let path = dir.0.join("snapshot.json");
        let journal = dir.0.join("lines.ndjson");
        let json = serde_json::to_vec(&cache).unwrap();
        let mut log = Vec::new();
        for (i, line) in lines.iter().enumerate().skip(if stale { 0 } else { base }) {
            serde_json::to_writer(
                &mut log,
                &LiveLineDeltaRef {
                    i,
                    t: &line.text,
                    a: &line.at,
                },
            )
            .unwrap();
            log.push(b'\n');
        }
        std::fs::write(&path, &json).unwrap();
        std::fs::write(&journal, &log).unwrap();
        let old = || before::load(&path, &journal, &cache.date, &cache.course_name).unwrap();
        let new = || current::load(&path, &journal, &cache.date, &cache.course_name).unwrap();
        for _ in 0..3 {
            assert_eq!(
                serde_json::to_vec(&old()).unwrap(),
                serde_json::to_vec(&new()).unwrap()
            );
        }
        let mut old_times = [Duration::ZERO; 9];
        let mut new_times = [Duration::ZERO; 9];
        for i in 0..9 {
            if i % 2 == 0 {
                old_times[i] = elapsed(old);
                new_times[i] = elapsed(new);
            } else {
                new_times[i] = elapsed(new);
                old_times[i] = elapsed(old);
            }
        }
        old_times.sort_unstable();
        new_times.sort_unstable();
        let (old_value, old_alloc) = allocation::tracked(old);
        let (new_value, new_alloc) = allocation::tracked(new);
        assert_eq!(
            serde_json::to_vec(&old_value).unwrap(),
            serde_json::to_vec(&new_value).unwrap()
        );
        assert_eq!(old_value.transcript_lines.len(), count);
        assert_eq!(old_alloc.live, new_alloc.live);
        if (mixed || stale) && count >= 1000 {
            assert!(new_alloc.peak < old_alloc.peak);
        }
        if stale {
            // Two borrowed fields per discarded row; only a few fixed buffer
            // allocations may differ between the full and streamed readers.
            assert!(new_alloc.calls + count * 2 <= old_alloc.calls + 4);
        }
        println!("{count} restored lines / {base} base / {case:?} / {} JSON + {} journal bytes: {:.3} -> {:.3} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; retained output {} -> {} bytes",
            json.len(),log.len(),old_times[4].as_secs_f64()*1000.0,new_times[4].as_secs_f64()*1000.0,
            old_alloc.calls,new_alloc.calls,old_alloc.requested,new_alloc.requested,old_alloc.peak,new_alloc.peak,old_alloc.live,new_alloc.live);
    }
}

fn main() {
    println!("Production file loader vs frozen predecessor; default dev profile; input/temp-file setup excluded; 9 alternating runs after warmup.");
    println!("Includes file reads, UTF-8 validation, JSON parsing, and complete replay with warm filesystem cache. Allocation tracked separately includes full owned output; excludes fixtures/allocator/realloc internals, IPC/UI/RSS/GPU and storage durability.");
    for count in [0, 1000, 10_000, 50_000] {
        for case in [live::Case::Folded, live::Case::MixedEscaped] {
            live::run(count, case);
        }
    }
    for count in [1000, 10_000, 50_000] {
        for case in [live::Case::MixedPlain, live::Case::StalePlain] {
            live::run(count, case);
        }
    }
}
