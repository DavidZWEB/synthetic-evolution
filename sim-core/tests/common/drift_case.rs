//! Shared native/WASM eye-drift scenario (spec §4.1).
//!
//! Sighted founders among plants breed in every heredity mode while each birth may turn
//! every eye its genome carries. Descendants look somewhere their founders did not, and
//! a checkpoint taken after eyes have turned continues to the uninterrupted run's state.
//! A mechanism case, not evidence of adaptation.

use sim_core::World;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::control::BrainInheritance;
use sim_core::genome::{Gene, Modality};

pub const MODES: [BrainInheritance; 3] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
    BrainInheritance::StructuralNull,
];
/// `state_hash` after `TICKS`, per mode.
pub const GOLDEN: [u64; 3] = [
    0x2f29_1295_be39_a161,
    0xfb96_7d1a_b014_f213,
    0x2026_dd3e_bc6b_1920,
];
const TICKS: u32 = 200;
const SAVE_TICK: u32 = 100;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

/// Every living agent's eye directions, in slot order.
fn eyes(world: &World) -> Vec<f32> {
    world
        .pool()
        .iter_live()
        .flat_map(|id| {
            world.genome(id).iter().filter_map(|gene| match gene {
                Gene::Sensor(s) if s.modality == Modality::VisionRay => Some(s.params[0]),
                _ => None,
            })
        })
        .collect()
}

fn run(mode: BrainInheritance, rate: f32) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    p.world.size = 200.0;
    p.world.founder_spread = 0.08;
    p.plants.max_plants = 256;
    p.feeding.gate = 0.0;
    p.sensing.vision_rays = 1;
    p.mutation.organs.eye_drift_rate = rate;
    p.mutation.organs.eye_drift_sigma = 0.3;
    p.reproduction.start_energy = 40.0;
    p.reproduction.threshold = 60.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(23, p, mode).unwrap();
    world.seed_founders(48);
    let founding = eyes(&world);
    let mut resumed: Option<World> = None;
    for tick in 1..=TICKS {
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        if tick == SAVE_TICK {
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    let hash = world.state_hash();
    if rate > 0.0 {
        assert!(
            eyes(&world)
                .iter()
                .any(|azimuth| !founding.contains(azimuth)),
            "{mode:?}: no eye turned"
        );
        assert_eq!(
            resumed.unwrap().state_hash(),
            hash,
            "{mode:?}: restored but ended elsewhere"
        );
    }
    hash
}

/// Each mode's run, checked for the turning it exists to pin and against the same world
/// without drift, before its hash is compared.
pub fn check_drift_runs() {
    let hashes = MODES.map(|mode| run(mode, 1.0));
    for (mode, &hash) in MODES.into_iter().zip(&hashes) {
        assert_ne!(
            hash,
            run(mode, 0.0),
            "{mode:?}: drift changed nothing, so the case cannot pin it"
        );
    }
    assert_eq!(hashes, GOLDEN);
}
