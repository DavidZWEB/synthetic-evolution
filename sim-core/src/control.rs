//! Experiment modes that change heredity without changing the world's parameters.
//!
//! The random-brain control is a separate world with the same seed and [`SimParams`].
//! Its offspring receive freshly randomized neural scalars instead of inheriting a
//! mutated parent brain. Both modes use the same structural rules, so this control
//! isolates scalar inheritance, not all evolution (spec §7.8, §10).

use crate::founder::FounderPlan;
use crate::genome::Gene;
use crate::mutate::{self, MutationState, structural};
use crate::params::SimParams;
use structural::StructuralMutationEvent;

/// Telemetry distinguishes topology-aware controls from Phase 1's fixed-topology run.
pub const RANDOMIZED_AT_BIRTH_PROTOCOL: &str = "randomized_at_birth_v2";

/// How neural scalars are assigned to offspring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BrainInheritance {
    /// Inherit the parent's genome and apply the configured mutation operators.
    #[default]
    Evolving = 0,
    /// Apply structural edits, then redraw weights, biases, taus, and oscillator periods.
    RandomizedAtBirth = 1,
}

impl BrainInheritance {
    pub(crate) fn prepare_offspring(
        self,
        plan: &FounderPlan,
        genes: &mut Vec<Gene>,
        params: &SimParams,
        state: &mut MutationState<'_>,
        on_event: impl FnMut(StructuralMutationEvent),
    ) {
        if self == Self::Evolving {
            mutate::mutate(genes, state.rng, &params.mutation);
        }
        structural::apply(genes, params, state, on_event);
        if self == Self::RandomizedAtBirth {
            plan.randomize_brain(state.rng, params, genes, state.neuron_scratch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InnovationId, Rng};

    #[test]
    fn disabled_structure_preserves_each_legacy_scalar_path_and_rng() {
        let params = SimParams::default();
        let mut next = 0;
        let plan = FounderPlan::new(&params, || {
            let id = InnovationId::new(next);
            next += 1;
            id
        })
        .unwrap();
        let mut initial = vec![Gene::default(); plan.len()];
        plan.instantiate(&mut Rng::from_seed(1), &params, &mut initial);
        for mode in [
            BrainInheritance::Evolving,
            BrainInheritance::RandomizedAtBirth,
        ] {
            let mut expected = initial.clone();
            let mut actual = Vec::with_capacity(params.storage.max_genes as usize);
            actual.extend_from_slice(&initial);
            let mut a = Rng::from_seed(17);
            let mut b = Rng::from_seed(17);
            let mut expected_scratch = vec![0; params.storage.max_neurons as usize];
            match mode {
                BrainInheritance::Evolving => {
                    mutate::mutate(&mut expected, &mut a, &params.mutation)
                }
                BrainInheritance::RandomizedAtBirth => {
                    plan.randomize_brain(&mut a, &params, &mut expected, &mut expected_scratch)
                }
            }
            let mut scratch = vec![0; params.storage.max_neurons as usize];
            let mut counter = next;
            let mut events = 0;
            mode.prepare_offspring(
                &plan,
                &mut actual,
                &params,
                &mut MutationState {
                    rng: &mut b,
                    next_innovation: &mut counter,
                    neuron_scratch: &mut scratch,
                },
                |_| events += 1,
            );
            assert_eq!(actual, expected);
            assert_eq!(a.state_fingerprint(), b.state_fingerprint());
            assert_eq!(counter, next);
            assert_eq!(events, 0);
        }
    }
}
