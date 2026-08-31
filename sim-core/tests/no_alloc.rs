//! Asserts the sim allocates nothing once it is warm.
//!
//! An allocation in the hot loop does not fail a functional test — it shows up as
//! frame-time jitter in the browser and as a slow drift in an overnight headless run,
//! which is exactly the kind of thing nobody traces back to a `Vec::push`. So it is
//! measured directly (CLAUDE.md invariant 4, spec §7.8 tier 2).
//!
//! Counting is per-thread rather than global: this binary's test harness allocates on
//! its own thread while the measured region runs, and a global counter would fold that
//! noise in and flake.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use glam::Vec3;
use sim_core::ids::AgentId;
use sim_core::params::SimParams;
use sim_core::world::World;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // A realloc is a growing collection, which is the failure this test exists to
        // catch — count it like a fresh allocation.
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn note_allocation() {
    // `try_with` because TLS is unavailable during thread teardown, and panicking
    // inside the allocator would abort the process rather than fail the test.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `body` with allocation counting on, and reports how many it made.
fn count_allocations(body: impl FnOnce()) -> u64 {
    ALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    body();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|n| n.get())
}

fn at(i: u32) -> Vec3 {
    Vec3::new(i as f32 % 500.0, (i / 500) as f32, 0.0)
}

#[test]
fn the_counter_actually_counts() {
    // Without this, a broken counter makes every other assertion in the file vacuous.
    let observed = count_allocations(|| {
        let v: Vec<u64> = (0..1_000).collect();
        std::hint::black_box(&v);
    });
    assert!(
        observed > 0,
        "the counting allocator is not observing allocations"
    );
}

#[test]
fn spawn_and_despawn_never_allocate() {
    let mut params = SimParams::default();
    params.world.max_agents = 10_000;
    let mut world = World::new(42, params).expect("valid params");

    // Warmup: the first pass touches every free-list and arena path. Construction
    // allocates by design — the pools are sized once, up front.
    let mut ids: Vec<AgentId> = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        ids.push(world.spawn_founder(at(i)).expect("pool sized for 10k"));
    }
    for id in ids.drain(..) {
        world.despawn(id);
    }

    let observed = count_allocations(|| {
        for round in 0..5 {
            for i in 0..10_000u32 {
                let id = world
                    .spawn_founder(at(i + round * 10_000))
                    .expect("pool has room");
                ids.push(id);
            }
            // Churn in reverse, so the free list is exercised rather than replayed.
            while let Some(id) = ids.pop() {
                world.despawn(id);
            }
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "spawn/despawn allocated {observed} times after warmup"
    );
}

#[test]
fn rebuilding_and_querying_the_spatial_hash_never_allocates() {
    // This runs every tick for every agent — step 1 of the tick, then once per sensor
    // during perception. It is the hottest path in the simulation (spec §2.2c).
    let mut params = SimParams::default();
    params.world.max_agents = 5_000;
    let mut world = World::new(9, params).expect("valid params");
    for i in 0..5_000 {
        world.spawn_founder(at(i)).expect("pool sized for 5k");
    }

    // Warmup: first rebuild touches every bucket and cursor.
    world.rebuild_spatial_hash();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.rebuild_spatial_hash();
            let positions = &world.agents().position;
            let radius = world.params().sensing.max_sense_radius();
            let mut seen = 0u64;
            for id in 0..500u32 {
                world.spatial_hash().for_each_within(
                    positions,
                    positions[id as usize],
                    radius,
                    |_, _, _| seen += 1,
                );
            }
            std::hint::black_box(seen);
        }
    });

    assert_eq!(
        observed, 0,
        "spatial hash allocated {observed} times after warmup"
    );
}

#[test]
fn a_whole_tick_of_systems_never_allocates() {
    // Every step of spec §2.4 that exists: rebuild, perceive, think, decide, move, and
    // the field update. Perception alone is expected to be 60–80% of tick cost once
    // vision is real (spec §2.2c) — it runs a spatial-hash query per eye per agent per
    // tick — so an allocation anywhere in here is the most expensive one in the project.
    let mut params = SimParams::default();
    params.world.max_agents = 2_000;
    let mut world = World::new(13, params).expect("valid params");
    for i in 0..2_000 {
        world.spawn_founder(at(i)).expect("pool sized for 2k");
    }
    // Something to smell and something to see, or the sensors take their cheapest path.
    world.deposit_chemo(0, Vec3::new(250.0, 250.0, 0.0), 500.0);
    world.rebuild_spatial_hash();
    world.perceive_all();
    world.drive_effectors();
    world.integrate_movement();
    world.update_chemo();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.rebuild_spatial_hash();
            world.perceive_all();
            world.step_brains();
            world.drive_effectors();
            world.integrate_movement();
            world.update_chemo();
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "a tick allocated {observed} times after warmup"
    );
}

#[test]
fn stepping_every_brain_never_allocates() {
    // Step 3 of the tick, run for every agent every tick (spec §2.4). The scratch
    // buffer the Euler step writes into is owned by the world and sized once; if it
    // ever becomes a per-agent `Vec` this is what says so.
    let mut params = SimParams::default();
    params.world.max_agents = 2_000;
    let mut world = World::new(11, params).expect("valid params");
    for i in 0..2_000 {
        world.spawn_founder(at(i)).expect("pool sized for 2k");
    }

    world.step_brains();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.step_brains();
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "stepping brains allocated {observed} times after warmup"
    );
}

#[test]
fn a_full_pool_refuses_without_allocating() {
    // The interesting case: at the population ceiling, every birth is a rejection.
    let mut params = SimParams::default();
    params.world.max_agents = 64;
    let mut world = World::new(1, params).expect("valid params");
    for i in 0..64 {
        world.spawn_founder(at(i)).expect("pool sized for 64");
    }
    let observed = count_allocations(|| {
        for i in 0..10_000 {
            assert!(world.spawn_founder(at(i)).is_none());
        }
    });
    assert_eq!(observed, 0, "rejecting a birth allocated {observed} times");
}
