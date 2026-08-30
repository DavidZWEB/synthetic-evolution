//! Guards the memory a world commits up front.
//!
//! Every pool and arena is allocated at `max_agents` on `World::new` and never grown,
//! because growing WASM memory detaches the JS views over the snapshot (spec §7.3).
//! That makes the footprint a fixed, up-front cost in a browser tab rather than
//! something that grows with the population — and it scales with genome size, so it
//! moves whenever the founding topology does.
//!
//! This is a smoke alarm, not a budget. It fires when a change makes the default world
//! much more expensive, which is the kind of thing nobody notices until a phone runs
//! out of memory on a shared link.
//!
//! It has fired once already, in the sense that matters: M5's compiled brains took the
//! default world from 57 MB to 71 MB, which is a quarter more for something that reads
//! like an implementation detail, and M6's compiled organs added 2 MB more. The
//! breakdown lives on `arena`'s module doc.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use sim_core::params::SimParams;
use sim_core::world::World;

// Per-thread, not global: tests in this binary run in parallel, and a shared counter
// means each one zeroes the others' measurement mid-flight. The same reason
// `no_alloc.rs` counts per-thread.
thread_local! {
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = BYTES.try_with(|b| b.set(b.get() + size));
}

fn take() -> usize {
    BYTES.with(|b| b.replace(0))
}

struct Tally;

unsafe impl GlobalAlloc for Tally {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Tally = Tally;

const MEGABYTE: usize = 1_048_576;
/// Comfortably above today's ~73 MB and well under what a phone will tolerate.
const CEILING_MB: usize = 96;

#[test]
fn a_default_world_fits_in_a_browser_tab() {
    take();
    let world = World::new(1, SimParams::default()).expect("defaults are valid");
    let used = take() / MEGABYTE;
    std::hint::black_box(&world);
    assert!(
        used <= CEILING_MB,
        "a default world commits {used} MB, over the {CEILING_MB} MB ceiling. \
         Either the genome grew or max_agents did — both are real costs, so raise the \
         ceiling deliberately rather than by reflex."
    );
}

#[test]
fn footprint_scales_with_the_pool_not_the_population() {
    // Doubling capacity should roughly double the commitment, with no dependence on
    // how many agents are actually spawned. If this stops holding, something started
    // allocating lazily and the no-grow guarantee is gone.
    let measure = |agents: u32| {
        let mut params = SimParams::default();
        params.world.max_agents = agents;
        take();
        let world = World::new(1, params).expect("valid params");
        let used = take();
        std::hint::black_box(&world);
        used
    };

    let small = measure(1_000);
    let large = measure(2_000);
    let ratio = large as f64 / small as f64;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "capacity doubled but memory changed by {ratio:.2}x"
    );
}
