#!/usr/bin/env bash
# Native stream delivery and accumulation; no app, UI or microphone is launched.
set -euo pipefail
selah_repo_root="$(cd "$(dirname "$0")/.." && pwd)"
selah_bench_root="$(mktemp -d "${TMPDIR:-/tmp}/selah-native-events.XXXXXX")"
trap 'rm -rf "$selah_bench_root"' EXIT
mkdir -p "$selah_bench_root/src"
cp "$selah_repo_root/src-tauri/Cargo.lock" "$selah_bench_root/Cargo.lock"
cp "$selah_repo_root/src-tauri/src/native_agent_state.rs" "$selah_bench_root/src/native_agent_state.rs"
cp "$selah_repo_root/src-tauri/src/latest_ui_mailbox.rs" "$selah_bench_root/src/latest_ui_mailbox.rs"
cp "$selah_repo_root/src-tauri/src/native_agent_events/stream.rs" "$selah_bench_root/src/native_agent_events.rs"
# Copy the production capture type; its unused platform dispatch helper is omitted.
sed '/^\/\/\/ Keep the UI intent/,$d' "$selah_repo_root/src-tauri/src/native_capture.rs" > "$selah_bench_root/src/native_capture.rs"
# SharedState also owns the production shortcut intent. Include its pure state
# type while leaving the STT status read and capture dispatch out of this bench.
sed -n '/^#\[derive(Default)\]/,/^#\[derive(Clone, Copy)\]/{ /^#\[derive(Clone, Copy)\]/!p; }' "$selah_repo_root/src-tauri/src/native_shortcut.rs" > "$selah_bench_root/src/native_shortcut.rs"
cat > "$selah_bench_root/Cargo.toml" <<'TOML'
[package]
name = "selah-native-event-benchmark"
version = "0.1.0"
edition = "2021"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
uuid = { version = "1", features = ["v4"] }
TOML
cat > "$selah_bench_root/src/main.rs" <<'RUST'
#![allow(dead_code)]
mod native_agent_state;
mod native_agent_events;
mod native_capture;
mod native_shortcut;
mod latest_ui_mailbox;
use native_agent_state::SharedState;
use native_agent_events::StreamEvent;
use serde::Deserialize;
use std::{alloc::{GlobalAlloc, Layout, System}, borrow::Cow, hint::black_box, sync::{Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Instant};
struct CountAlloc;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 { if ACTIVE.load(Ordering::Relaxed) { ALLOCS.fetch_add(1, Ordering::Relaxed); } System.alloc(layout) }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 { if ACTIVE.load(Ordering::Relaxed) { ALLOCS.fetch_add(1, Ordering::Relaxed); } System.alloc_zeroed(layout) }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 { if ACTIVE.load(Ordering::Relaxed) { ALLOCS.fetch_add(1, Ordering::Relaxed); } System.realloc(pointer, layout, size) }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) { System.dealloc(pointer, layout); }
}
#[global_allocator] static ALLOC: CountAlloc = CountAlloc;
const ITERATIONS: usize = 20_000;
struct Delivery<'a> { state: &'a Mutex<SharedState>, conversation: &'a str, request: &'a str, text: &'a str, payload: &'a str }
fn deliver(data: &Delivery<'_>, request: &str, text: Cow<'_, str>) {
    black_box(data.state.lock().unwrap_or_else(|e| e.into_inner()).accept_stream(data.conversation, request, StreamEvent::Token(text)));
}
fn value_json(data: &Delivery<'_>) {
    let value: serde_json::Value = serde_json::from_str(black_box(data.payload)).unwrap();
    if value["type"].as_str() == Some("token") {
        deliver(data, value["turn_id"].as_str().unwrap(), Cow::Borrowed(value["text"].as_str().unwrap()));
    }
}
// A borrowed JSON baseline, not a production subscriber. Incoming request IDs
// are checked identically to direct delivery; frontend serialization is excluded.
#[derive(Deserialize)]
struct TokenPayload<'a> {
    #[serde(rename = "type", borrow)] kind: Cow<'a, str>,
    #[serde(borrow)] turn_id: Cow<'a, str>,
    #[serde(borrow)] text: Cow<'a, str>,
}
fn borrowed_json(data: &Delivery<'_>) {
    let event: TokenPayload<'_> = serde_json::from_str(black_box(data.payload)).unwrap();
    if event.kind == "token" { deliver(data, &event.turn_id, event.text); }
}
fn direct(data: &Delivery<'_>) {
    deliver(data, black_box(data.request), Cow::Borrowed(black_box(data.text)));
}
fn measure(data: &Delivery<'_>, operation: fn(&Delivery<'_>), expected: &str) -> (f64, f64) {
    ACTIVE.store(false, Ordering::Relaxed);
    data.state.lock().unwrap().result_accumulated.clear();
    let start = Instant::now();
    for _ in 0..ITERATIONS { operation(data); }
    let duration = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(data.state.lock().unwrap().result_accumulated, expected);
    data.state.lock().unwrap().result_accumulated.clear();
    ALLOCS.store(0, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
    for _ in 0..ITERATIONS { operation(data); }
    ACTIVE.store(false, Ordering::Relaxed);
    let allocations = ALLOCS.load(Ordering::Relaxed) as f64 / ITERATIONS as f64;
    assert_eq!(data.state.lock().unwrap().result_accumulated, expected);
    (duration, allocations)
}
fn main() {
    println!("Native delivery + production ownership/accumulation: {ITERATIONS} tokens/trial, seven rotating trials. Allocation requests include realloc; result buffer is preallocated. Frontend serialization/delivery, UI, model, STT, CPU/RSS and GPU are excluded.");
    for (scenario, text, foreign) in [("Japanese", "授業について", false), ("escaped Unicode", "日本語\n\"quotes\"\t👩🏽‍💻", false), ("foreign request", "discarded", true)] {
        let mut state = SharedState::default();
        let owner = state.begin_stream("conversation".into()).owner;
        state.result_accumulated.reserve(text.len() * ITERATIONS);
        let state = Mutex::new(state);
        let request = if foreign { "another-request" } else { owner.request_id() };
        let payload = serde_json::to_string(&serde_json::json!({"turn_id": request, "type": "token", "text": text})).unwrap();
        let expected = if foreign { String::new() } else { text.repeat(ITERATIONS) };
        let data = Delivery { state: &state, conversation: owner.conversation_id(), request, text, payload: &payload };
        let operations: [(&str, fn(&Delivery<'_>)); 3] = [("Value JSON", value_json), ("borrowed JSON", borrowed_json), ("direct typed", direct)];
        let mut trials: [Vec<(f64, f64)>; 3] = std::array::from_fn(|_| Vec::new());
        for trial in 0..7 {
            for step in 0..3 {
                let index = (trial + step) % 3;
                trials[index].push(measure(&data, operations[index].1, &expected));
            }
        }
        for index in 0..3 {
            trials[index].sort_by(|a,b| a.0.total_cmp(&b.0));
            println!("{scenario} / {}: {:.3} ms, {:.1} allocations/token", operations[index].0, trials[index][3].0, trials[index][3].1);
        }
    }
}
RUST
CARGO_TARGET_DIR="$selah_repo_root/src-tauri/target/native-events-benchmark" cargo run --offline --release --manifest-path "$selah_bench_root/Cargo.toml"
