//! Shared native/WASM structural-null continuation.
//!
//! A breeding population with structural edits enabled, so donor draws, topology
//! transfer, parent-scalar carry-over, and slot reuse all reach the hash. It pins mechanics, not
//! evidence about whether inherited structure is useful.

use sim_core::World;
use sim_core::control::BrainInheritance;

// The structural null heredity mode (spec section 7.8).
pub const NULL_GOLDEN: u64 = 0x15ff_60e7_59ac_76bf;

const FOUNDERS: u32 = 32;
const TICKS: u64 = 500;

fn run(mode: BrainInheritance) -> World {
    let mut params = crate::scenario::params();
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
    {
        let mode = BrainInheritance::StructuralNull;
        let world = run(mode);
        assert!(
            world
                .pool()
                .iter_live()
                .any(|id| u64::from(world.agents().age[id.index()]) < TICKS),
            "{mode:?}: nothing was born, so donor transfer never ran"
        );
        assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
        // Same seed and params: the null may not collapse into the scalar control or
        // the evolving world it is compared with.
        assert_ne!(world.state_hash(), scalar, "{mode:?}");
        assert_ne!(world.state_hash(), evolving, "{mode:?}");
        assert_eq!(
            world.state_hash(),
            NULL_GOLDEN,
            "structural null={:016x}",
            world.state_hash()
        );
    }
}
