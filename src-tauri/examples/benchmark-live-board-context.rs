//! Actual cumulative board formatter and frozen predecessor; generated data only.
//! No app, microphone, model, user files, persistence or IPC is initialized.
use std::hint::black_box;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
mod live {
    pub use super::types::{LiveSummaryChunk, SharedSummaryChunk};
}
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/ai_output/context/before.rs"]
mod before;
#[path = "../src/live/ai_output/context.rs"]
mod current;
#[path = "../src/live/ai_output/context/fixtures.rs"]
mod fixtures;

fn elapsed(build: &impl Fn() -> String, repeats: usize) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..repeats {
        let started = Instant::now();
        let output = black_box(build());
        total += started.elapsed();
        black_box(&output);
        drop(output);
    }
    total / repeats as u32
}

fn main() {
    println!("Actual board context source vs frozen predecessor; {}; 3 warmup + 9 alternating batches, medians per call.",
        if cfg!(debug_assertions) { "default dev profile" } else { "optimized build" });
    println!("Includes latest board selection, indexes and full output. Separate System allocation samples include final String; exclude fixture creation, destruction/equality, models/network, app/UI/RSS/GPU and allocator/realloc internals.");
    for count in [0, 16, 96, 512, 4096] {
        let input = vec![
            fixtures::summary(Some(fixtures::board(count, 1))),
            fixtures::summary(None),
        ];
        let old = || before::format_latest_whiteboard_context(black_box(&input));
        let new = || current::format_latest_whiteboard_context(black_box(&input));
        for _ in 0..3 {
            assert_eq!(old(), new());
        }
        let repeats = if count < 512 { 100 } else { 10 };
        let mut old_times = [Duration::ZERO; 9];
        let mut new_times = [Duration::ZERO; 9];
        for i in 0..9 {
            if i % 2 == 0 {
                old_times[i] = elapsed(&old, repeats);
                new_times[i] = elapsed(&new, repeats);
            } else {
                new_times[i] = elapsed(&new, repeats);
                old_times[i] = elapsed(&old, repeats);
            }
        }
        old_times.sort_unstable();
        new_times.sort_unstable();
        let (old_text, old_alloc) = allocation::tracked(old);
        let (new_text, new_alloc) = allocation::tracked(new);
        assert_eq!(old_text.as_bytes(), new_text.as_bytes());
        assert_eq!(old_alloc.live, old_text.capacity());
        assert_eq!(new_alloc.live, new_text.capacity());
        println!("{count} nodes / {} bytes: {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}",
            new_text.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
            old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested, old_alloc.peak, new_alloc.peak);
    }
}
