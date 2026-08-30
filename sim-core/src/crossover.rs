//! NEAT-style recombination: aligning two genomes by innovation id.
//!
//! **Nothing calls this.** V1 reproduces asexually, and the sexual pathway does not
//! unlock until Phase 6 — after predation, because sex carries a twofold cost that
//! only Red Queen dynamics pay for (spec §3.4).
//!
//! It is written and tested now on purpose. Alignment is the piece most likely to have
//! subtle bugs, and debugging it inside a live ecosystem, where the symptom is
//! "evolution seems worse", is miserable. Written against a static test bench it is a
//! contained problem.
//!
//! Deliberately not here: genetic distance and species clustering. They reuse this
//! alignment walk and arrive with speciation in Phase 2.

use crate::genome::{Gene, GenomeError};
use crate::ids::InnovationId;
use crate::rng::Rng;

/// Which parent a disjoint or excess gene came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fitter {
    A,
    B,
    /// Neither is measurably better, so unmatched genes from both are kept.
    Equal,
}

/// Recombines two genomes into `out`.
///
/// Matching genes — the same innovation id in both parents — are taken from one
/// parent at random, which is what lets a lineage explore combinations of traits it
/// already has. Disjoint and excess genes come from the fitter parent only; taking
/// them from both would grow genomes monotonically and every offspring would be a
/// superset of its parents.
///
/// `out` is cleared and reused, so a caller that keeps one buffer allocates nothing
/// after the first call.
pub fn crossover(a: &[Gene], b: &[Gene], fitter: Fitter, rng: &mut Rng, out: &mut Vec<Gene>) {
    debug_assert!(
        crate::genome::validate(a).is_ok(),
        "parent A is not coherent"
    );
    debug_assert!(
        crate::genome::validate(b).is_ok(),
        "parent B is not coherent"
    );
    out.clear();

    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (ka, kb) = (a[i].sort_key(), b[j].sort_key());
        if ka == kb {
            // Matching gene: coin flip. Body and meta genes match by trait rather than
            // by innovation id, and inherit through the same path.
            out.push(if rng.chance(0.5) { a[i] } else { b[j] });
            i += 1;
            j += 1;
        } else if ka < kb {
            if fitter != Fitter::B {
                out.push(a[i]);
            }
            i += 1;
        } else {
            if fitter != Fitter::A {
                out.push(b[j]);
            }
            j += 1;
        }
    }

    // Excess: whatever runs past the end of the shorter genome.
    if fitter != Fitter::B {
        out.extend_from_slice(&a[i..]);
    }
    if fitter != Fitter::A {
        out.extend_from_slice(&b[j..]);
    }

    // Both parents are sorted and the walk is monotonic, so the result is too.
    debug_assert!(out.windows(2).all(|w| w[0].sort_key() < w[1].sort_key()));
    prune_dangling(out);
}

/// Drops genes whose neuron references did not survive recombination.
///
/// Inheriting disjoint genes from one parent can bring in a connection whose endpoint
/// neuron came from the other, which would leave the child referencing a neuron it
/// does not have. Alignment cannot prevent that; this repairs it, and is why
/// `crossover` can promise a valid genome out of any two valid genomes in.
fn prune_dangling(out: &mut Vec<Gene>) {
    let neurons: Vec<InnovationId> = out
        .iter()
        .take_while(|g| matches!(g, Gene::Neuron(_)))
        .filter_map(Gene::innovation)
        .collect();
    let known = |id: InnovationId| neurons.binary_search(&id).is_ok();

    out.retain(|gene| match gene {
        Gene::Sensor(s) => s.targets[..s.modality.channels()].iter().all(|&t| known(t)),
        Gene::Effector(e) => known(e.source),
        Gene::Connection(c) => known(c.from) && known(c.to),
        Gene::Neuron(_) | Gene::Body(_) | Gene::Meta(_) => true,
    });
}

/// Recombines and validates, for callers that would rather have the check than the
/// speed. The sim proper will use [`crossover`] and rely on the `debug_assert`.
pub fn crossover_checked(
    a: &[Gene],
    b: &[Gene],
    fitter: Fitter,
    rng: &mut Rng,
    out: &mut Vec<Gene>,
) -> Result<(), GenomeError> {
    crossover(a, b, fitter, rng, out);
    crate::genome::validate(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{ConnectionGene, Gene, NeuronGene, fixtures::tiny, validate};
    use crate::rng::Rng;
    use proptest::prelude::*;

    /// Drops genes at the given sort keys. Only ever used to remove genes nothing
    /// references, so both parents stay coherent.
    fn without(genes: &[Gene], drop: &[(u8, u32)]) -> Vec<Gene> {
        genes
            .iter()
            .copied()
            .filter(|g| !drop.contains(&g.sort_key()))
            .collect()
    }

    fn conn_keys(genes: &[Gene]) -> Vec<(u8, u32)> {
        genes
            .iter()
            .filter(|g| matches!(g, Gene::Connection(_)))
            .map(Gene::sort_key)
            .collect()
    }

    #[test]
    fn identical_parents_reproduce_themselves() {
        let a = tiny();
        let mut out = Vec::new();
        crossover(&a, &a, Fitter::Equal, &mut Rng::from_seed(3), &mut out);
        assert_eq!(out, a);
    }

    #[test]
    fn matching_genes_come_from_one_parent_or_the_other() {
        // Same structure, different weights: every gene in the child must be a gene
        // from one of the parents, never a blend.
        let a = tiny();
        let mut b = a.clone();
        for gene in b.iter_mut() {
            if let Gene::Connection(c) = gene {
                c.weight = -99.0;
            }
        }
        let mut out = Vec::new();
        crossover(&a, &b, Fitter::Equal, &mut Rng::from_seed(11), &mut out);
        assert_eq!(out.len(), a.len());
        for (i, gene) in out.iter().enumerate() {
            assert!(
                *gene == a[i] || *gene == b[i],
                "gene {i} is neither parent's"
            );
        }
    }

    #[test]
    fn disjoint_genes_follow_the_fitter_parent() {
        let a = tiny();
        let dropped = conn_keys(&a);
        let b = without(&a, &dropped);
        assert!(b.len() < a.len());

        let mut out = Vec::new();
        crossover(&a, &b, Fitter::A, &mut Rng::from_seed(1), &mut out);
        assert_eq!(
            out.len(),
            a.len(),
            "A is fitter, so A's extra genes should survive"
        );

        crossover(&a, &b, Fitter::B, &mut Rng::from_seed(1), &mut out);
        assert_eq!(
            out.len(),
            b.len(),
            "B is fitter, so A's extra genes should be dropped"
        );

        crossover(&a, &b, Fitter::Equal, &mut Rng::from_seed(1), &mut out);
        assert_eq!(out.len(), a.len(), "neither fitter, so keep the union");
    }

    #[test]
    fn excess_genes_past_the_shorter_parent_are_handled() {
        // Excess is the tail beyond where the shorter genome ends, and is a different
        // code path from disjoint genes in the middle.
        let a = tiny();
        let last = a.last().unwrap().sort_key();
        let b = without(&a, &[last]);
        let mut out = Vec::new();
        crossover(&a, &b, Fitter::A, &mut Rng::from_seed(2), &mut out);
        assert_eq!(out.len(), a.len());
        crossover(&a, &b, Fitter::B, &mut Rng::from_seed(2), &mut out);
        assert_eq!(out.len(), b.len());
    }

    #[test]
    fn the_child_is_sorted_and_free_of_duplicates() {
        let a = tiny();
        let b = without(&a, &conn_keys(&a)[..1]);
        let mut out = Vec::new();
        crossover(&a, &b, Fitter::Equal, &mut Rng::from_seed(4), &mut out);
        assert!(out.windows(2).all(|w| w[0].sort_key() < w[1].sort_key()));
    }

    #[test]
    fn a_connection_left_without_its_neuron_is_pruned() {
        // Defensive rather than reachable: two genomes sharing an innovation id should
        // agree on its endpoints. This proves the guard works if that ever stops
        // holding, which is exactly when it would be impossible to debug live.
        let ghost = InnovationId::new(900);
        let mut a = tiny();
        a.push(Gene::Connection(ConnectionGene {
            id: InnovationId::new(800),
            from: ghost,
            to: ghost,
            weight: 1.0,
            enabled: true,
        }));
        a.sort_by_key(Gene::sort_key);
        // `a` is deliberately incoherent, so go through the raw walk.
        let mut out = Vec::new();
        let b = tiny();
        crossover_ignoring_parent_checks(&a, &b, Fitter::A, &mut Rng::from_seed(6), &mut out);
        assert_eq!(validate(&out), Ok(()), "pruning left a dangling reference");
        assert!(
            !out.iter()
                .any(|g| g.innovation() == Some(InnovationId::new(800)))
        );
    }

    /// The same walk as [`crossover`] without the parent preconditions, so the pruning
    /// guard can be tested against input the asserts would otherwise reject.
    fn crossover_ignoring_parent_checks(
        a: &[Gene],
        b: &[Gene],
        fitter: Fitter,
        rng: &mut Rng,
        out: &mut Vec<Gene>,
    ) {
        out.clear();
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            let (ka, kb) = (a[i].sort_key(), b[j].sort_key());
            if ka == kb {
                out.push(if rng.chance(0.5) { a[i] } else { b[j] });
                i += 1;
                j += 1;
            } else if ka < kb {
                if fitter != Fitter::B {
                    out.push(a[i]);
                }
                i += 1;
            } else {
                if fitter != Fitter::A {
                    out.push(b[j]);
                }
                j += 1;
            }
        }
        if fitter != Fitter::B {
            out.extend_from_slice(&a[i..]);
        }
        if fitter != Fitter::A {
            out.extend_from_slice(&b[j..]);
        }
        super::prune_dangling(out);
    }

    #[test]
    fn a_neuron_only_in_one_parent_does_not_strand_its_connections() {
        let mut a = tiny();
        let extra = InnovationId::new(500);
        a.push(Gene::Neuron(NeuronGene {
            id: extra,
            bias: 0.0,
            tau: 1.0,
            ..NeuronGene::default()
        }));
        a.push(Gene::Connection(ConnectionGene {
            id: InnovationId::new(501),
            from: extra,
            to: extra,
            weight: 0.5,
            enabled: true,
        }));
        a.sort_by_key(Gene::sort_key);
        assert_eq!(validate(&a), Ok(()));

        let b = tiny();
        let mut out = Vec::new();
        for fitter in [Fitter::A, Fitter::B, Fitter::Equal] {
            crossover(&a, &b, fitter, &mut Rng::from_seed(8), &mut out);
            assert_eq!(
                validate(&out),
                Ok(()),
                "{fitter:?} produced an incoherent child"
            );
        }
    }

    #[test]
    fn the_output_buffer_is_reused_not_appended_to() {
        let a = tiny();
        let mut out = vec![a[0]; 99];
        crossover(&a, &a, Fitter::Equal, &mut Rng::from_seed(9), &mut out);
        assert_eq!(out.len(), a.len(), "stale contents survived");
    }

    proptest! {
        /// Crossover of two valid genomes yields a valid genome — the property the
        /// spec calls out as most likely to hide a subtle bug (spec §3.4, §7.8).
        #[test]
        fn any_two_valid_parents_make_a_valid_child(
            seed in any::<u64>(),
            drop_a in prop::collection::vec(0usize..5, 0..4),
            drop_b in prop::collection::vec(0usize..5, 0..4),
            fitter in prop::sample::select(vec![Fitter::A, Fitter::B, Fitter::Equal]),
        ) {
            let base = tiny();
            let keys = conn_keys(&base);
            let pick = |idx: &[usize]| -> Vec<(u8, u32)> {
                idx.iter().filter_map(|&i| keys.get(i % keys.len().max(1))).copied().collect()
            };
            let a = without(&base, &pick(&drop_a));
            let b = without(&base, &pick(&drop_b));
            prop_assert_eq!(validate(&a), Ok(()));
            prop_assert_eq!(validate(&b), Ok(()));

            let mut out = Vec::new();
            prop_assert_eq!(
                crossover_checked(&a, &b, fitter, &mut Rng::from_seed(seed), &mut out),
                Ok(())
            );
        }

        /// Recombination is deterministic given a seed, like everything else.
        #[test]
        fn the_same_seed_gives_the_same_child(seed in any::<u64>()) {
            let a = tiny();
            let mut b = a.clone();
            for gene in b.iter_mut() {
                if let Gene::Connection(c) = gene { c.weight *= -1.0; }
            }
            let (mut x, mut y) = (Vec::new(), Vec::new());
            crossover(&a, &b, Fitter::Equal, &mut Rng::from_seed(seed), &mut x);
            crossover(&a, &b, Fitter::Equal, &mut Rng::from_seed(seed), &mut y);
            prop_assert_eq!(x, y);
        }
    }
}
