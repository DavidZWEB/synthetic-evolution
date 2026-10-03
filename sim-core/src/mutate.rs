//! Scalar mutation and the transient state shared by structural edits.
//!
//! Every operator works **in place on a gene slice**, because mutation happens on
//! every birth and a birth happens inside the tick. Nothing allocates.
//!
//! Finite scalar results and draw order retain the legacy behavior; bias overflow is
//! capped at the f32 representation boundary. `structural` owns bounded topology
//! changes, and the heredity policy that combines them lives in `control`.
//!
//! Neural and organ operators have separate owners and share bounded editing
//! primitives. Gene duplication and meta-gene mutation remain outside this milestone.

use crate::genome::{Activation, Gene};
use crate::params::MutationParams;
use crate::rng::Rng;

pub(crate) mod edit;
mod events;
pub(crate) mod organs;
pub mod structural;
pub use events::{
    OperatorCounts, StructuralMutationCounts, StructuralMutationEvent, StructuralMutationResult,
    StructuralOperator,
};

/// Borrowed world-owned mutation resources, not access to physical world state.
pub(crate) struct MutationState<'a> {
    pub rng: &'a mut Rng,
    pub next_innovation: &'a mut u32,
    pub neuron_scratch: &'a mut [u32],
}

/// Mutates a genome in place.
///
/// Draw order is fixed and does not depend on gene contents: every connection draws
/// its reset test then its perturbation test, in gene order. A conditional draw would
/// make the RNG stream depend on genome state and two structurally identical lineages
/// would desynchronise (spec §7.4).
pub fn mutate(genes: &mut [Gene], rng: &mut Rng, params: &MutationParams) {
    for gene in genes.iter_mut() {
        match gene {
            Gene::Connection(c) => {
                if rng.chance(params.weight_reset_rate) {
                    // Uniform resample: lets a weight escape a local optimum in one
                    // step, which a Gaussian nudge of any sane sigma cannot.
                    c.weight = rng.range(-params.weight_limit, params.weight_limit);
                } else if rng.chance(params.weight_perturb_rate) {
                    c.weight += rng.normal(0.0, params.weight_perturb_sigma);
                    c.weight = c.weight.clamp(-params.weight_limit, params.weight_limit);
                }
            }
            Gene::Neuron(n) => {
                if rng.chance(params.neuron_perturb_rate) {
                    // Preserve coherent input to structural edits even if a finite
                    // perturbation scale overflows f32; this is a representation bound.
                    n.bias = (n.bias + rng.normal(0.0, params.bias_perturb_sigma))
                        .clamp(-f32::MAX, f32::MAX);
                    // Multiplicative on tau, so it explores across orders of magnitude
                    // instead of random-walking off the bottom of its range.
                    let scale = 1.0 + rng.normal(0.0, params.tau_perturb_factor);
                    n.tau = (n.tau * scale.max(0.1)).clamp(TAU_FLOOR, TAU_CEILING);
                    if n.activation == Activation::Oscillator {
                        let scale = 1.0 + rng.normal(0.0, params.tau_perturb_factor);
                        n.period = (n.period * scale.max(0.1)).clamp(PERIOD_FLOOR, PERIOD_CEILING);
                    }
                }
            }
            // Body and meta traits are inherited but not yet mutable, and sensor and
            // effector params are fixed this phase. Their operators arrive with the
            // phases that need them (spec §3.3).
            Gene::Sensor(_) | Gene::Effector(_) | Gene::Body(_) | Gene::Meta(_) => {}
        }
    }
}

/// Hard bounds on evolvable neuron parameters.
///
/// Not tuning knobs: they are what stops a random walk producing a tau of zero (a
/// division by zero in the CTRNN) or a period so long the oscillator is a constant.
/// The useful range lives in `SimParams`, well inside these.
const TAU_FLOOR: f32 = 1e-3;
const TAU_CEILING: f32 = 1e3;
const PERIOD_FLOOR: f32 = 1.0;
const PERIOD_CEILING: f32 = 1e5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{Gene, GenomeError, fixtures::tiny, validate};
    use crate::params::{MutationParams, SimParams};
    use crate::rng::Rng;
    use proptest::prelude::*;

    #[test]
    fn finite_bias_genes_survive_extreme_valid_perturbations() {
        let mut genes = tiny();
        let params = MutationParams {
            neuron_perturb_rate: 1.0,
            bias_perturb_sigma: f32::MAX,
            ..MutationParams::default()
        };
        let mut rng = Rng::from_seed(42);
        for _ in 0..20 {
            mutate(&mut genes, &mut rng, &params);
            assert_eq!(validate(&genes), Ok(()));
        }
    }

    fn weights(genes: &[Gene]) -> Vec<f32> {
        genes
            .iter()
            .filter_map(|g| match g {
                Gene::Connection(c) => Some(c.weight),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mutation_moves_weights() {
        let params = SimParams::default().with_dense_founder();
        let mut next_id = 0;
        let plan = crate::founder::FounderPlan::new(&params, &mut Rng::from_seed(0), || {
            let id = crate::ids::InnovationId::new(next_id);
            next_id += 1;
            id
        })
        .expect("valid founder params");
        let mut genes = vec![Gene::default(); plan.genes().len()];
        plan.instantiate(&mut Rng::from_seed(2), &params, &mut genes);
        let before = weights(&genes);
        let mut rng = Rng::from_seed(1);
        mutate(&mut genes, &mut rng, &params.mutation);
        assert_ne!(weights(&genes), before, "nothing changed at default rates");
    }

    #[test]
    fn zero_rates_change_nothing() {
        let mut genes = tiny();
        let before = genes.clone();
        let mut rng = Rng::from_seed(1);
        let params = MutationParams {
            weight_perturb_rate: 0.0,
            weight_reset_rate: 0.0,
            neuron_perturb_rate: 0.0,
            ..MutationParams::default()
        };
        mutate(&mut genes, &mut rng, &params);
        assert_eq!(genes, before);
    }

    #[test]
    fn the_same_seed_produces_the_same_mutation() {
        let params = MutationParams::default();
        let mut a = tiny();
        let mut b = tiny();
        mutate(&mut a, &mut Rng::from_seed(77), &params);
        mutate(&mut b, &mut Rng::from_seed(77), &params);
        assert_eq!(a, b);
    }

    #[test]
    fn structural_genes_are_untouched_this_phase() {
        // Sensor params, effector bindings, body and meta traits have no operator in
        // Phase 1. If one grows, this test should be updated deliberately (spec §3.3).
        let mut genes = tiny();
        let before = genes.clone();
        let mut rng = Rng::from_seed(5);
        for _ in 0..100 {
            mutate(&mut genes, &mut rng, &MutationParams::default());
        }
        for (after, original) in genes.iter().zip(before.iter()) {
            match (after, original) {
                (Gene::Sensor(a), Gene::Sensor(b)) => assert_eq!(a, b),
                (Gene::Effector(a), Gene::Effector(b)) => assert_eq!(a, b),
                (Gene::Body(a), Gene::Body(b)) => assert_eq!(a, b),
                (Gene::Meta(a), Gene::Meta(b)) => assert_eq!(a, b),
                _ => {}
            }
        }
    }

    proptest! {
        /// The acceptance criterion for M4: mutation never orphans a reference.
        ///
        /// Phase 1's operators only touch scalars, so this cannot fail yet. It is
        /// written against the general claim rather than that shortcut so it starts
        /// failing the moment Phase 2's structural operators break it.
        #[test]
        fn mutation_never_produces_an_incoherent_genome(
            seed in any::<u64>(),
            rounds in 1usize..50,
            perturb in 0f32..1.0,
            reset in 0f32..1.0,
            neuron in 0f32..1.0,
        ) {
            let params = MutationParams {
                weight_perturb_rate: perturb,
                weight_reset_rate: reset,
                neuron_perturb_rate: neuron,
                ..MutationParams::default()
            };
            let mut genes = tiny();
            let mut rng = Rng::from_seed(seed);
            for _ in 0..rounds {
                mutate(&mut genes, &mut rng, &params);
                prop_assert_eq!(validate(&genes), Ok(()));
            }
        }

        /// A random walk must not drive tau or period into a division by zero or an
        /// infinity, however many generations it runs for.
        #[test]
        fn neuron_parameters_stay_in_their_bounds(seed in any::<u64>()) {
            let mut genes = crate::founder::FounderPlan::new(
                &SimParams::default(),
                &mut Rng::from_seed(0),
                {
                    let mut n = 0u32;
                    move || { n += 1; crate::ids::InnovationId::new(n - 1) }
                },
            ).expect("valid founder params").genes().to_vec();
            let mut rng = Rng::from_seed(seed);
            let params = MutationParams { neuron_perturb_rate: 1.0, ..MutationParams::default() };
            // Founder templates leave scalars at zero; give tau a legal start first.
            for gene in genes.iter_mut() {
                if let Gene::Neuron(n) = gene {
                    n.tau = 1.0;
                    n.period = 60.0;
                }
            }
            for _ in 0..2_000 {
                mutate(&mut genes, &mut rng, &params);
            }
            prop_assert_ne!(validate(&genes), Err(GenomeError::BadNeuronParameter));
            prop_assert_ne!(validate(&genes), Err(GenomeError::NonFinite));
        }

        /// Weights are bounded however long the walk runs, so the CTRNN cannot be
        /// driven into saturation by drift alone.
        #[test]
        fn weights_stay_within_their_limit(seed in any::<u64>()) {
            let params = MutationParams::default();
            let mut genes = tiny();
            let mut rng = Rng::from_seed(seed);
            for _ in 0..1_000 {
                mutate(&mut genes, &mut rng, &params);
            }
            for w in weights(&genes) {
                prop_assert!(w.abs() <= params.weight_limit + 1e-4, "weight escaped: {}", w);
            }
        }
    }
}
