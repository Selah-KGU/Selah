//! Actual model board parser vs frozen predecessor; generated JSON only.
//! No app, microphone, network, user files, persistence or IPC.
use std::hint::black_box;
use std::time::{Duration, Instant};
#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::{LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode};
#[allow(dead_code)]
#[path = "../src/live/ai_output/json.rs"]
mod text;
#[allow(unused_imports)]
use text::{clamp_chars, value_to_trimmed_string};
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/ai_output/board/before.rs"]
mod before;
#[path = "../src/live/ai_output/board.rs"]
mod current;
#[path = "../src/live/ai_output/board/fixtures.rs"]
mod fixtures;

fn elapsed(build: &impl Fn() -> Option<LiveWhiteboard>, repeats: usize) -> Duration {
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

fn run(input: serde_json::Value, label: &str, repeats: usize) {
    let old = || before::parse_live_whiteboard(Some(black_box(&input)));
    let new = || current::parse_live_whiteboard(Some(black_box(&input)));
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
            old_times[i] = elapsed(&old, repeats);
            new_times[i] = elapsed(&new, repeats);
        } else {
            new_times[i] = elapsed(&new, repeats);
            old_times[i] = elapsed(&old, repeats);
        }
    }
    old_times.sort_unstable();
    new_times.sort_unstable();
    let (old_board, old_alloc) = allocation::tracked(old);
    let (new_board, new_alloc) = allocation::tracked(new);
    let bytes = serde_json::to_vec(&new_board).unwrap();
    assert_eq!(serde_json::to_vec(&old_board).unwrap(), bytes);
    let result = new_board.as_ref().map(|b| (b.nodes.len(), b.edges.len()));
    println!("{label}: result {result:?} / {} bytes; {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; retained output {} -> {}",
        bytes.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
        old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested,
        old_alloc.peak, new_alloc.peak, old_alloc.live, new_alloc.live);
}

fn main() {
    println!("Actual board parser and frozen predecessor; {}; 3 warmup + 9 alternating batches, median per call.",
        if cfg!(debug_assertions) { "default dev profile" } else { "optimized build" });
    println!("Includes JSON field conversion, normalization, dedup and full owned result. Input JSON creation/decoding, result destruction/equality, model/network, app/UI/RSS/GPU and allocator/realloc internals excluded; allocation samples separate and include owned output.");
    for count in [0, 16, 96, 512, 4096] {
        run(
            fixtures::board(count, 1),
            &format!("{count} input nodes"),
            if count < 512 { 30 } else { 3 },
        );
    }
    let mut large = fixtures::board(16, 1);
    for node in large["nodes"].as_array_mut().unwrap() {
        node["label"] = serde_json::json!("日本語 👩🏽‍💻 ".repeat(4000));
        node["detail"] = node["label"].clone();
        node["source_excerpt"] = node["label"].clone();
        node["external_source"] = node["label"].clone();
    }
    run(large, "16 nodes with long model fields", 3);
}
