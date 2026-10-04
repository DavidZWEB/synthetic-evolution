//! Body-trait mutation: size, muscle, mouth, and signature colour drift by birth.
//!
//! Owns each trait's step and the range it is clamped to (spec §3.5), not what the
//! traits do; movement, intake, and upkeep read them in the tick. Meta genes stay
//! fixed, and heredity modes decide nothing here: every child's body mutates alike.

use crate::genome::{BodyTrait, Gene};
use crate::math;
use crate::params::SimParams;
use crate::rng::Rng;

/// Steps each body-trait gene, in gene order, with probability
/// `mutation.body_trait_rate` (spec §3.5).
///
/// Size, muscle, and mouth are multiplied by `exp(σ·N(0, 1))`, so a trait explores in
/// proportion to its value and a step can never carry it through zero. Colour channels
/// take an additive `σ·N(0, 1)` step. Every result is clamped to its trait's range. A
/// zero rate draws nothing, so a world with body mutation off consumes the stream it
/// did before this operator existed.
pub(crate) fn apply(genes: &mut [Gene], params: &SimParams, rng: &mut Rng) {
    let rate = params.mutation.body_trait_rate;
    if rate <= 0.0 {
        return;
    }
    let sigma = params.mutation.body_trait_sigma;
    let body = &params.body;
    for gene in genes {
        let Gene::Body(gene) = gene else {
            continue;
        };
        if !rng.chance(rate) {
            continue;
        }
        let step = rng.normal(0.0, sigma);
        gene.value = match gene.trait_ {
            BodyTrait::Size => scaled(gene.value, step, body.size_range),
            BodyTrait::Muscle => scaled(gene.value, step, body.muscle_range),
            BodyTrait::Mouth => scaled(gene.value, step, body.mouth_range),
            BodyTrait::SignatureR | BodyTrait::SignatureG | BodyTrait::SignatureB => {
                (gene.value + step).clamp(0.0, 1.0)
            }
        };
    }
}

fn scaled(value: f32, step: f32, [low, high]: [f32; 2]) -> f32 {
    (value * math::exp(step)).clamp(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::BodyGene;

    fn body() -> Vec<Gene> {
        [
            (BodyTrait::Size, 3.0),
            (BodyTrait::SignatureR, 0.5),
            (BodyTrait::SignatureG, 0.0),
            (BodyTrait::SignatureB, 1.0),
            (BodyTrait::Muscle, 1.0),
            (BodyTrait::Mouth, 1.0),
        ]
        .into_iter()
        .map(|(trait_, value)| Gene::Body(BodyGene { trait_, value }))
        .collect()
    }

    fn values(genes: &[Gene]) -> Vec<f32> {
        genes
            .iter()
            .filter_map(|gene| match gene {
                Gene::Body(body) => Some(body.value),
                _ => None,
            })
            .collect()
    }

    fn mutating(sigma: f32) -> SimParams {
        let mut params = SimParams::default();
        params.mutation.body_trait_rate = 1.0;
        params.mutation.body_trait_sigma = sigma;
        params
    }

    #[test]
    fn a_zero_rate_changes_nothing_and_draws_nothing() {
        let mut genes = body();
        let mut rng = Rng::from_seed(3);
        let before = rng.state_fingerprint();
        apply(&mut genes, &SimParams::default(), &mut rng);
        assert_eq!(genes, body());
        assert_eq!(rng.state_fingerprint(), before);
    }

    #[test]
    fn each_trait_takes_its_own_kind_of_step() {
        // The same draws by hand: a chance test, then a normal, per body gene in order.
        let mut genes = body();
        let mut rng = Rng::from_seed(11);
        apply(&mut genes, &mutating(0.05), &mut rng);
        let mut expected = Rng::from_seed(11);
        let steps: Vec<f32> = (0..6)
            .map(|_| {
                assert!(expected.chance(1.0));
                expected.normal(0.0, 0.05)
            })
            .collect();
        let after = values(&genes);
        assert_eq!(after[0], 3.0 * math::exp(steps[0]), "size scales");
        assert_eq!(after[1], 0.5 + steps[1], "colour shifts");
        assert_eq!(after[2], steps[2].clamp(0.0, 1.0), "colour clamps");
        assert_eq!(after[3], (1.0 + steps[3]).clamp(0.0, 1.0), "colour clamps");
        assert_eq!(after[4], math::exp(steps[4]), "muscle scales");
        assert_eq!(after[5], math::exp(steps[5]), "mouth scales");
        assert_eq!(rng.state_fingerprint(), expected.state_fingerprint());
    }

    #[test]
    fn every_trait_stays_inside_its_range() {
        // A huge step pins traits to their bounds within a few births.
        let params = mutating(4.0);
        let mut genes = body();
        let mut rng = Rng::from_seed(7);
        let mut seen = [false; 2];
        for _ in 0..500 {
            apply(&mut genes, &params, &mut rng);
            for gene in &genes {
                let Gene::Body(body) = gene else { continue };
                let [low, high] = match body.trait_ {
                    BodyTrait::Size => params.body.size_range,
                    BodyTrait::Muscle => params.body.muscle_range,
                    BodyTrait::Mouth => params.body.mouth_range,
                    _ => [0.0, 1.0],
                };
                assert!((low..=high).contains(&body.value), "{body:?}");
                seen[0] |= body.value == low;
                seen[1] |= body.value == high;
            }
        }
        assert_eq!(seen, [true, true], "the clamp was never reached");
    }

    #[test]
    fn only_body_genes_change() {
        let mut genes = crate::genome::fixtures::tiny();
        let before = genes.clone();
        apply(&mut genes, &mutating(0.5), &mut Rng::from_seed(5));
        for (after, original) in genes.iter().zip(&before) {
            match (after, original) {
                (Gene::Body(_), Gene::Body(_)) => {}
                _ => assert_eq!(after, original),
            }
        }
        assert_ne!(genes, before, "no body gene moved");
    }
}
