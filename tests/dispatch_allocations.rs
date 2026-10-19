//! Dispatching a ready source must not allocate once the loop's buffers have grown to fit:
//! a GUI event loop dispatches once per frame, and an allocation per wake shows up in every
//! frame's profile.

#![cfg(unix)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use calloop::generic::Generic;
use calloop::{EventLoop, Interest, Mode, PostAction};

/// Counts the allocations made on a thread while it has counting switched on.
struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

fn count() {
    if COUNTING.with(Cell::get) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn dispatching_a_ready_source_does_not_allocate() {
    let mut event_loop = EventLoop::<u32>::try_new().unwrap();
    let (mut writer, reader) = UnixStream::pair().unwrap();
    // One byte that is never read: the source stays readable, so every dispatch wakes on it.
    writer.write_all(&[1]).unwrap();
    event_loop
        .handle()
        .insert_source(
            Generic::new(reader, Interest::READ, Mode::Level),
            |_, _, woken: &mut u32| {
                *woken += 1;
                Ok(PostAction::Continue)
            },
        )
        .unwrap();
    let mut woken = 0;
    // The first dispatch may size the loop's buffers.
    event_loop
        .dispatch(Some(Duration::ZERO), &mut woken)
        .unwrap();

    COUNTING.with(|counting| counting.set(true));
    for _ in 0..1000 {
        event_loop
            .dispatch(Some(Duration::ZERO), &mut woken)
            .unwrap();
    }
    COUNTING.with(|counting| counting.set(false));

    assert_eq!(woken, 1001, "every dispatch woke on the ready source");
    assert_eq!(
        ALLOCATIONS.load(Ordering::Relaxed),
        0,
        "1000 dispatches of a ready source allocated"
    );
}
