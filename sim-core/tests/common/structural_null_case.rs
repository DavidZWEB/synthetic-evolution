//! Shared native/WASM structural-null continuation.
//!
//! A breeding population with structural edits enabled, so donor draws, topology
//! transfer, scalar redraw, and slot reuse all reach the hash. It pins mechanics, not
//! evidence about whether inherited structure is useful.

use sim_core::control::BrainInheritance;
use sim_core::{SimParams, World};

// New scenario for the structural_null_v1 heredity mode (spec section 7.8).
pub const NULL_GOLDEN: u64 = 0xb41e_ed86_1fb8_72c9;
// The same scenario under structural_null_v2 (spec section 7.8).
pub const NULL_V2_GOLDEN: u64 = 0x48be_8a21_8ab6_ed9f;

const FOUNDERS: u32 = 32;
const TICKS: u64 = 500;

fn run(mode: BrainInheritance) -> World {
    let mut params = SimParams::default().without_structural_mutation();
    params.world.max_agents = 512;
    params.body.size = 1.5;
    params.metabolism.k_sensor = 0.003_125;
    params.feeding.reach = 8.0;
    params.reproduction.maturity_ticks = 100;
    params.reproduction.threshold = 120.0;
    let rates = &mut params.mutation.structural;
    rates.add_connection_rate = 0.05;
    rates.add_neuron_rate = 0.02;
    rates.toggle_connection_rate = 0.02;
    let mut world = World::new_with_brain_inheritance(42, params, mode).unwrap();
    assert_eq!(world.seed_founders(FOUNDERS), FOUNDERS);
    for _ in 0..TICKS {
        world.step();
    }
    world
}

pub fn check_structural_null_run() {
    let scalar = run(BrainInheritance::RandomizedAtBirth).state_hash();
    let evolving = run(BrainInheritance::Evolving).state_hash();
    let mut hashes = Vec::new();
    for mode in [
        BrainInheritance::StructuralNull,
        BrainInheritance::StructuralNullV2,
    ] {
        let world = run(mode);
        assert!(
            world
                .pool()
                .iter_live()
                .any(|id| u64::from(world.agents().age[id.index()]) < TICKS),
            "{mode:?}: nothing was born, so donor transfer never ran"
        );
        assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
        // Same seed and params: neither null may collapse into a control or the
        // evolving world it is compared with.
        assert_ne!(world.state_hash(), scalar, "{mode:?}");
        assert_ne!(world.state_hash(), evolving, "{mode:?}");
        hashes.push(world.state_hash());
    }
    assert_eq!(
        hashes,
        [NULL_GOLDEN, NULL_V2_GOLDEN],
        "structural null v1={:016x}, v2={:016x}",
        hashes[0],
        hashes[1]
    );
}
