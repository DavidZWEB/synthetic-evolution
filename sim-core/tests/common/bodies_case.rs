//! Shared native/WASM body-trait scenario (spec §3.5).
//!
//! Births step size, muscle, mouth, gut, and colour in every heredity mode, and the tick
//! moves, feeds, and charges the bodies that result. The step is large so a short run
//! reaches both ends of the ranges: a mechanism case, not evidence of adaptation.

use sim_core::World;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::control::BrainInheritance;

/// Halfway, with mutated guts, muscles, and mouths alive: a restore must read each from
/// its genome and continue to the uninterrupted run's state.
const SAVE_TICK: u32 = 150;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

pub const MODES: [BrainInheritance; 3] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
    BrainInheritance::StructuralNull,
];
/// `state_hash` after `TICKS`, per mode.
pub const GOLDEN: [u64; 3] = [
    0x6735_7a84_2c9a_0a03,
    0x08e5_bd5f_26ae_4c17,
    0x4809_873f_2ad0_77a3,
];
const TICKS: u32 = 300;

fn run(mode: BrainInheritance) -> u64 {
    let mut p = crate::scenario::params();
    p.world.size = 100.0;
    p.world.max_agents = 48;
    p.world.founder_spread = 0.2;
    p.plants.max_plants = 64;
    p.plants.energy_input_rate = 400.0;
    p.plants.max_energy = 100.0;
    p.chemo.cells = [8, 8, 1];
    p.sensing.vision_range = 20.0;
    p.sensing.chemo_radius = 20.0;
    p.metabolism.base = 0.3;
    p.reproduction.start_energy = 20.0;
    p.reproduction.threshold = 30.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    p.feeding.rate = 20.0;
    p.feeding.gate = 0.0;
    p.feeding.reach = 20.0;
    p.mutation.body_trait_rate = 1.0;
    p.mutation.body_trait_sigma = 0.5;
    p.mutation.gut_sigma = 0.5;
    let body = p.body.clone();
    // Seed 10 since founders carry the gut gene: its draws moved every later step, and
    // under seed 5 no evolving mouth reached its floor within the run.
    let mut world = World::new_with_brain_inheritance(10, p, mode).unwrap();
    world.seed_founders(12);
    // Traits change only at birth, so reaching both ends of every range takes many
    // mutated births.
    let mut extremes = [false; 8];
    let mut resumed: Option<World> = None;
    for tick in 1..=TICKS {
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        if tick == SAVE_TICK {
            assert!(
                world
                    .pool()
                    .iter_live()
                    .any(|id| world.agents().gut[id.index()] < 1.0),
                "{mode:?}: the checkpoint holds no shortened gut"
            );
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
        let agents = world.agents();
        for id in world.pool().iter_live() {
            let i = id.index();
            for (k, (value, [low, high])) in [
                (agents.size[i], body.size_range),
                (agents.muscle[i], body.muscle_range),
                (agents.mouth[i], body.mouth_range),
                (agents.gut[i], [0.0, 1.0]),
            ]
            .into_iter()
            .enumerate()
            {
                assert!(
                    (low..=high).contains(&value),
                    "{mode:?}: trait {k} at {value}"
                );
                extremes[2 * k] |= value == low;
                extremes[2 * k + 1] |= value == high;
            }
        }
    }
    assert_eq!(extremes, [true; 8], "{mode:?}: a range was never reached");
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    let hash = world.state_hash();
    assert_eq!(
        resumed.unwrap().state_hash(),
        hash,
        "{mode:?}: restored with mutated bodies but ended elsewhere"
    );
    hash
}

pub fn check_body_runs() {
    for (mode, golden) in MODES.into_iter().zip(GOLDEN) {
        assert_eq!(run(mode), golden, "{mode:?}");
    }
}
