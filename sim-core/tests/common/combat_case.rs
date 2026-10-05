//! Shared native/WASM bite scenario (spec §4.2).
//!
//! Founders whose bites are awake from the start, crowded within reach, in every
//! heredity mode: swings pay and cool down, hits wound and take mouthfuls, health
//! regenerates, kills leave corpses, and fed biters breed. A checkpoint taken mid-fight
//! continues to the uninterrupted run's state. A mechanism case, not evidence of
//! predation; it ends before the fight empties the world.

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
    0xf905_fbd9_4f31_5f0c,
    0xb4e8_b968_afa5_2807,
    0x1d23_99cc_37d9_af9b,
];
const TICKS: u32 = 135;
/// Mid-fight: cooldowns running, wounds healing, corpses on the ground.
const SAVE_TICK: u32 = 75;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

fn run(mode: BrainInheritance) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    p.world.founder_spread = 0.03;
    p.plants.max_plants = 64;
    // A founder's bite is a fifth effector.
    p.storage.max_effectors = 5;
    p.founder.bite = true;
    p.combat.dormant_bias = 3.0;
    p.combat.reach = 8.0;
    p.combat.attack_damage = 0.5;
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
        if tick == SAVE_TICK {
            assert!(
                live.iter().any(|&i| agents.cooldown[i] > 0)
                    && live.iter().any(|&i| agents.health[i] < 1.0),
                "{mode:?}: the checkpoint holds no fight in progress"
            );
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    assert!(
        wounded && killed && born,
        "{mode:?}: no bite wounded and killed, or nothing was born"
    );
    let hash = world.state_hash();
    assert_eq!(
        resumed.unwrap().state_hash(),
        hash,
        "{mode:?}: restored mid-fight but ended elsewhere"
    );
    hash
}

/// Each mode's run, checked for the fight it exists to pin before its hash is compared.
pub fn check_combat_runs() {
    let hashes = MODES.map(run);
    assert_eq!(hashes, GOLDEN);
}
