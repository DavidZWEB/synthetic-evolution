//! Allocation-free genetic distance over canonical typed-gene slices.
//!
//! Measures retained innovation history and matching connection weights (spec §3.4).
//! It does not classify species, reconcile innovations, or score ecological usefulness.

use core::cmp::Ordering;

use crate::genome::{self, Gene};
use crate::params::DistanceParams;

/// Components remain visible so calibration can distinguish marker churn from weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Distance {
    pub disjoint: usize,
    pub excess: usize,
    /// Larger innovation-bearing gene count, with a minimum of one.
    pub normalizer: usize,
    pub matching_connections: usize,
    pub mean_weight_difference: f64,
    pub value: f64,
}

/// Compare genomes accepted by [`genome::validate`] using validated coefficients.
///
/// Aligns by kind and innovation ID in linear time, with constant auxiliary space.
/// An unmatched ID beyond the other genome's maximum *of that kind* is excess;
/// all other unmatched IDs are disjoint. A missing kind makes all its IDs excess.
/// Body/meta traits contribute neither counts nor normalization. Disabled connections
/// still match and contribute weights; other scalar/binding differences are ignored.
///
/// This is not a metric on full genomes: distinct genomes can have zero distance.
/// Call [`DistanceParams::validate`] at standalone boundaries, or use coefficients
/// from an already validated [`crate::SimParams`].
pub fn between(mut a: &[Gene], mut b: &[Gene], params: &DistanceParams) -> Distance {
    debug_assert!(genome::validate(a).is_ok(), "genome A must be coherent");
    debug_assert!(genome::validate(b).is_ok(), "genome B must be coherent");
    debug_assert!(params.validate().is_ok(), "coefficients must be validated");
    let mut result = Distance {
        disjoint: 0,
        excess: 0,
        normalizer: 1,
        matching_connections: 0,
        mean_weight_difference: 0.0,
        value: 0.0,
    };
    let (mut a_count, mut b_count) = (0, 0);
    let mut weight_difference = 0.0;
    while let Some(kind) = a
        .first()
        .into_iter()
        .chain(b.first())
        .map(|gene| gene.sort_key().0)
        .min()
    {
        let (a_kind, a_rest) = a.split_at(a.partition_point(|gene| gene.sort_key().0 == kind));
        let (b_kind, b_rest) = b.split_at(b.partition_point(|gene| gene.sort_key().0 == kind));
        a = a_rest;
        b = b_rest;
        if a_kind
            .first()
            .or(b_kind.first())
            .is_some_and(|gene| gene.innovation().is_none())
        {
            continue;
        }
        a_count += a_kind.len();
        b_count += b_kind.len();
        let (mut i, mut j) = (0, 0);
        while i < a_kind.len() && j < b_kind.len() {
            match a_kind[i].sort_key().cmp(&b_kind[j].sort_key()) {
                Ordering::Less => {
                    result.disjoint += 1;
                    i += 1;
                }
                Ordering::Greater => {
                    result.disjoint += 1;
                    j += 1;
                }
                Ordering::Equal => {
                    if let (Gene::Connection(left), Gene::Connection(right)) =
                        (a_kind[i], b_kind[j])
                    {
                        // Widen before subtraction: opposite finite f32 weights can
                        // differ by more than f32::MAX (spec §3.4).
                        weight_difference +=
                            (f64::from(left.weight) - f64::from(right.weight)).abs();
                        result.matching_connections += 1;
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        result.excess += (a_kind.len() - i) + (b_kind.len() - j);
    }
    result.normalizer = a_count.max(b_count).max(1);
    if result.matching_connections > 0 {
        result.mean_weight_difference = weight_difference / result.matching_connections as f64;
    }
    result.value = f64::from(params.disjoint_coefficient)
        * (result.disjoint as f64 / result.normalizer as f64)
        + f64::from(params.excess_coefficient) * (result.excess as f64 / result.normalizer as f64)
        + f64::from(params.weight_coefficient) * result.mean_weight_difference;
    result
}
