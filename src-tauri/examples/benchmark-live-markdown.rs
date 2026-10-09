//! Read-only benchmark of the actual Markdown source and frozen predecessor.
//! No app, microphone, model, user files, persistence or IPC is initialized.
use chrono::{DateTime, Local, TimeZone};
use std::hint::black_box;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
#[path = "../src/live/markdown/before.rs"]
mod before;
#[path = "../src/live/markdown.rs"]
mod current;
#[path = "../src/live/markdown/fixtures.rs"]
mod fixtures;

// The private LIVE module's clock-format adapter; both compared sources use
// the same timestamps/format. Regression tests use the production clock helper.
const FREE_NOTE_FOLDER_NAME: &str = "自由ノート";
fn format_datetime(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[path = "support/allocation.rs"]
mod allocation;
use allocation::tracked;

fn elapsed(build: impl FnOnce() -> String) -> Duration {
    let started = Instant::now();
    let output = black_box(build());
    let elapsed = started.elapsed();
    black_box(&output);
    drop(output); // Destruction and equality checks are outside elapsed time.
    elapsed
}

fn main() {
    let started = Local
        .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
        .single()
        .unwrap();
    let ended = started + chrono::Duration::hours(2);
    let course = fixtures::course(false);
    let overall = "### 全体要約\n- 完全な要約 🌕\n\n### 今回の論点\n- 確認";
    println!("Actual source vs frozen predecessor; default dev profile; fixture setup excluded.");
    println!("Timing: tracking disabled, 9 alternating runs, medians. Allocation: separate tracked calls.");
    println!("Peak requested live bytes at allocator boundaries include the output; exclude inputs, allocator overhead, realloc internals, disk/JSON IPC/UI/RSS/GPU.");
    for count in [0, 1_000, 10_000, 50_000] {
        let lines = fixtures::lines(count);
        let chunks = fixtures::summaries((count / 200).max(1));
        let old = || before::build_markdown(&course, started, ended, overall, &chunks, &lines);
        let new = || current::build_markdown(&course, started, ended, overall, &chunks, &lines);
        for _ in 0..3 {
            assert_eq!(old(), new());
        }
        let mut old_times = [Duration::ZERO; 9];
        let mut new_times = [Duration::ZERO; 9];
        for index in 0..9 {
            if index % 2 == 0 {
                old_times[index] = elapsed(old);
                new_times[index] = elapsed(new);
            } else {
                new_times[index] = elapsed(new);
                old_times[index] = elapsed(old);
            }
        }
        old_times.sort_unstable();
        new_times.sort_unstable();
        let (old_text, old_alloc) = tracked(old);
        let (new_text, new_alloc) = tracked(new);
        assert_eq!(old_alloc.live, old_text.capacity());
        assert_eq!(new_alloc.live, new_text.capacity());
        assert_eq!(old_text.as_bytes(), new_text.as_bytes());
        if count >= 1_000 {
            assert!(old_alloc.calls > count);
            assert!(
                new_alloc.calls < 100,
                "line-dependent allocation appeared: {new_alloc:?}"
            );
            assert!(new_alloc.peak < old_alloc.peak);
        }
        println!("{count} lines / {} chunks / {} output bytes: {:.3} -> {:.3} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; final capacity {} -> {}",
            chunks.len(), old_text.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
            old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested,
            old_alloc.peak, new_alloc.peak, old_text.capacity(), new_text.capacity());
    }
}
