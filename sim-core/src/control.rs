//! Experiment modes that change heredity without changing the world's parameters.
//!
//! Each control is a separate world with the same seed and [`SimParams`]. The scalar
//! control redraws neural scalars instead of inheriting a mutated parent brain; the
//! structural nulls replace the inherited topology with a living donor's, v1 redrawing
//! scalars and v2 keeping the parent's on shared genes. All modes use the same mutation
//! rules (spec §7.8, §10). Choosing which world runs which
//! mode is the shells' concern.

use crate::founder::FounderPlan;
use crate::genome::Gene;
use crate::mutate::{self, MutationState, organs, structural};
use crate::params::SimParams;
use crate::pool::SlotPool;
use crate::{AgentId, Rng};
use structural::StructuralMutationEvent;

/// Telemetry distinguishes topology-aware controls from Phase 1's fixed-topology run.
pub const RANDOMIZED_AT_BIRTH_PROTOCOL: &str = "randomized_at_birth_v3";

/// Telemetry identity of the donor-topology, redrawn-scalar structural null (spec §7.8).
pub const STRUCTURAL_NULL_PROTOCOL: &str = "structural_null_v1";

/// Telemetry identity of the donor-topology, parent-scalar structural null (spec §7.8).
pub const STRUCTURAL_NULL_V2_PROTOCOL: &str = "structural_null_v2";

/// What offspring inherit from the parent's brain: scalars and topology, topology only, or
/// neither.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BrainInheritance {
    /// Inherit the parent's genome and apply the configured mutation operators.
    #[default]
    Evolving = 0,
    /// Apply structural edits, then redraw weights, biases, taus, and oscillator periods.
    RandomizedAtBirth = 1,
    /// Start from a living donor's topology with the parent's body, then proceed as
    /// [`Self::RandomizedAtBirth`]. Breaks parent-to-child structural inheritance.
    StructuralNull = 2,
    /// Start from a living donor's topology with the parent's body and the parent's
    /// neural scalars on every gene the two share, then mutate as [`Self::Evolving`].
    /// Decouples a lineage's structure from its weights while keeping brains working.
    StructuralNullV2 = 3,
}

impl BrainInheritance {
    /// Whether children start from a donor's topology rather than the parent's.
    pub fn takes_donor_topology(self) -> bool {
        matches!(self, Self::StructuralNull | Self::StructuralNullV2)
    }
}

impl BrainInheritance {
    pub(crate) fn prepare_offspring(
        self,
        plan: &FounderPlan,
        genes: &mut Vec<Gene>,
        params: &SimParams,
        state: &mut MutationState<'_>,
        mut on_event: impl FnMut(StructuralMutationEvent),
    ) {
        let redraws = matches!(self, Self::RandomizedAtBirth | Self::StructuralNull);
        if !redraws {
            mutate::mutate(genes, state.rng, &params.mutation);
        }
        organs::apply(genes, params, state, &mut on_event);
        structural::apply(genes, params, state, on_event);
        if redraws {
            plan.randomize_brain(state.rng, params, genes, state.neuron_scratch);
        }
    }
}

/// The structural null's topology donor: a uniformly drawn living agent other than
/// the parent, in slot order so the draw is deterministic. Agents born earlier in the
/// same tick are eligible. With no other living agent the parent is its own donor, and
/// no random number is drawn.
pub(crate) fn pick_donor(pool: &SlotPool, parent: AgentId, rng: &mut Rng) -> AgentId {
    let candidates = pool.live_count() - u32::from(pool.is_alive(parent));
    if candidates == 0 {
        return parent;
    }
    let chosen = rng.below(candidates) as usize;
    pool.iter_live()
        .filter(|&agent| agent != parent)
        .nth(chosen)
        .unwrap_or(parent)
}

/// A structural-null child's starting genome: the donor's neurons, sensors, effectors,
/// and connections with the parent's body and meta genes. Every genome carries one gene
/// per body and meta trait, so the result has the donor's length and stays sorted.
pub(crate) fn donor_topology(parent: &[Gene], donor: &[Gene], out: &mut Vec<Gene>) {
    let is_body = |gene: &&Gene| matches!(gene, Gene::Body(_) | Gene::Meta(_));
    out.clear();
    out.extend(donor.iter().filter(|gene| !is_body(gene)));
    out.extend(parent.iter().filter(is_body));
    debug_assert_eq!(out.len(), donor.len());
}

/// Gives a donor-topology genome the parent's neural scalars (weights, biases, taus,
/// oscillator periods) wherever the parent has the same gene. A gene counts as shared
/// only when its innovation ID and structural fields agree, so the donor's structure,
/// connection enable states, and organ parameters are never changed. Both genomes are
/// sorted by [`Gene::sort_key`], so lookup is a binary search.
pub(crate) fn parent_scalars(parent: &[Gene], genes: &mut [Gene]) {
    for gene in genes {
        let Ok(index) = parent.binary_search_by_key(&gene.sort_key(), Gene::sort_key) else {
            continue;
        };
        match (gene, &parent[index]) {
            (Gene::Neuron(child), Gene::Neuron(from)) if child.activation == from.activation => {
                child.bias = from.bias;
                child.tau = from.tau;
                child.period = from.period;
            }
            (Gene::Connection(child), Gene::Connection(from))
                if (child.from, child.to) == (from.from, from.to) =>
            {
                child.weight = from.weight;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InnovationId, Rng};

    #[test]
    fn scalar_redraw_preserves_new_organ_parameters_and_bindings_exactly() {
        let mut params = SimParams::default();
        params.mutation.organs.add_sensor_rate = 1.0;
        params.mutation.organs.chemo_weight = 0.0;
        params.mutation.organs.energy_weight = 0.0;
        let mut plan_id = 100;
        let plan = FounderPlan::new(&params, &mut Rng::from_seed(0), || {
            let id = InnovationId::new(plan_id);
            plan_id += 1;
            id
        })
        .unwrap();
        let initial = crate::genome::fixtures::tiny();
        let mut expected = Vec::with_capacity(params.storage.max_genes as usize);
        let mut actual = Vec::with_capacity(params.storage.max_genes as usize);
        expected.extend_from_slice(&initial);
        actual.extend_from_slice(&initial);
        let mut a = Rng::from_seed(17);
        let mut b = Rng::from_seed(17);
        let mut a_next = 10;
        let mut b_next = 10;
        let mut a_scratch = vec![0; params.storage.max_neurons as usize];
        let mut b_scratch = a_scratch.clone();
        organs::apply(
            &mut expected,
            &params,
            &mut MutationState {
                rng: &mut a,
                next_innovation: &mut a_next,
                neuron_scratch: &mut a_scratch,
            },
            |_| {},
        );
        BrainInheritance::RandomizedAtBirth.prepare_offspring(
            &plan,
            &mut actual,
            &params,
            &mut MutationState {
                rng: &mut b,
                next_innovation: &mut b_next,
                neuron_scratch: &mut b_scratch,
            },
            |_| {},
        );
        let non_neural = |genes: &[Gene]| {
            genes
                .iter()
                .copied()
                .filter(|gene| !matches!(gene, Gene::Neuron(_) | Gene::Connection(_)))
                .collect::<Vec<_>>()
        };
        assert_eq!(non_neural(&actual), non_neural(&expected));
        assert_ne!(actual, expected);
        assert_eq!(a_next, b_next);
    }

    #[test]
    fn disabled_structure_preserves_each_legacy_scalar_path_and_rng() {
        let params = SimParams::default();
        let mut next = 0;
        let plan = FounderPlan::new(&params, &mut Rng::from_seed(0), || {
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
                BrainInheritance::Evolving | BrainInheritance::StructuralNullV2 => {
                    mutate::mutate(&mut expected, &mut a, &params.mutation)
                }
                BrainInheritance::RandomizedAtBirth | BrainInheritance::StructuralNull => {
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
