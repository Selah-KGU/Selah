//! Actual LIVE chunk/overall/TODO user text vs its immediately preceding
//! assembly. Generated records only: no app, model, IO, IPC or microphone.
use std::hint::black_box;
use std::time::{Duration, Instant};
#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
pub use types::{SharedSummaryChunk, SharedTranscriptLine};
#[path = "../src/live/context_text.rs"]
mod context_text;
mod live {
    pub use super::context_text::{build_context_text, ContextPart};
    pub use super::types::{
        LiveCourseInfo, LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine,
        SharedSummaryChunk, SharedTranscriptLine,
    };
}
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/live/generation/request_text/before.rs"]
mod before;
#[path = "../src/live/generation/request_text.rs"]
mod current;
#[path = "../src/live/generation/request_text/fixtures.rs"]
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

fn compare(label: &str, old: impl Fn() -> String, new: impl Fn() -> String, repeats: usize) {
    for _ in 0..3 {
        assert_eq!(old(), new());
    }
    let mut old_times = [Duration::ZERO; 9];
    let mut new_times = [Duration::ZERO; 9];
    for index in 0..9 {
        if index % 2 == 0 {
            old_times[index] = elapsed(&old, repeats);
            new_times[index] = elapsed(&new, repeats);
        } else {
            new_times[index] = elapsed(&new, repeats);
            old_times[index] = elapsed(&old, repeats);
        }
    }
    old_times.sort_unstable();
    new_times.sort_unstable();
    let (old_text, old_alloc) = allocation::tracked(old);
    let (new_text, new_alloc) = allocation::tracked(new);
    assert_eq!(old_text.as_bytes(), new_text.as_bytes());
    assert_eq!(old_alloc.live, old_text.capacity());
    assert_eq!(new_alloc.live, new_text.capacity());
    assert_eq!(new_text.capacity(), new_text.len());
    assert_eq!(new_alloc.calls, 1);
    println!("{label} / {} output bytes: {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}",
        new_text.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
        old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested, old_alloc.peak, new_alloc.peak);
}

fn main() {
    println!("Actual LIVE user-text assembly vs immediate predecessor; {}; 3 warmups, nine alternating batch medians per call.", if cfg!(debug_assertions) { "default dev profile" } else { "optimized build" });
    println!("Includes all selected transcript/summary text, final String and overall/TODO course headers/notes. Chunk's already-formatted course header/note and TODO plan are inputs. Allocation samples exclude fixture preparation, output destruction/comparison, config/system prompt, AI/network, async/IPC, app/UI/RSS/GPU and allocator/realloc internals.");
    for (line_count, summary_count, repeat_text) in [
        (0, 0, 0),
        (80, 8, 24),
        (10000, 64, 24),
        (1000, 512, 24),
        (8, 6, 4096),
    ] {
        let mut input = fixtures::input(line_count, summary_count, 5);
        for (index, line) in input.lines.iter_mut().enumerate() {
            std::sync::Arc::make_mut(line).text = format!(
                "line{index}: {} endline{index}",
                "発話の全文 日本語・中文 🌕\n".repeat(repeat_text)
            );
        }
        for (index, chunk) in input.summaries.iter_mut().enumerate() {
            std::sync::Arc::make_mut(chunk).body = format!(
                "body{index}: {} endbody{index}",
                "全要約の本文 日本語・中文 👩🏽‍💻\n".repeat(repeat_text)
            );
        }
        let repeats = if line_count > 1000 || repeat_text > 24 {
            3
        } else {
            25
        };
        let suffix =
            format!("{line_count} lines, {summary_count} summaries, {repeat_text} text repeats");
        compare(
            &format!("chunk / {suffix}"),
            || {
                before::chunk(
                    "講義 fixture",
                    black_box(&input.summaries),
                    &input.lines,
                    "元の注記",
                )
            },
            || {
                current::chunk(
                    "講義 fixture",
                    black_box(&input.summaries),
                    &input.lines,
                    "元の注記",
                )
            },
            repeats,
        );
        compare(
            &format!("overall / {suffix}"),
            || before::overall(&input.course, black_box(&input.lines), &input.summaries),
            || current::overall(&input.course, black_box(&input.lines), &input.summaries),
            repeats,
        );
        compare(
            &format!("todo / {suffix}"),
            || {
                before::todo(
                    &input.course,
                    black_box(&input.lines),
                    &input.summaries,
                    "授業計画 fixture",
                )
            },
            || {
                current::todo(
                    &input.course,
                    black_box(&input.lines),
                    &input.summaries,
                    "授業計画 fixture",
                )
            },
            repeats,
        );
    }
}
