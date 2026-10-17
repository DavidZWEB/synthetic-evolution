//! Shared native/WASM scenario for the carnivory research mechanisms.
//!
//! Crowded founders with a cryptically wired bite awake from the start, gut and jaw
//! below one so both power curves run, digestion waste, diet plasticity, a short
//! handling time that expires inside the run, eye drift, and body-trait mutation at
//! every birth. A checkpoint taken mid-fight continues to the uninterrupted run's
//! state. A mechanism case, not evidence of predation.

use sim_core::World;
use sim_core::checkpoint::CheckpointLimits;
use sim_core::control::BrainInheritance;

pub const MODES: [BrainInheritance; 3] = [
    BrainInheritance::Evolving,
    BrainInheritance::RandomizedAtBirth,
    BrainInheritance::StructuralNull,
];
/// `state_hash` after `TICKS`, per mode, recorded from the native run.
pub const GOLDEN: [u64; 3] = [
    0x9158_3143_5789_6fea,
    0xee66_9f9d_0861_227e,
    0xd876_3e3e_8f05_6b30,
];
const TICKS: u32 = 240;
/// The checkpoint is taken at the first tick from here on with a meal in progress.
const SAVE_FROM: u32 = 20;
const LIMITS: CheckpointLimits = CheckpointLimits {
    max_bytes: 64 << 20,
    max_core_bytes: 96 << 20,
};

fn run(mode: BrainInheritance) -> u64 {
    let mut p = crate::scenario::params();
    p.world.max_agents = 64;
    p.world.founder_spread = 0.03;
    p.plants.max_plants = 64;
    p.storage.max_effectors = 5;
    p.sensing.vision_rays = 1;
    p.sensing.vision_azimuth_spread = 1.0;
    p.founder.bite = true;
    p.founder.bite_cryptic = true;
    p.founder.gut = true;
    p.founder.jaw = true;
    p.body.gut = 0.6;
    p.body.jaw = 0.5;
    p.combat.dormant_bias = 3.0;
    p.combat.reach = 8.0;
    p.combat.attack_damage = 0.5;
    p.combat.handling_seconds = 0.5;
    p.combat.jaw_bite_gain = 1.0;
    p.combat.jaw_bite_exponent = 0.7;
    p.feeding.jaw_graze_cost = 0.5;
    p.feeding.jaw_graze_exponent = 1.3;
    p.metabolism.k_gut = 0.04;
    p.digestion.plant_floor = 0.1;
    p.digestion.plant_curve = 0.5;
    p.digestion.meat_penalty = 0.3;
    p.digestion.plasticity = 0.5;
    p.digestion.acclimation = 0.05;
    p.digestion.diet_memory = 50.0;
    p.mutation.weight_perturb_rate = 0.5;
    p.mutation.weight_perturb_sigma = 0.5;
    p.mutation.body_trait_rate = 1.0;
    p.mutation.body_trait_sigma = 0.05;
    p.mutation.gut_sigma = 0.15;
    p.mutation.jaw_sigma = 0.1;
    p.mutation.organs.eye_drift_rate = 1.0;
    p.mutation.organs.eye_drift_sigma = 0.3;
    p.corpses = sim_core::params::CorpseParams::default();
    p.corpses.max_corpses = 64;
    p.reproduction.start_energy = 40.0;
    p.reproduction.threshold = 60.0;
    p.reproduction.gate = 0.0;
    p.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(23, p, mode).unwrap();
    world.seed_founders(48);
    let (mut handled, mut released, mut acclimated, mut killed, mut born) =
        (false, false, false, false, false);
    let mut resumed: Option<World> = None;
    for tick in 1..=TICKS {
        let before: Vec<(usize, u32)> = world
            .pool()
            .iter_live()
            .map(|id| (id.index(), world.agents().handling[id.index()]))
            .collect();
        world.step();
        if let Some(resumed) = &mut resumed {
            resumed.step();
        }
        let agents = world.agents();
        let live: Vec<usize> = world.pool().iter_live().map(|id| id.index()).collect();
        handled |= live.iter().any(|&i| agents.handling[i] > 0);
        released |= before
            .iter()
            .any(|&(i, h)| h == 1 && live.contains(&i) && agents.handling[i] == 0);
        acclimated |= live
            .iter()
            .any(|&i| (agents.gut[i] - agents.gut_set[i]).abs() > 1e-4);
        born |= live.iter().any(|&i| agents.age[i] < tick);
        killed |= world.corpses().count() > 0;
        if resumed.is_none() && tick >= SAVE_FROM && live.iter().any(|&i| agents.handling[i] > 0) {
            resumed = Some(World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap());
        }
    }
    assert!(
        handled && released && acclimated && killed && born,
        "{mode:?}: handled {handled} released {released} acclimated {acclimated} \
         killed {killed} born {born}"
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

/// Each mode's hash after its run has shown the mechanisms it exists to pin.
pub fn carnivory_hashes() -> [u64; 3] {
    MODES.map(run)
}
