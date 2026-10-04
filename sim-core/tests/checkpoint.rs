//! Checkpoint continuation: a restored world must be the world it was saved from.
//!
//! The scenario churns through structural births, deaths, slot reuse, fragmented
//! arenas, species turnover, recurrent state, energy reserves, and future commands,
//! then collapses, because a matching hash right after loading proves none of those
//! (spec §7.8). The scenario and its cross-target references live in
//! `common/checkpoint_case.rs`.

use proptest::prelude::*;
use sim_core::World;
use sim_core::checkpoint::{CHECKPOINT_FORMAT, CheckpointError, CheckpointLimits};
use sim_core::control::BrainInheritance;
use sim_core::species::SpeciesEventCounts;

#[path = "common/scenario.rs"]
mod scenario;

#[path = "common/checkpoint_case.rs"]
mod case;
use case::{CONTINUE_TICKS, LIMITS, MODES, churned, params, step};

/// The coverage this test claims; if tuning moves the scenario, it fails loudly.
fn assert_rich(world: &World) {
    let live: Vec<_> = world.pool().iter_live().collect();
    assert_eq!(live.len(), 32, "full pool");
    assert!(
        world.pool().incarnations().iter().any(|&n| n > 1),
        "slot reuse"
    );
    assert!(
        live.iter()
            .any(|id| world.agents().energy_reserve[id.index()] != 0.0),
        "compensated energy"
    );
    assert!(
        live.iter()
            .any(|&id| world.brain(id).iter().any(|n| n.state != 0.0)),
        "recurrent state"
    );
    assert!(
        world
            .storage_usage()
            .iter()
            .any(|usage| usage.largest_free_block < usage.free_elements),
        "fragmented arenas"
    );
    assert_eq!(world.pending_commands(), 2, "future commands");
}

#[test]
fn restored_worlds_continue_exactly_in_every_heredity_mode() {
    for mode in MODES {
        let mut original = churned(mode);
        assert_rich(&original);
        let bytes = original.checkpoint();
        let mut restored = World::from_checkpoint(&bytes, LIMITS).unwrap();
        assert_eq!(restored.state_hash(), original.state_hash());
        assert_eq!(
            restored.checkpoint(),
            bytes,
            "re-encoding is byte-identical"
        );
        assert_eq!(restored.seed(), original.seed());

        let (mut a, mut b) = (SpeciesEventCounts::default(), SpeciesEventCounts::default());
        let incarnations = original.pool().incarnations().to_vec();
        for tick in 0..CONTINUE_TICKS {
            step(&mut original, &mut a);
            step(&mut restored, &mut b);
            assert_eq!(
                restored.state_hash(),
                original.state_hash(),
                "{mode:?} diverged {tick} ticks after restore"
            );
        }
        assert_eq!(a, b);
        assert_eq!(original.pending_commands(), 0, "queued commands ran");
        assert!(a.extinct > 0, "continuation saw extinctions");
        assert!(
            original.pool().incarnations() != incarnations.as_slice(),
            "continuation freed and reused slots"
        );
    }
}

#[test]
fn saving_neither_advances_time_nor_draws_randomness() {
    let mut saved = churned(BrainInheritance::Evolving);
    let mut twin = churned(BrainInheritance::Evolving);
    let before = saved.state_hash();
    let _ = saved.checkpoint();
    assert_eq!(saved.state_hash(), before);
    for _ in 0..50 {
        saved.step();
        twin.step();
    }
    assert_eq!(saved.state_hash(), twin.state_hash());
}

#[test]
fn retuned_worlds_keep_their_original_grid() {
    let mut world = churned(BrainInheritance::Evolving);
    let cells = world.spatial_hash().cells_per_axis();
    let mut narrower = world.params().clone();
    narrower.sensing.vision_range = 5.0;
    narrower.sensing.chemo_radius = 5.0;
    world.set_params(narrower).unwrap();
    let mut restored = World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap();
    assert_eq!(restored.spatial_hash().cells_per_axis(), cells);
    for _ in 0..100 {
        world.step();
        restored.step();
    }
    assert_eq!(restored.state_hash(), world.state_hash());
}

#[test]
fn empty_and_fresh_worlds_round_trip() {
    for founders in [0, 3] {
        let mut world = World::new(5, params()).unwrap();
        world.seed_founders(founders);
        let restored = World::from_checkpoint(&world.checkpoint(), LIMITS).unwrap();
        assert_eq!(restored.state_hash(), world.state_hash());
    }
}

#[test]
fn malformed_checkpoints_are_refused_explicitly() {
    let bytes = churned(BrainInheritance::Evolving).checkpoint();
    assert!(matches!(
        World::from_checkpoint(
            &bytes,
            CheckpointLimits {
                max_bytes: bytes.len() - 1,
                ..LIMITS
            }
        ),
        Err(CheckpointError::TooLarge)
    ));
    let mut magic = bytes.clone();
    magic[0] ^= 1;
    assert!(matches!(
        World::from_checkpoint(&magic, LIMITS),
        Err(CheckpointError::NotACheckpoint)
    ));
    let mut format = bytes.clone();
    format[8..12].copy_from_slice(&(CHECKPOINT_FORMAT + 1).to_le_bytes());
    assert!(matches!(
        World::from_checkpoint(&format, LIMITS),
        Err(CheckpointError::UnsupportedFormat(_))
    ));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(World::from_checkpoint(&trailing, LIMITS).is_err());
    for len in (0..bytes.len()).step_by(bytes.len() / 97 + 1) {
        assert!(
            World::from_checkpoint(&bytes[..len], LIMITS).is_err(),
            "accepted a {len}-byte truncation"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Untrusted bytes are refused or restored; they never panic (spec §7.10).
    #[test]
    fn corrupted_checkpoints_never_panic(
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
    ) {
        let mut bytes = World::new(5, params())
            .map(|mut world| {
                world.seed_founders(4);
                world.step();
                world.checkpoint()
            })
            .unwrap();
        for (index, mask) in flips {
            let i = index.index(bytes.len());
            bytes[i] ^= mask.max(1);
        }
        if let Ok(world) = World::from_checkpoint(&bytes, LIMITS) {
            // Anything accepted is self-consistent: its own hash re-verifies.
            prop_assert!(World::from_checkpoint(&world.checkpoint(), LIMITS).is_ok());
        }
    }
}

#[test]
fn checkpoints_match_the_cross_target_references() {
    case::check_cross_target_checkpoints();
}
