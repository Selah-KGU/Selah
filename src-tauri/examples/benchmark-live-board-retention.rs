//! Actual board carry-forward and guard selection vs frozen owned predecessor.
//! Synthetic immutable diagrams only; no app, model, IO, recording or IPC.
use std::hint::black_box;
use std::time::{Duration, Instant};
#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
#[allow(unused_imports)]
mod live {
    pub use super::types::{
        LiveSummaryChunk, LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode,
        SharedSummaryChunk,
    };
}
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/ai_output/reconcile/before.rs"]
mod before;
#[allow(dead_code, unused_imports)]
#[path = "../src/live/ai_output/context.rs"]
mod context;
#[path = "../src/live/ai_output/reconcile.rs"]
mod current;
#[path = "../src/live/ai_output/reconcile/fixtures.rs"]
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
fn medians<A, B>(
    old: &impl Fn() -> A,
    new: &impl Fn() -> B,
    repeats: usize,
) -> (Duration, Duration) {
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
fn show(
    label: &str,
    times: (Duration, Duration),
    old: &allocation::AllocationSample,
    new: &allocation::AllocationSample,
) {
    println!("{label}: {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; returned requested live bytes {} -> {}", times.0.as_secs_f64()*1000.0,times.1.as_secs_f64()*1000.0,old.calls,new.calls,old.requested,new.requested,old.peak,new.peak,old.live,new.live);
}
fn main() {
    println!("Actual immutable board retention vs owned predecessor; {}; 3 warmups, nine alternating batch medians.", if cfg!(debug_assertions) { "default dev profile" } else { "optimized build" });
    println!("Includes carry-forward with existing diagnostics, retained diagram collections, or borrowed guard predicates. Separate System samples exclude input diagrams, equality/JSON comparison, output destruction, parsing/enrichment, full summary metadata, model/IPC, app/UI/RSS/GPU and allocator/realloc internals. Shared output refers to the existing input allocation; zero sample bytes do not mean zero board memory.");
    for count in [16, 96, 512, 4096] {
        let previous: SharedWhiteboard = fixtures::board(count, 1).into();
        let old = || before::reconcile_whiteboard(Some(black_box(previous.as_ref())), None);
        let new = || current::reconcile_whiteboard(Some(black_box(&previous)), None);
        let times = medians(&old, &new, if count > 512 { 10 } else { 50 });
        let (old_output, old_alloc) = allocation::tracked(old);
        let (new_output, new_alloc) = allocation::tracked(new);
        assert_eq!(
            serde_json::to_vec(&old_output).unwrap(),
            serde_json::to_vec(&new_output).unwrap()
        );
        assert!(std::sync::Arc::ptr_eq(
            new_output.as_ref().unwrap(),
            &previous
        ));
        assert_eq!(new_alloc.calls, 0);
        assert_eq!(new_alloc.live, 0);
        show(
            &format!("carry / {count} nodes, {} edges", previous.edges.len()),
            times,
            &old_alloc,
            &new_alloc,
        );
    }
    let previous: SharedWhiteboard = fixtures::board(512, 1).into();
    let old = || {
        (0..32)
            .map(|_| before::reconcile_whiteboard(Some(previous.as_ref()), None).unwrap())
            .collect::<Vec<_>>()
    };
    let new = || {
        (0..32)
            .map(|_| current::reconcile_whiteboard(Some(&previous), None).unwrap())
            .collect::<Vec<_>>()
    };
    let times = medians(&old, &new, 3);
    let (old_output, old_alloc) = allocation::tracked(old);
    let (new_output, new_alloc) = allocation::tracked(new);
    assert_eq!(
        serde_json::to_vec(&old_output).unwrap(),
        serde_json::to_vec(&new_output).unwrap()
    );
    assert_eq!(new_alloc.calls, 1);
    assert_eq!(
        new_alloc.live,
        new_output.capacity() * std::mem::size_of::<SharedWhiteboard>()
    );
    assert!(new_output
        .iter()
        .all(|board| std::sync::Arc::ptr_eq(board, &previous)));
    show(
        "retained / 32 versions of 512 nodes",
        times,
        &old_alloc,
        &new_alloc,
    );
    for (name, mut model) in [
        ("same", fixtures::board(4096, 1)),
        ("growing", fixtures::board(4097, 1)),
        ("shrunk", fixtures::board(3, 1)),
        ("edge churn", fixtures::board(4096, 1)),
    ] {
        if name == "edge churn" {
            model.edges.clear();
        }
        let previous = fixtures::board(4096, 1);
        let old = || before::should_keep_previous_whiteboard(black_box(&previous), &model);
        let new = || current::should_keep_previous_whiteboard(black_box(&previous), &model);
        let times = medians(&old, &new, 10);
        let (old_result, old_alloc) = allocation::tracked(old);
        let (new_result, new_alloc) = allocation::tracked(new);
        assert_eq!(old_result, new_result);
        assert_eq!(old_alloc.live, 0);
        assert_eq!(new_alloc.live, 0);
        if name == "growing" {
            assert_eq!(new_alloc.calls, 0);
        }
        show(&format!("guard / {name}"), times, &old_alloc, &new_alloc);
    }
}
