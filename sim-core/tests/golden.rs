//! The golden hash: the highest-value test in the project (spec §7.8).
//!
//! Every other test here asserts something someone thought of. This one asserts that
//! nothing changed at all, which is how it catches the changes nobody predicted — a
//! summation reordered, a `HashMap` introduced, a platform `sin` slipping past the lint,
//! a default nudged in passing.
//!
//! **A failure here is not automatically a bug.** Behaviour or hash coverage moved.
//! An intended change updates the constant in the same commit and explains its cause.
//! Keep coverage-only refreshes separate from dynamics changes so reviewers can tell
//! which happened. An unexplained update is the red flag.
//!
//! The runs use `SimParams::default()` on purpose. That makes the test sensitive to every
//! shipped default, which is the point: a default is behaviour, and changing one should
//! take the same deliberate step as changing code.
//!
//! The scenario and its pinned values live in `common/golden_case.rs`, because
//! `shells/wasm` asserts the same ones — see `wasm_agrees_with_native` there.

#[allow(dead_code)]
#[path = "common/golden_case.rs"]
mod common;
use common::*;

#[path = "common/storage_case.rs"]
mod storage_case;

#[path = "common/structural_case.rs"]
mod structural_case;

#[path = "common/organs_case.rs"]
mod organs_case;

#[path = "common/species_world_case.rs"]
mod species_world_case;

#[test]
fn classified_world_matches_its_reference_in_both_modes() {
    species_world_case::check_classified_world_runs();
}

#[test]
fn organ_mutation_matches_its_reference_in_both_modes() {
    organs_case::check_organ_runs();
}

#[test]
fn structural_mutation_matches_its_reference_in_both_modes() {
    structural_case::check_structural_runs();
}

#[test]
fn variable_storage_matches_its_reference_in_both_heredity_modes() {
    storage_case::check_variable_storage_runs();
}

#[test]
fn the_pinned_runs_still_hash_to_their_golden_values() {
    check_golden_runs();
}

#[test]
fn two_runs_in_one_process_agree() {
    // Spec §7.8's stated criterion for M8. Distinct from the pinned values: those catch
    // behaviour moving between commits, this catches a world whose result depends on
    // anything left over from a previous one — a `static`, an allocator address, a clock.
    assert_eq!(run(7, shipped(), 64, 400), run(7, shipped(), 64, 400));
    assert_eq!(run(7, breeding(), 64, 400), run(7, breeding(), 64, 400));
}

#[test]
fn interleaved_worlds_do_not_contaminate_each_other() {
    // Three worlds stepped in lockstep rather than one after another. A `static` anywhere
    // in `sim-core` passes the test above and fails this one (spec §7.2).
    let alone = run(11, shipped(), 32, 200);
    let mut worlds: Vec<_> = (0..3).map(|_| seeded(11, shipped(), 32)).collect();
    for _ in 0..200 {
        for w in &mut worlds {
            w.step();
        }
    }
    for w in &worlds {
        assert_eq!(w.state_hash(), alone);
    }
}

#[test]
fn a_single_changed_tick_changes_the_hash() {
    // Guards the guard: if `state_hash` folded too little, every assertion above could
    // pass while the run underneath had changed completely.
    assert_ne!(
        run(42, shipped(), FOUNDERS, SHIPPED_TICKS),
        run(42, shipped(), FOUNDERS, SHIPPED_TICKS - 1)
    );
}
