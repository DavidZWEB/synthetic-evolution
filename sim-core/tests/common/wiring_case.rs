//! Shared native/WASM wired-sensor and knockout scenario (spec §3.3, §4.1).
//!
//! Founders that see and smell, at dimmed gains, breed cheaply while births add wired
//! sensors, in every heredity mode. A live retune then blinds every eye, and a
//! checkpoint taken after it continues to the uninterrupted run's state. A mechanism
//! case, not evidence that wiring protects innovation.

use sim_core::World;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::control::BrainInheritance;
use sim_core::genome::Gene;

pub const MODES: [BrainInheritance; 3] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
    BrainInheritance::StructuralNull,
];
/// `state_hash` after `TICKS`, per mode.
pub const GOLDEN: [u64; 3] = [
    0xe40b_c961_731e_20ae,
    0x0fcf_dab6_20b6_36b4,
    0xb9e2_f1a2_572a_c7f6,
];
const TICKS: u32 = 120;
/// Every eye goes dark here, on a running world.
const BLIND_TICK: u32 = 40;
/// After the knockout, so the checkpoint carries the retuned gains.
const SAVE_TICK: u32 = 60;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

/// Whether `genes` carries a sensor that arrived wired: its addition gives the wire the
/// next innovation after the sensor's own. Exact here, where no neural operator adds
/// connections.
fn wired(genes: &[Gene]) -> bool {
    genes.iter().any(|gene| {
        let Gene::Sensor(sensor) = gene else {
            return false;
        };
        genes.iter().any(|other| {
            matches!(other, Gene::Connection(wire)
                if wire.id.raw() == sensor.id.raw() + 1 && sensor.targets.contains(&wire.from))
        })
    })
}

fn run(mode: BrainInheritance) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    // Crowded among plenty of plants, so founders eat and breed within the run.
    p.world.founder_spread = 0.05;
    p.plants.max_plants = 256;
    p.sensing.vision_rays = 1;
    p.sensing.vision_gain = 0.5;
    p.sensing.chemo_gain = 0.75;
    p.mutation.organs.add_sensor_rate = 0.5;
    p.mutation.organs.wired_weight_scale = 0.25;
    // Cheap births, so organs are added and the heredity modes part ways in the run.
    p.reproduction.start_energy = 40.0;
    p.reproduction.threshold = 45.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(29, p.clone(), mode).unwrap();
    world.seed_founders(32);
    let mut resumed: Option<World> = None;
    for tick in 1..=TICKS {
        if tick == BLIND_TICK {
            let mut blind = p.clone();
            blind.sensing.vision_gain = 0.0;
            world.set_params(blind).unwrap();
        }
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        if tick == SAVE_TICK {
            assert!(
                world.pool().iter_live().any(|id| wired(world.genome(id))),
                "{mode:?}: the checkpoint holds no wired sensor"
            );
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    let hash = world.state_hash();
    assert_eq!(
        resumed.unwrap().state_hash(),
        hash,
        "{mode:?}: restored after the knockout but ended elsewhere"
    );
    hash
}

/// Each mode's run, checked for the wired organs it exists to pin before its hash is
/// compared.
pub fn check_wiring_runs() {
    let hashes = MODES.map(run);
    assert_eq!(hashes, GOLDEN);
}
