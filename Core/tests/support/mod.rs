//! A `GlobalAlloc` that tracks how many bytes are live at once.
//!
//! Peak live bytes answer a question wall-clock timing cannot: whether a
//! change makes the native bridge hold more of a model in memory. The counts
//! come from the allocator rather than the OS, so they exclude allocator
//! bookkeeping and stay comparable across machines.

#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct AllocationMeter;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// Charge a range the allocator actually handed back.
fn charge(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

impl AllocationMeter {
    /// Bytes currently held by the process.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed)
    }

    /// The high-water mark since the last [`AllocationMeter::reset_peak`].
    pub fn peak() -> usize {
        PEAK.load(Ordering::Relaxed)
    }

    /// Start a fresh high-water mark from whatever is live right now, so one
    /// measured phase excludes allocations an earlier phase left behind.
    pub fn reset_peak() {
        PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for AllocationMeter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            charge(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc_zeroed(layout);
        if !pointer.is_null() {
            charge(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(pointer, layout, size);
        if !next.is_null() {
            if size >= layout.size() {
                charge(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}
