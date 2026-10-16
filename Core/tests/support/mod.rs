//! Peak live bytes measure whether a change makes the native bridge hold more
//! of a model in memory. The counts come from the allocator, so they exclude
//! allocator bookkeeping and stay comparable across machines.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering};

pub struct AllocationMeter;

const CALLS_PER_FOLD: usize = 1024;

/// Maximum bytes one thread holds uncharged. An allocation that passes this
/// limit triggers an immediate fold.
const BYTES_PER_FOLD: isize = 16 * 1024;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

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

fn fold(bytes: isize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

/// Charges or credits bytes to the running total on this thread. The total
/// folds into the shared counters when it passes the [`BYTES_PER_FOLD`] or
/// [`CALLS_PER_FOLD`] limit. Returns the amount that the caller must fold now,
/// or zero.
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
    /// Bytes the process currently holds, less the uncharged total on each
    /// thread.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed).max(0) as usize
    }

    /// The high-water mark since the last [`AllocationMeter::reset_peak`].
    pub fn peak() -> usize {
        PEAK.load(Ordering::Relaxed).max(0) as usize
    }

    /// Sets a fresh high-water mark at the bytes that are live now, so one
    /// measured phase excludes allocations from an earlier phase.
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
