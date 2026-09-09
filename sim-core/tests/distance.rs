//! Properties and independent reference comparisons for typed-gene distance.
//!
//! Classification and threshold policy are deliberately outside this first M4 slice.

use proptest::prelude::*;
use sim_core::InnovationId;
use sim_core::distance::{self, Distance};
use sim_core::genome::{
    self, BodyGene, BodyTrait, EffectorGene, Gene, MetaGene, MetaTrait, Modality, SensorGene,
};
use sim_core::params::DistanceParams;

#[path = "common/distance_case.rs"]
mod common;
use common::{connection, neuron};

#[test]
fn hand_worked_and_deletion_cases_match_the_cross_target_reference() {
    common::check_distance_cases();
}

#[test]
fn empty_and_trait_only_genomes_have_no_distance_or_normalization_weight() {
    let params = DistanceParams::default();
    let traits = [
        Gene::Body(BodyGene {
            trait_: BodyTrait::Size,
            value: 3.0,
        }),
        Gene::Meta(MetaGene {
            trait_: MetaTrait::MutationRate,
            value: 0.25,
        }),
    ];
    for (a, b) in [(&[][..], &[][..]), (&traits[..], &[][..])] {
        let measured = distance::between(a, b, &params);
        assert_eq!(
            measured,
            Distance {
                disjoint: 0,
                excess: 0,
                normalizer: 1,
                matching_connections: 0,
                mean_weight_difference: 0.0,
                value: 0.0,
            }
        );
    }
    let measured = distance::between(&[neuron(0)], &traits, &params);
    assert_eq!(
        (measured.disjoint, measured.excess, measured.normalizer),
        (0, 1, 1)
    );
    assert_eq!(measured.value, 1.0);
}

#[test]
fn same_numeric_innovation_in_a_different_kind_does_not_match() {
    let a = [neuron(0), neuron(1), neuron(2)];
    let b = [neuron(0), neuron(1), connection(2, 0, 1, 0.0, true)];
    let measured = distance::between(&a, &b, &DistanceParams::default());
    assert_eq!(
        (measured.disjoint, measured.excess, measured.normalizer),
        (0, 2, 3)
    );
    assert_eq!(measured.matching_connections, 0);
}

#[test]
fn ignored_scalars_and_bindings_do_not_turn_distance_into_full_genome_equality() {
    let (a, _) = common::hand_worked_pair();
    let mut b = a.clone();
    for gene in &mut b {
        match gene {
            Gene::Neuron(n) => {
                n.bias = 0.25;
                n.tau = 2.0;
            }
            Gene::Sensor(s) => {
                s.params[0] = 1.0;
                s.targets[0] = sim_core::InnovationId::new(8);
            }
            Gene::Effector(e) => {
                e.params[0] = 1.0;
                e.source = sim_core::InnovationId::new(0);
            }
            Gene::Connection(c) => {
                c.enabled = !c.enabled;
                c.from = sim_core::InnovationId::new(8);
            }
            Gene::Body(g) => g.value += 0.5,
            Gene::Meta(g) => g.value += 0.5,
        }
    }
    genome::validate(&b).unwrap();
    assert_ne!(a, b);
    let measured = distance::between(&a, &b, &DistanceParams::default());
    assert_eq!(measured.value, 0.0);
    assert_eq!(measured.matching_connections, 2);
}

#[test]
fn mean_uses_all_matching_connections_including_disabled_but_no_other_genes() {
    let a = [
        neuron(0),
        neuron(1),
        connection(2, 0, 1, -2.0, false),
        connection(3, 1, 0, 0.5, true),
    ];
    let b = [
        neuron(0),
        neuron(1),
        connection(2, 0, 1, 1.0, false),
        connection(3, 1, 0, 1.5, false),
    ];
    let params = DistanceParams {
        disjoint_coefficient: 0.0,
        excess_coefficient: 0.0,
        weight_coefficient: 1.0,
    };
    let measured = distance::between(&a, &b, &params);
    assert_eq!(measured.matching_connections, 2);
    assert_eq!(measured.mean_weight_difference, 2.0);
    assert_eq!(measured.value, 2.0);
}

#[test]
fn finite_f32_extremes_stay_finite_after_subtraction_and_weighting() {
    let a = [neuron(0), neuron(1), connection(2, 0, 1, f32::MAX, true)];
    let b = [neuron(0), neuron(1), connection(2, 0, 1, -f32::MAX, false)];
    let params = DistanceParams {
        disjoint_coefficient: f32::MAX,
        excess_coefficient: f32::MAX,
        weight_coefficient: f32::MAX,
    };
    params.validate().unwrap();
    let measured = distance::between(&a, &b, &params);
    assert_eq!(measured.mean_weight_difference, 2.0 * f64::from(f32::MAX));
    assert!(measured.value.is_finite() && measured.value > f64::from(f32::MAX));
    assert_eq!(measured, distance::between(&b, &a, &params));
    assert_eq!(distance::between(&a, &a, &params).value, 0.0);
}

fn reference(a: &[Gene], b: &[Gene], params: &DistanceParams) -> Distance {
    let mut disjoint = 0;
    let mut excess = 0;
    for (genes, other) in [(a, b), (b, a)] {
        for gene in genes.iter().filter(|g| g.innovation().is_some()) {
            if other.iter().any(|g| g.sort_key() == gene.sort_key()) {
                continue;
            }
            let maximum = other
                .iter()
                .filter(|g| g.innovation().is_some() && g.sort_key().0 == gene.sort_key().0)
                .map(|g| g.sort_key().1)
                .max();
            if maximum.is_none_or(|maximum| gene.sort_key().1 > maximum) {
                excess += 1;
            } else {
                disjoint += 1;
            }
        }
    }
    let mut matching_connections = 0;
    let mut sum = 0.0;
    for gene in a {
        if let Gene::Connection(left) = gene
            && let Some(Gene::Connection(right)) =
                b.iter().find(|g| g.sort_key() == gene.sort_key())
        {
            matching_connections += 1;
            sum += (f64::from(left.weight) - f64::from(right.weight)).abs();
        }
    }
    let count = |genes: &[Gene]| genes.iter().filter(|g| g.innovation().is_some()).count();
    let normalizer = count(a).max(count(b)).max(1);
    let mean_weight_difference = if matching_connections == 0 {
        0.0
    } else {
        sum / matching_connections as f64
    };
    Distance {
        disjoint,
        excess,
        normalizer,
        matching_connections,
        mean_weight_difference,
        value: f64::from(params.disjoint_coefficient) * (disjoint as f64 / normalizer as f64)
            + f64::from(params.excess_coefficient) * (excess as f64 / normalizer as f64)
            + f64::from(params.weight_coefficient) * mean_weight_difference,
    }
}

fn generated_genome() -> impl Strategy<Value = Vec<Gene>> {
    (
        prop::collection::btree_set(2u32..10, 0..8),
        prop::collection::btree_map(10u32..14, (-1000i32..1000, any::<bool>()), 0..4),
        prop::collection::btree_set(20u32..24, 0..4),
        prop::collection::btree_set(30u32..34, 0..4),
        prop::option::of(0.0f32..4.0),
        prop::option::of(0.0f32..1.0),
    )
        .prop_map(|(neurons, connections, sensors, effectors, body, meta)| {
            let mut genes = vec![neuron(0), neuron(1)];
            genes.extend(neurons.into_iter().map(neuron));
            genes.extend(sensors.into_iter().map(|id| {
                Gene::Sensor(SensorGene {
                    id: InnovationId::new(id),
                    modality: Modality::Interoception,
                    targets: [
                        InnovationId::new(id % 2),
                        InnovationId::NULL,
                        InnovationId::NULL,
                        InnovationId::NULL,
                    ],
                    ..Default::default()
                })
            }));
            genes.extend(effectors.into_iter().map(|id| {
                Gene::Effector(EffectorGene {
                    id: InnovationId::new(id),
                    source: InnovationId::new(id % 2),
                    ..Default::default()
                })
            }));
            genes.extend(connections.into_iter().map(|(id, (weight, enabled))| {
                connection(
                    id,
                    (id - 10) / 2,
                    (id - 10) % 2,
                    weight as f32 / 8.0,
                    enabled,
                )
            }));
            genes.extend(body.map(|value| {
                Gene::Body(BodyGene {
                    trait_: BodyTrait::Size,
                    value,
                })
            }));
            genes.extend(meta.map(|value| {
                Gene::Meta(MetaGene {
                    trait_: MetaTrait::MutationRate,
                    value,
                })
            }));
            genes
        })
}

proptest! {
    #[test]
    fn linear_walk_matches_reference_and_distance_properties(
        a in generated_genome(), b in generated_genome(),
        disjoint in 0.0f32..10.0, excess in 0.0f32..10.0, weight in 0.0f32..10.0,
    ) {
        let params = DistanceParams {
            disjoint_coefficient: disjoint,
            excess_coefficient: excess,
            weight_coefficient: weight,
        };
        prop_assert_eq!(genome::validate(&a), Ok(()));
        prop_assert_eq!(genome::validate(&b), Ok(()));
        let measured = distance::between(&a, &b, &params);
        prop_assert_eq!(measured, reference(&a, &b, &params));
        prop_assert_eq!(measured, distance::between(&b, &a, &params));
        prop_assert_eq!(distance::between(&a, &a, &params).value, 0.0);
        prop_assert!(measured.value.is_finite() && measured.value >= 0.0);
    }
}
