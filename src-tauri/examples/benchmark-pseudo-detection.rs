//! Actual pseudo-call detectors vs frozen predecessors; synthetic text only.
use std::fmt::Debug;
use std::hint::black_box;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/agent_text.rs"]
mod agent_text;
#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
#[path = "../src/agent_pseudo_call/detection/before.rs"]
mod before;
#[path = "../src/agent_pseudo_call/detection.rs"]
mod current;

fn elapsed<R>(build: &impl Fn() -> R) -> Duration {
    let start = Instant::now();
    let output = black_box(build());
    let time = start.elapsed();
    black_box(output);
    time
}

fn run<R: Eq + Debug>(
    text: &str,
    label: &str,
    operation: &str,
    old: impl Fn() -> R,
    new: impl Fn() -> R,
    new_allocation_limit: usize,
) {
    for _ in 0..3 {
        assert_eq!(old(), new());
    }
    let mut old_times = [Duration::ZERO; 9];
    let mut new_times = [Duration::ZERO; 9];
    for i in 0..9 {
        if i % 2 == 0 {
            old_times[i] = elapsed(&old);
            new_times[i] = elapsed(&new);
        } else {
            new_times[i] = elapsed(&new);
            old_times[i] = elapsed(&old);
        }
    }
    old_times.sort_unstable();
    new_times.sort_unstable();
    let (old_value, old_alloc) = allocation::tracked(old);
    let (new_value, new_alloc) = allocation::tracked(new);
    assert_eq!(old_value, new_value);
    assert_eq!(old_alloc.live, 0);
    assert_eq!(new_alloc.live, 0);
    assert!(new_alloc.calls <= new_allocation_limit);
    println!("{label} / {operation}: {} bytes; result {new_value:?}; {:.3} -> {:.3} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}",
        text.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
        old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested,
        old_alloc.peak, new_alloc.peak);
}

fn scan(text: &str, label: &str, thinking: bool) {
    run(
        text,
        label,
        "find_start",
        || before::find_start(black_box(text)),
        || current::find_start(black_box(text)),
        0,
    );
    run(
        text,
        label,
        "has_any",
        || before::has_any(black_box(text)),
        || current::has_any(black_box(text)),
        usize::from(thinking),
    );
    run(
        text,
        label,
        "leading",
        || before::contains_leading(black_box(text)),
        || agent_text::contains_leading_pseudo_tool_call(black_box(text)),
        0,
    );
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "default dev profile"
    } else {
        "optimized build"
    };
    println!("Production pseudo-call detectors vs frozen predecessor; {profile}; 3 warmup + 9 alternating samples; allocation tracking separate.");
    println!("Includes UTF-8 scanning, marker detection, and thinking stripping for has_any. Excludes input setup, SSE/JSON/tool argument parsing, IO/network/models, app/UI/RSS/GPU and allocator/realloc internals.");
    for count in [128, 512, 2048] {
        let text = "普通の回答🙂 explanation words\n".repeat(count);
        scan(&text, &format!("normal words x{count}"), false);
        let tail = format!("{text}‹TaSk_CaLl：read_file {{\"path\":\"/tmp/文.pdf\"}}›");
        scan(&tail, &format!("marker at tail x{count}"), false);
    }
    for count in [32, 128, 512] {
        let text = format!("{}ordinary", " ‹`〈<\u{3000}".repeat(count));
        scan(&text, &format!("wrapper run x{count}"), false);
    }
    let wrapped = format!("{}CALL：read_file", " ‹`〈<\u{3000}".repeat(512));
    scan(&wrapped, "leading wrapped marker", false);
    let thinking = format!(
        "<think>{}</think>{}",
        "hidden call:x ".repeat(2048),
        "普通の回答🙂 explanation words\n".repeat(512)
    );
    scan(&thinking, "tagged answer", true);
    scan("", "empty", false);
}
