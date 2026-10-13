//! A `GlobalAlloc` that tracks how many bytes are live at once.
//!
//! Peak live bytes answer a question wall-clock timing cannot: whether a
//! change makes the native bridge hold more of a model in memory. The counts
//! come from the allocator rather than the OS, so they exclude allocator
//! bookkeeping and stay comparable across machines.
//!
//! Each thread keeps its own running total and folds it into the shared
//! figures periodically rather than charging on every call. Charging on every
//! call makes each allocation a contended read-modify-write across the mesher's
//! worker threads, and the suite then measures its own instrumentation: one
//! build streams in seconds and takes hundreds of seconds under the meter. The
//! thread that ends holding a partial total pays it on the way out, so the live
//! figure cannot stall above the truth.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering};

pub struct AllocationMeter;

/// Calls a thread charges before folding its running total in.
const CALLS_PER_FOLD: usize = 1024;

/// Bytes a thread may leave uncharged, so a large buffer is never invisible.
const BYTES_PER_FOLD: isize = 16 * 1024;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

/// One thread's uncharged allocations: bytes still held, and calls seen.
struct Uncharged {
    bytes: Cell<isize>,
    calls: Cell<usize>,
}

impl Drop for Uncharged {
    fn drop(&mut self) {
        let bytes = self.bytes.replace(0);
        if bytes != 0 {
            fold(bytes);
        }
    }
}

thread_local! {
    static UNCHARGED: Uncharged = const {
        Uncharged {
            bytes: Cell::new(0),
            calls: Cell::new(0),
        }
    };
}

/// Fold one thread's running total in and mark the high-water mark.
fn fold(bytes: isize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

/// Charge or credit bytes on this thread. Returns the amount that still has to
/// reach the shared figures, which is zero while the thread keeps accumulating.
#[inline]
fn pending_fold(bytes: isize) -> isize {
    UNCHARGED
        .try_with(|uncharged| {
            let pending = uncharged.bytes.get() + bytes;
            let calls = uncharged.calls.get() + 1;
            if pending.abs() >= BYTES_PER_FOLD || calls >= CALLS_PER_FOLD {
                uncharged.bytes.set(0);
                uncharged.calls.set(0);
                pending
            } else {
                uncharged.bytes.set(pending);
                uncharged.calls.set(calls);
                0
            }
        })
        .unwrap_or(bytes)
}

impl AllocationMeter {
    /// Bytes currently held by the process, less whatever each thread has yet
    /// to fold in.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed).max(0) as usize
    }

    /// The high-water mark since the last [`AllocationMeter::reset_peak`].
    pub fn peak() -> usize {
        PEAK.load(Ordering::Relaxed).max(0) as usize
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
            let pending = pending_fold(layout.size() as isize);
            if pending != 0 {
                fold(pending);
            }
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc_zeroed(layout);
        if !pointer.is_null() {
            let pending = pending_fold(layout.size() as isize);
            if pending != 0 {
                fold(pending);
            }
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        let pending = pending_fold(-(layout.size() as isize));
        if pending != 0 {
            fold(pending);
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(pointer, layout, size);
        if !next.is_null() {
            let pending = pending_fold(size as isize - layout.size() as isize);
            if pending != 0 {
                fold(pending);
            }
        }
        next
    }
}
