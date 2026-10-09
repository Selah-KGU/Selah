//! Full source enrichment vs the immediately preceding indexed algorithm.
//! Generated inputs only; no app, model, microphone, storage or IPC.
use std::hint::black_box;
use std::time::{Duration, Instant};
#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
mod live {
    pub use super::types::LiveTranscriptLine;
}
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/whiteboard/excerpts/before.rs"]
mod before;
#[path = "../src/live/whiteboard/excerpts.rs"]
mod current;
#[path = "../src/live/whiteboard/excerpts/fixtures.rs"]
mod fixtures;

fn elapsed<R>(build: &impl Fn() -> R, repeats: usize) -> Duration {
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
fn medians<R>(old: &impl Fn() -> R, new: &impl Fn() -> R, repeats: usize) -> (Duration, Duration) {
    for _ in 0..3 {
        black_box(old());
        black_box(new());
    }
    let mut old_times = [Duration::ZERO; 9];
    let mut new_times = [Duration::ZERO; 9];
    for index in 0..9 {
        if index % 2 == 0 {
            old_times[index] = elapsed(old, repeats);
            new_times[index] = elapsed(new, repeats);
        } else {
            new_times[index] = elapsed(new, repeats);
            old_times[index] = elapsed(old, repeats);
        }
    }
    old_times.sort_unstable();
    new_times.sort_unstable();
    (old_times[4], new_times[4])
}

fn main() {
    println!("Actual full source enrichment vs frozen indexed predecessor; {}; 3 warmups, nine alternating batch medians.", if cfg!(debug_assertions) {"default dev profile"} else {"optimized build"});
    println!("Includes model-board clone to own both measured outputs, inheritance indexes, terms, lazy speech index, matching and final excerpts. Separate System samples include returned owned board; exclude fixture creation, JSON comparison/output destruction, parsing, model/IPC, app/UI/RSS/GPU and allocator/realloc internals. Already-normalized speech refers to borrowed input; zero normalization allocations do not mean zero input memory.");
    let mut cases = Vec::new();
    for (name, nodes, lines) in [
        ("empty", 0, 0),
        ("small", 8, 24),
        ("matching", 75, 500),
        ("long speech", 75, 5000),
        ("already normalized", 75, 5000),
    ] {
        let mut input = fixtures::fixture(nodes, lines, 1);
        input.terms.clear();
        for node in &mut input.board.nodes {
            node.source_type = "lecture".into();
            node.node_type = "structure".into();
            node.source_excerpt.clear();
        }
        if name == "already normalized" {
            for (index, line) in input.lines.iter_mut().enumerate() {
                std::sync::Arc::make_mut(line).text =
                    format!("{index}一次資料方法論api{}abc1école👩🏽‍💻字幕の原文", index % 7);
            }
        }
        cases.push((name, input, false));
    }
    let mut supplied = fixtures::fixture(512, 500, 1);
    supplied.previous = fixtures::fixture(4096, 0, 1).previous;
    for node in &mut supplied.board.nodes {
        node.source_excerpt = "supplied citation 日本語 👩🏽‍💻".into();
    }
    cases.push(("supplied / previous 4096 nodes", supplied, true));
    let mut inherited = fixtures::fixture(512, 500, 1);
    for node in &mut inherited.board.nodes {
        node.source_excerpt.clear();
        node.source_type = "lecture".into();
    }
    for node in &mut inherited.previous.nodes {
        node.source_excerpt = " inherited API 日本語 👩🏽‍💻 ".repeat(256);
    }
    cases.push(("inherited long sources", inherited, true));
    for (name, input, inherit) in cases {
        let previous = inherit.then_some(&input.previous);
        let old = || {
            before::enrich_whiteboard_source_excerpts(
                Some(black_box(&input.board).clone()),
                previous,
                &input.terms,
                &input.lines,
            )
        };
        let new = || {
            current::enrich_whiteboard_source_excerpts(
                Some(black_box(&input.board).clone()),
                previous,
                &input.terms,
                &input.lines,
            )
        };
        let times = medians(&old, &new, if input.lines.len() >= 500 { 1 } else { 10 });
        let (old_output, old_alloc) = allocation::tracked(old);
        let (new_output, new_alloc) = allocation::tracked(new);
        let old_bytes = serde_json::to_vec(&old_output).unwrap();
        let new_bytes = serde_json::to_vec(&new_output).unwrap();
        assert_eq!(old_bytes, new_bytes);
        if name == "supplied / previous 4096 nodes" {
            let (_, clone_only) = allocation::tracked(|| input.board.clone());
            assert_eq!(new_alloc.calls, clone_only.calls);
            assert_eq!(new_alloc.requested, clone_only.requested);
            assert_eq!(new_alloc.peak, clone_only.peak);
            assert_eq!(new_alloc.live, clone_only.live);
        }
        println!("{name} / {} nodes, {} lines / {} output bytes: {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; returned requested live bytes {} -> {}",input.board.nodes.len(),input.lines.len(),new_bytes.len(),times.0.as_secs_f64()*1000.0,times.1.as_secs_f64()*1000.0,old_alloc.calls,new_alloc.calls,old_alloc.requested,new_alloc.requested,old_alloc.peak,new_alloc.peak,old_alloc.live,new_alloc.live);
    }
}
