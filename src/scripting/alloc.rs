//! Counting global allocator: the net bytes each thread has allocated.
//!
//! A script evaluation runs on one thread, so the change of this counter
//! across the evaluation is the memory it holds: the Starlark heap's arena
//! chunks, dict tables, series data and strings alike. The runtime checks
//! it on every native call and every 1000 ticks (ADR-0024).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub struct Counting;

thread_local! {
    // Const-initialized and without a destructor, so accessing it from the
    // allocator never allocates and works during thread teardown.
    static NET: Cell<isize> = const { Cell::new(0) };
}

fn add(n: isize) {
    let _ = NET.try_with(|c| c.set(c.get().wrapping_add(n)));
}

/// Net bytes allocated by the current thread (frees of memory allocated
/// elsewhere make it drift, so only differences are meaningful).
pub fn thread_net() -> isize {
    NET.try_with(Cell::get).unwrap_or(0)
}

// SAFETY: every method forwards to `System` unchanged and only updates a
// thread-local counter, which neither allocates nor unwinds.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            add(layout.size() as isize);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            add(layout.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        add(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            add(new_size as isize - layout.size() as isize);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;
