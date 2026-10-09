//! Isolated production thinking-filter benchmark. No app/network/model/audio.
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/agent_text.rs"]
mod agent_text;
#[path = "support/allocation.rs"]
mod allocation;
#[path = "../src/agent_provider/stream/think_filter/before.rs"]
mod before;
#[path = "../src/agent_provider/stream/think_filter.rs"]
mod current;

#[derive(Default)]
struct Sink {
    hash: Sha256,
    calls: usize,
    bytes: usize,
}
#[derive(Debug, PartialEq, Eq)]
struct Output {
    hash: [u8; 32],
    calls: usize,
    bytes: usize,
}
fn filter(chunks: &[(String, bool)], previous: bool) -> Output {
    let sink = Arc::new(Mutex::new(Sink::default()));
    let captured = Arc::clone(&sink);
    let callback = move |text: &str, thinking: bool| {
        let mut sink = captured.lock().unwrap();
        sink.hash.update([u8::from(thinking)]);
        sink.hash.update(text.len().to_le_bytes());
        sink.hash.update(text.as_bytes());
        sink.calls += 1;
        sink.bytes += text.len();
    };
    let (mut feed, mut flush) = if previous {
        before::ThinkFilter::wrap_with_flush(callback)
    } else {
        current::ThinkFilter::wrap_with_flush(callback)
    };
    for (text, think) in chunks {
        feed(text, *think);
    }
    flush();
    drop(feed);
    drop(flush);
    let sink = Arc::try_unwrap(sink).ok().unwrap().into_inner().unwrap();
    Output {
        hash: sink.hash.finalize().into(),
        calls: sink.calls,
        bytes: sink.bytes,
    }
}
fn elapsed(build: impl FnOnce() -> Output) -> Duration {
    let start = Instant::now();
    let value = black_box(build());
    let elapsed = start.elapsed();
    black_box(value);
    elapsed
}
fn run(chunks: &[(String, bool)], label: &str) {
    let old = || filter(chunks, true);
    let new = || filter(chunks, false);
    for _ in 0..3 {
        assert_eq!(old(), new());
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
    assert_eq!(old_value, new_value);
    assert_eq!(old_alloc.live, 0);
    assert_eq!(new_alloc.live, 0);
    if chunks.len() >= 1000 {
        assert!(old_alloc.calls > chunks.len() / 2);
        assert!(new_alloc.calls < 100);
    }
    println!("{label}: {} chunks / {} callbacks / {} output bytes: {:.3} -> {:.3} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}",
        chunks.len(), old_value.calls, old_value.bytes,
        old_times[4].as_secs_f64()*1000.0,new_times[4].as_secs_f64()*1000.0,
        old_alloc.calls,new_alloc.calls,old_alloc.requested,new_alloc.requested,old_alloc.peak,new_alloc.peak);
}
fn main() {
    println!("Actual thinking filter vs frozen predecessor; default dev profile; input setup excluded; 9 alternating runs after warmup.");
    println!("Includes wrapper Arc/Mutex/closures, tag filtering and ordered SHA-256 sink; allocations tracked separately. Excludes SSE/JSON/network/models/app/UI/RSS/GPU, inputs/allocator/realloc internals.");
    for count in [0, 1000, 10_000] {
        let plain: Vec<_> = (0..count)
            .map(|i| (format!("{i} 完全な回答 👩🏽‍💻 "), false))
            .collect();
        run(&plain, "visible Unicode");
        let mixed: Vec<_> = (0..count)
            .map(|i| match i % 8 {
                0 => ("回答 <thou".into(), false),
                1 => ("ght>推論 中文 🌕 ".into(), false),
                2 => ("ネイティブな思考 <think> ".into(), true),
                3 => ("途中</thought>回答 ".into(), false),
                4 => ("<thinking>別の思考 👩🏽‍💻".into(), false),
                5 => ("</thinking>続き ".into(), false),
                6 => ("<think>終わり".into(), false),
                _ => ("</think>最後の回答 ".into(), false),
            })
            .collect();
        run(&mixed, "mixed inline/upstream thinking");
    }
    let large = "完整的段落 👩🏽‍💻\n".repeat(10_000);
    run(&[(large, false)], "one large visible chunk");
}
