//! Shared native/WASM cryptic-bite scenario (spec §4.2).
//!
//! Crowded founders whose dormant bites read every source at weight 0, in every heredity
//! mode, with weight mutation strong enough that descendants wake their bites within the
//! run. Founders stay silent, descendants swing, and a checkpoint taken after biting has
//! begun continues to the uninterrupted run's state. A mechanism case, not evidence of
//! predation.

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
    0xe0e5_ec62_b488_b3bd,
    0x763f_3240_d599_a478,
    0x2a1e_a166_a00e_eebf,
];
const TICKS: u32 = 240;
/// The checkpoint is taken at the first tick from here on after a swing.
const SAVE_FROM: u32 = 60;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

fn run(mode: BrainInheritance, cryptic: bool) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    // A small world, so the crowd stands among plants, eats, and breeds: descendants
    // are where a mutated link can wake a bite.
    p.world.size = 200.0;
    p.world.founder_spread = 0.08;
    p.plants.max_plants = 256;
    p.feeding.gate = 0.0;
    // A founder's bite is a fifth effector.
    p.storage.max_effectors = 5;
    p.founder.bite = true;
    p.founder.bite_cryptic = cryptic;
    p.combat.reach = 8.0;
    p.combat.attack_damage = 0.5;
    // Strong enough that some newborn's reset lands on a cryptic link early.
    p.mutation.weight_reset_rate = 0.3;
    p.mutation.weight_perturb_rate = 0.5;
    // The shared scenario keeps no corpses; kills here leave them.
    p.corpses = sim_core::params::CorpseParams::default();
    p.corpses.max_corpses = 64;
    // Cheap births, so descendants arrive within the run.
    p.reproduction.start_energy = 40.0;
    p.reproduction.threshold = 60.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(23, p, mode).unwrap();
    world.seed_founders(48);
    world.step();
    assert_eq!(world.bite_counts().swings, 0, "{mode:?}: a founder swung");
    let mut resumed: Option<World> = None;
    for tick in 2..=TICKS {
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        if resumed.is_none() && tick >= SAVE_FROM && world.bite_counts().swings > 0 {
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    let hash = world.state_hash();
    if cryptic {
        assert!(world.bite_counts().swings > 0, "{mode:?}: no bite woke");
        assert_eq!(
            resumed
                .expect("a checkpoint was taken after biting began")
                .state_hash(),
            hash,
            "{mode:?}: restored but ended elsewhere"
        );
    }
    hash
}

/// Each mode's run, checked for the waking it exists to pin and against the same world
/// with an unwired dormant bite, before its hash is compared.
pub fn check_cryptic_runs() {
    let hashes = MODES.map(|mode| run(mode, true));
    for (mode, &hash) in MODES.into_iter().zip(&hashes) {
        assert_ne!(
            hash,
            run(mode, false),
            "{mode:?}: the cryptic wiring changed nothing, so the case cannot pin it"
        );
    }
    assert_eq!(hashes, GOLDEN);
}
