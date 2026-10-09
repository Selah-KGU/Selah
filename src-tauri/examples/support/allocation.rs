//! Benchmark-only tracker. Never installed in the app or library test executable.
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

// This allocator is installed only in this standalone example, never in the
// application or test library. Inputs/warmup are outside tracking; the measured
// function only borrows them. Returned allocations, if any, remain live for the
// sample. Requested bytes exclude allocator metadata/RSS and realloc internals.
struct TrackingAllocator;
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;
static TRACKING: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    CALLS.fetch_add(1, Relaxed);
    REQUESTED.fetch_add(size, Relaxed);
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
}
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the unchanged allocation contract to System.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && TRACKING.load(Relaxed) {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the unchanged allocation contract to System.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && TRACKING.load(Relaxed) {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACKING.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        // SAFETY: only delegate the caller's original pointer/layout.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: preserve pointer/layout/new-size and System's ownership rules.
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() && TRACKING.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
            allocated(size);
        }
        next
    }
}

#[derive(Debug)]
pub(super) struct AllocationSample {
    pub(super) calls: usize,
    pub(super) requested: usize,
    pub(super) peak: usize,
    pub(super) live: usize,
}
pub(super) fn tracked<R>(build: impl FnOnce() -> R) -> (R, AllocationSample) {
    for counter in [&CALLS, &REQUESTED, &LIVE, &PEAK] {
        counter.store(0, Relaxed);
    }
    TRACKING.store(true, Relaxed);
    let output = black_box(build());
    TRACKING.store(false, Relaxed);
    let sample = AllocationSample {
        calls: CALLS.load(Relaxed),
        requested: REQUESTED.load(Relaxed),
        peak: PEAK.load(Relaxed),
        live: LIVE.load(Relaxed),
    };
    (output, sample)
}
