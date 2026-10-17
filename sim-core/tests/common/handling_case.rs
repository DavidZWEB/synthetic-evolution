//! Shared native/WASM handling-time scenario (spec §4.2).
//!
//! The bite scenario with handling time on, among plants: every hit keeps its biter
//! from grazing until the timer runs out, though it may still scavenge. A checkpoint
//! taken while a mouth is busy continues to the uninterrupted run's state. A mechanism
//! case, not evidence of predation.

use sim_core::World;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::control::BrainInheritance;

pub const MODES: [BrainInheritance; 3] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
    BrainInheritance::StructuralNull,
];
/// `state_hash` after `TICKS`, per mode.
pub const GOLDEN: [u64; 3] = [
    0x0c4b_465d_da48_d8c5,
    0x2ce5_d3ff_0a81_8080,
    0x943b_9966_a26d_0264,
];
const TICKS: u32 = 135;
/// The checkpoint is taken at the first tick from here on with a mouth still busy.
const SAVE_FROM: u32 = 30;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

fn run(mode: BrainInheritance, handling_seconds: f32) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    // A small world, so the crowd stands among plants and every agent tries to eat:
    // handling then decides who grazes.
    p.world.size = 200.0;
    p.world.founder_spread = 0.08;
    p.plants.max_plants = 256;
    p.feeding.gate = 0.0;
    // A founder's bite is a fifth effector.
    p.storage.max_effectors = 5;
    p.founder.bite = true;
    p.combat.dormant_bias = 3.0;
    p.combat.reach = 8.0;
    p.combat.attack_damage = 0.5;
    p.combat.handling_seconds = handling_seconds;
    // The shared scenario keeps no corpses; kills here leave them.
    p.corpses = sim_core::params::CorpseParams::default();
    p.corpses.max_corpses = 64;
    // Cheap births, so the heredity modes part ways within the run.
    p.reproduction.start_energy = 40.0;
    p.reproduction.threshold = 60.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(23, p, mode).unwrap();
    world.seed_founders(48);
    let (mut wounded, mut killed, mut born) = (false, false, false);
    let mut handled = false;
    let mut resumed: Option<World> = None;
    for tick in 1..=TICKS {
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        let agents = world.agents();
        let live: Vec<usize> = world.pool().iter_live().map(|id| id.index()).collect();
        wounded |= live.iter().any(|&i| agents.health[i] < 1.0);
        born |= live.iter().any(|&i| agents.age[i] < tick);
        // The starved die empty, so a corpse is a bite's kill.
        killed |= world.corpses().count() > 0;
        handled |= live.iter().any(|&i| agents.handling[i] > 0);
        // Without handling there is no busy mouth to wait for; save at the first chance.
        let busy = live.iter().any(|&i| agents.handling[i] > 0) || handling_seconds == 0.0;
        if resumed.is_none() && tick >= SAVE_FROM && busy {
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    assert!(
        wounded && killed && born && (handled || handling_seconds == 0.0),
        "{mode:?}: no bite wounded, killed, and was handled, or nothing was born"
    );
    let hash = world.state_hash();
    assert_eq!(
        resumed
            .expect("a checkpoint was taken mid-meal")
            .state_hash(),
        hash,
        "{mode:?}: restored mid-meal but ended elsewhere"
    );
    hash
}

/// Five ticks at 60 per second: long enough to span a checkpoint, short enough to run
/// out several times in the run.
const HANDLING_SECONDS: f32 = 5.0 / 60.0;

/// Each mode's run, checked for the meals it exists to pin, and against the same run
/// without handling, before its hash is compared.
pub fn check_handling_runs() {
    let hashes = MODES.map(|mode| run(mode, HANDLING_SECONDS));
    for (mode, &hash) in MODES.into_iter().zip(&hashes) {
        assert_ne!(
            hash,
            run(mode, 0.0),
            "{mode:?}: handling changed nothing, so the case cannot pin it"
        );
    }
    assert_eq!(hashes, GOLDEN);
}
