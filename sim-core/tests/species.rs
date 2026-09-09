//! Standalone species policy, bounds, and churn; World integration is separate.
//!
//! All thresholds are explicit experiment values, not selected ecological defaults.

use sim_core::genome::Gene;
use sim_core::ids::{InnovationId, SpeciesId};
use sim_core::params::DistanceParams;
use sim_core::species::{Assignment, Classifier, Departure, Unclassified, UnknownSpecies};

#[path = "common/species_case.rs"]
mod common;
use common::{classifier, genome};

#[test]
fn lifecycle_and_marker_turnover_match_shared_native_wasm_cases() {
    common::check_species_cases();
    common::check_marker_turnover();
}

#[test]
fn threshold_is_strict_and_nearest_is_not_first_compatible() {
    let mut exact = classifier(3, 1.0);
    let root = exact.classify(&genome(0.0)).unwrap();
    let equal = exact.classify(&genome(1.0)).unwrap();
    assert!(equal.created);
    assert_ne!(root.species, equal.species);
    let mut below = classifier(2, 1.0);
    let root = below.classify(&genome(0.0)).unwrap();
    assert_eq!(
        below
            .classify(&genome(f32::from_bits(1.0f32.to_bits() - 1)))
            .unwrap(),
        Assignment {
            species: root.species,
            created: false
        }
    );
    let mut nearest = classifier(3, 3.0);
    nearest.classify(&genome(0.0)).unwrap();
    let second = nearest.classify(&genome(4.0)).unwrap();
    assert_eq!(
        nearest.classify(&genome(2.5)).unwrap().species,
        second.species
    );
}

#[test]
fn copied_representatives_are_immutable_and_outlive_the_original_member() {
    let mut classifier = classifier(2, 1.0);
    let mut source = genome(0.0);
    let first = classifier.classify(&source).unwrap();
    let Gene::Connection(connection) = &mut source[2] else {
        unreachable!()
    };
    connection.weight = 100.0;
    assert_eq!(
        classifier.representative(first.species),
        Some(&genome(0.0)[..])
    );
    assert_eq!(
        classifier.classify(&genome(0.25)).unwrap().species,
        first.species
    );
    assert_eq!(
        classifier.remove_member(first.species),
        Ok(Departure::MemberRemoved)
    );
    assert_eq!(
        classifier.representative(first.species),
        Some(&genome(0.0)[..])
    );
    assert_eq!(
        classifier.classify(&genome(-0.75)).unwrap().species,
        first.species
    );
}

#[test]
fn empty_genomes_still_consume_one_representative_slot() {
    let mut classifier = classifier(1, 0.5);
    let first = classifier.classify(&[]).unwrap();
    assert_eq!(classifier.representative(first.species), Some(&[][..]));
    assert_eq!(
        classifier.classify(&genome(0.0)),
        Err(Unclassified::Capacity)
    );
    assert_eq!(classifier.classify(&[]).unwrap().species, first.species);
    assert_eq!(
        classifier.remove_member(first.species),
        Ok(Departure::MemberRemoved)
    );
    assert_eq!(
        classifier.remove_member(first.species),
        Ok(Departure::Extinct)
    );
    assert!(classifier.classify(&genome(0.0)).unwrap().created);
}

#[test]
fn invalid_members_and_oversized_genomes_leave_state_unchanged() {
    let mut classifier = classifier(1, 0.5);
    let first = classifier.classify(&genome(0.0)).unwrap();
    for id in [SpeciesId::NULL, SpeciesId::new(99)] {
        assert_eq!(classifier.remove_member(id), Err(UnknownSpecies(id)));
    }
    let genes: Vec<_> = (0..9)
        .map(|id| {
            let mut gene = genome(0.0)[0];
            let Gene::Neuron(n) = &mut gene else {
                unreachable!()
            };
            n.id = InnovationId::new(id);
            gene
        })
        .collect();
    assert_eq!(
        classifier.classify(&genes),
        Err(Unclassified::GenomeTooLarge)
    );
    assert_eq!(
        classifier.active().collect::<Vec<_>>(),
        vec![(first.species, 1)]
    );
}

#[test]
fn invalid_layouts_coefficients_thresholds_and_budgets_fail_before_reservation() {
    for (capacity, max_genes) in [(1, 0), (u32::MAX, 2), (1, u32::MAX), (u32::MAX, 1)] {
        assert!(Classifier::estimated_construction_bytes(capacity, max_genes).is_err());
        assert!(
            Classifier::try_new(
                capacity,
                max_genes,
                1.0,
                DistanceParams::default(),
                u64::MAX
            )
            .is_err()
        );
    }
    for threshold in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(Classifier::try_new(1, 3, threshold, DistanceParams::default(), 1024).is_err());
    }
    for invalid in [-1.0, f32::NAN, f32::INFINITY] {
        let params = DistanceParams {
            weight_coefficient: invalid,
            ..Default::default()
        };
        assert!(Classifier::try_new(1, 3, 1.0, params, 1024).is_err());
    }
    let bytes = Classifier::estimated_construction_bytes(2, 8).unwrap();
    assert!(Classifier::try_new(2, 8, 1.0, DistanceParams::default(), bytes - 1).is_err());
    Classifier::try_new(2, 8, 1.0, DistanceParams::default(), bytes).unwrap();
    let mut disabled = Classifier::try_new(0, 8, 1.0, DistanceParams::default(), 0).unwrap();
    assert_eq!(disabled.classify(&genome(0.0)), Err(Unclassified::Capacity));
    assert_eq!(disabled.active().count(), 0);
}

#[test]
fn interleaved_classifiers_do_not_share_members_or_id_counters() {
    let mut a = classifier(8, 0.5);
    let mut b = classifier(8, 0.5);
    for weight in [0.0, 2.0, 0.25, 4.0] {
        assert_eq!(a.classify(&genome(weight)), b.classify(&genome(weight)));
    }
    let first = SpeciesId::new(0);
    a.remove_member(first).unwrap();
    assert_ne!(
        a.active().collect::<Vec<_>>(),
        b.active().collect::<Vec<_>>()
    );
    b.remove_member(first).unwrap();
    assert_eq!(
        a.active().collect::<Vec<_>>(),
        b.active().collect::<Vec<_>>()
    );
}

#[test]
fn full_capacity_churn_reuses_storage_without_reusing_ids() {
    let capacity = 256;
    let max_genes = 1024;
    let bytes = Classifier::estimated_construction_bytes(capacity, max_genes).unwrap();
    let mut classifier = Classifier::try_new(
        capacity,
        max_genes,
        1.0,
        DistanceParams {
            weight_coefficient: 1.0,
            ..Default::default()
        },
        bytes,
    )
    .unwrap();
    for index in 0..capacity {
        assert!(
            classifier
                .classify(&genome(index as f32 * 2.0))
                .unwrap()
                .created
        );
    }
    let start = std::time::Instant::now();
    let rounds = 2048;
    for index in 0..rounds {
        let oldest = classifier.active().next().unwrap().0;
        assert_eq!(classifier.remove_member(oldest), Ok(Departure::Extinct));
        let created = classifier
            .classify(&genome((capacity + index) as f32 * 2.0))
            .unwrap();
        assert!(created.created);
        assert_eq!(created.species.raw(), capacity + index);
        assert_eq!(classifier.active().count(), capacity as usize);
    }
    println!(
        "{rounds} retire/create cycles at {capacity} active species: {:?}; reserved {bytes} bytes",
        start.elapsed()
    );
    assert_eq!(
        classifier.active().map(|(_, members)| members).sum::<u32>(),
        capacity
    );
}
