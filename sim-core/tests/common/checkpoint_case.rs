//! The checkpoint continuation scenario, shared by native tests and the wasm
//! cross-target test (spec §7.10).
//!
//! Pins both the encoded bytes and the state after restoring and continuing, so a
//! checkpoint written by either target restores identically on the other. One
//! definition on purpose: two copies decay into each target agreeing with itself.

use glam::Vec3;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::command::{Command, Kind};
use sim_core::control::BrainInheritance;
use sim_core::species::SpeciesEventCounts;
use sim_core::{SimParams, World};

pub const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};
pub const MODES: [BrainInheritance; 2] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
];
/// Mid-churn: full pool, reused slots, fragmented arenas, species turnover.
pub const SAVE_TICK: u32 = 200;
/// Long enough to reach the queued commands and the population collapse.
pub const CONTINUE_TICKS: u32 = 350;

/// FNV-1a of the checkpoint bytes, per mode: both targets must encode identically.
pub const CHECKPOINT_FINGERPRINT: [u64; 2] = [0x86f8_fa84_0be7_33ad, 0x6b01_125b_a900_7435];
/// `state_hash` after restoring and continuing `CONTINUE_TICKS`, per mode.
pub const CONTINUED_HASH: [u64; 2] = [0x737a_a4fb_0b35_3fd9, 0x56a9_8c23_6bdd_c1c3];

pub fn params() -> SimParams {
    let mut p = SimParams::default();
    p.world.size = 100.0;
    p.world.max_agents = 32;
    p.world.founder_spread = 0.2;
    p.plants.max_plants = 64;
    p.plants.energy_input_rate = 400.0;
    p.plants.max_energy = 100.0;
    p.chemo.cells = [8, 8, 1];
    p.sensing.vision_range = 20.0;
    p.sensing.vision_rays = 1;
    p.sensing.chemo_radius = 20.0;
    p.brain.hidden_neurons = 2;
    p.brain.oscillators = 1;
    p.metabolism.base = 0.3;
    p.reproduction.start_energy = 20.0;
    p.reproduction.threshold = 30.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    p.feeding.rate = 100.0;
    p.feeding.gate = 0.0;
    p.feeding.reach = 20.0;
    p.species.threshold = 0.05;
    let rates = &mut p.mutation.structural;
    rates.remove_connection_rate = 0.5;
    rates.remove_neuron_rate = 0.3;
    rates.toggle_connection_rate = 0.5;
    rates.add_connection_rate = 0.5;
    rates.add_neuron_rate = 0.3;
    p
}

pub fn step(world: &mut World, events: &mut SpeciesEventCounts) {
    world.step_with_all_observers(|_| {}, |_| {}, |event| events.record(event));
}

/// The world at `SAVE_TICK`, with two founder commands queued for its future.
pub fn churned(mode: BrainInheritance) -> World {
    let mut world = World::new_with_brain_inheritance(11, params(), mode).unwrap();
    world.seed_founders(12);
    let mut events = SpeciesEventCounts::default();
    for _ in 0..SAVE_TICK {
        step(&mut world, &mut events);
    }
    assert!(
        events.created > events.extinct && events.extinct > 0,
        "{events:?}"
    );
    let tick = world.tick_count();
    for (delta, x) in [(5, 10.0), (60, 80.0)] {
        world.push_command(Command {
            apply_at_tick: tick + delta,
            kind: Kind::SpawnFounder {
                position: Vec3::new(x, 50.0, 0.0),
            },
        });
    }
    world
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Checks this target against the pinned cross-target references.
pub fn check_cross_target_checkpoints() {
    for (index, mode) in MODES.into_iter().enumerate() {
        let bytes = churned(mode).checkpoint();
        let mut restored = World::from_checkpoint(&bytes, LIMITS).unwrap();
        for _ in 0..CONTINUE_TICKS {
            restored.step();
        }
        assert_eq!(
            (fnv1a(&bytes), restored.state_hash()),
            (CHECKPOINT_FINGERPRINT[index], CONTINUED_HASH[index]),
            "{mode:?} checkpoint references"
        );
    }
}
