//! The golden scenario, shared verbatim by the native golden test and the wasm
//! cross-target test.
//!
//! One definition on purpose. The criterion is that **native and wasm agree**, and two
//! copies of a scenario decay into each target agreeing with itself — which is what the
//! criterion exists to rule out. Included by path rather than imported because a test
//! fixture is not part of `sim-core`'s public surface and should not become one.

use glam::Vec3;
use sim_core::params::SimParams;
use sim_core::world::World;

/// The shipped defaults, 200 founders, 300 ticks.
pub const SHIPPED_GOLDEN: u64 = 0xea06_56fd_0f18_2485;

/// A configuration that reproduces, 200 founders, 500 ticks.
pub const BREEDING_GOLDEN: u64 = 0x0cae_eeb2_e480_8e78;

pub const FOUNDERS: u32 = 200;
pub const SHIPPED_TICKS: u64 = 300;
pub const BREEDING_TICKS: u64 = 500;

/// The shipped configuration, with only the pool sized down from 5000.
pub fn shipped() -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = 512;
    params
}

/// A configuration that actually reproduces, so the golden run covers the birth path.
/// Measured at M7: the defaults produce no births at all, and a hash over a population
/// that only ever starves would not pin `resolve_births`.
pub fn breeding() -> SimParams {
    let mut params = shipped();
    params.body.size = 1.5;
    params.metabolism.k_sensor = 0.003_125;
    params.feeding.reach = 8.0;
    // Brought forward so births land inside a short run. At the shipped 300 and 150 the
    // first birth arrives past tick 2000, which would make this a slow test that spent
    // most of its time not covering the thing it exists to cover.
    params.reproduction.maturity_ticks = 100;
    params.reproduction.threshold = 120.0;
    params
}

/// Founders on a golden-angle spiral, so the layout is a pure function of the count and
/// carries no accidental symmetry.
pub fn seeded(seed: u64, params: SimParams, founders: u32) -> World {
    let mut world = World::new(seed, params).expect("valid params");
    let size = world.params().world.size;
    for i in 0..founders {
        let a = i as f32 * 2.399_963_2;
        let r = size * 0.4 * (i as f32 / founders as f32);
        world
            .spawn_founder(Vec3::new(
                size * 0.5 + r * sim_core::math::cos(a),
                size * 0.5 + r * sim_core::math::sin(a),
                0.0,
            ))
            .expect("pool has room");
    }
    world
}

pub fn advanced(seed: u64, params: SimParams, founders: u32, ticks: u64) -> World {
    let mut world = seeded(seed, params, founders);
    for _ in 0..ticks {
        world.step();
    }
    world
}

pub fn run(seed: u64, params: SimParams, founders: u32, ticks: u64) -> u64 {
    advanced(seed, params, founders, ticks).state_hash()
}

/// Whether anything was born during the run. An agent present since tick 0 has an age
/// equal to the tick count; anything younger arrived through `resolve_births`.
pub fn saw_a_birth(world: &World, ticks: u64) -> bool {
    world
        .pool()
        .iter_live()
        .any(|id| (world.agents().age[id.index()] as u64) < ticks)
}

/// The two pinned runs, checked for the thing each exists to cover before its value is
/// compared. Shared so that native and wasm assert the same guards, not just the same
/// numbers.
pub fn check_golden_runs() {
    let shipped_world = advanced(42, shipped(), FOUNDERS, SHIPPED_TICKS);
    assert!(
        shipped_world.population() > 0,
        "pinned a world with nothing left in it"
    );
    assert_eq!(shipped_world.state_hash(), SHIPPED_GOLDEN);

    let breeding_world = advanced(42, breeding(), FOUNDERS, BREEDING_TICKS);
    assert!(
        saw_a_birth(&breeding_world, BREEDING_TICKS),
        "nothing was born, so this pins a run that never reached resolve_births"
    );
    assert_eq!(breeding_world.state_hash(), BREEDING_GOLDEN);
}
