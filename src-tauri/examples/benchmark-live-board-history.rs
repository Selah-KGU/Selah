//! Actual full Call 2 context vs its frozen assembly. Generated records only;
//! no app, model, network, audio, user data, filesystem or native IPC starts.
use std::hint::black_box;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
pub use types::{SharedSummaryChunk, SharedTranscriptLine};
#[path = "../src/live/context_text.rs"]
#[allow(dead_code)]
mod context_text;
pub use context_text::{build_context_text, ContextPart};
#[allow(unused_imports)]
mod live {
    pub use super::types::{
        LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine, SharedSummaryChunk,
        SharedTranscriptLine,
    };
    pub use super::{build_context_text, ContextPart};
}
#[allow(dead_code)]
#[path = "../src/live/whiteboard/history.rs"]
mod current;
use current::WhiteboardContext;
#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
#[path = "../src/live/whiteboard/history/before.rs"]
mod before;
#[path = "../src/live/whiteboard/history/fixtures.rs"]
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
    println!("Actual full Call 2 user-message assembly vs frozen predecessor; {}; 3 warmups, nine alternating batch medians per call.", if cfg!(debug_assertions) { "default dev profile" } else { "optimized build" });
    println!("Includes full history, four recent term details, current terms, separators and final String. Separate System allocation samples exclude input creation, output destruction/comparison, other prompt components' formatting, async/IPC, model/network, app/UI/RSS/GPU and allocator/realloc internals.");
    for (count, repeat_text) in [(0, 0), (8, 24), (64, 24), (512, 24), (64, 4096)] {
        let mut chunks = fixtures::summaries(count, 5);
        for (index, chunk) in chunks.iter_mut().enumerate() {
            let chunk = std::sync::Arc::make_mut(chunk);
            chunk.body = format!(
                "開始{index}\n{}\n終端{index}",
                "講義の完全な本文 日本語・中文 🌕\n".repeat(repeat_text)
            );
        }
        let terms = chunks
            .last()
            .map(|chunk| chunk.terms.as_slice())
            .unwrap_or(&[]);
        let context = WhiteboardContext {
            course: "講義: 履歴 fixture\n授業コード: fixture",
            summaries: &chunks,
            latest_board: "主題: 前の累積白板\n- 内容全件",
            body: "今回の全文 👩🏽‍💻",
            terms,
            range: "10:00-10:10",
            transcript: ContextPart::Text("- [10:00] 今回の全文の発話\n"),
        };
        let old = || before::build(black_box(&context));
        let new = || context.build();
        for _ in 0..3 {
            assert_eq!(old(), new());
        }
        let repeats = if repeat_text > 24 { 3 } else { 50 };
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
        assert_eq!(new_alloc.calls, 1);
        assert_eq!(new_text.capacity(), new_text.len());
        println!("{count} chunks / {repeat_text} body repeats / {} output bytes: {:.6} -> {:.6} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}",
            new_text.len(), old_times[4].as_secs_f64()*1000.0, new_times[4].as_secs_f64()*1000.0,
            old_alloc.calls, new_alloc.calls, old_alloc.requested, new_alloc.requested, old_alloc.peak, new_alloc.peak);
    }
}
