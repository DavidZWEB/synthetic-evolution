//! Shared native/WASM species lifecycle and historical-marker scenarios.
//!
//! Thresholds here are controlled test values, not shipped classification defaults.

use sim_core::genome::{ConnectionGene, Gene, NeuronGene};
use sim_core::ids::{InnovationId, SpeciesId};
use sim_core::params::DistanceParams;
use sim_core::species::{Assignment, Classifier, Departure, Unclassified, UnknownSpecies};

pub fn genome(weight: f32) -> [Gene; 3] {
    [
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(0),
            tau: 1.0,
            ..Default::default()
        }),
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(1),
            tau: 1.0,
            ..Default::default()
        }),
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(2),
            from: InnovationId::new(0),
            to: InnovationId::new(1),
            weight,
            enabled: true,
        }),
    ]
}

pub fn classifier(capacity: u32, threshold: f64) -> Classifier {
    Classifier::try_new(
        capacity,
        8,
        threshold,
        DistanceParams {
            weight_coefficient: 1.0,
            ..Default::default()
        },
        Classifier::estimated_construction_bytes(capacity, 8).unwrap(),
    )
    .unwrap()
}

pub fn check_species_cases() {
    let mut classifier = classifier(2, 1.5);
    let low = classifier.classify(&genome(0.0)).unwrap();
    let high = classifier.classify(&genome(2.0)).unwrap();
    assert_eq!(
        low,
        Assignment {
            species: SpeciesId::new(0),
            created: true
        }
    );
    assert_eq!(
        high,
        Assignment {
            species: SpeciesId::new(1),
            created: true
        }
    );
    assert_eq!(
        classifier.classify(&genome(1.0)).unwrap(),
        Assignment {
            species: low.species,
            created: false,
        }
    );
    assert_eq!(
        classifier.classify(&genome(1.75)).unwrap().species,
        high.species
    );
    assert_eq!(
        classifier.remove_member(low.species),
        Ok(Departure::MemberRemoved)
    );
    assert_eq!(
        classifier.representative(low.species),
        Some(&genome(0.0)[..])
    );
    assert_eq!(
        classifier.remove_member(low.species),
        Ok(Departure::Extinct)
    );
    assert_eq!(
        classifier.remove_member(low.species),
        Err(UnknownSpecies(low.species))
    );
    assert!(classifier.representative(low.species).is_none());

    let reborn = classifier.classify(&genome(0.0)).unwrap();
    assert_eq!(
        reborn,
        Assignment {
            species: SpeciesId::new(2),
            created: true
        }
    );
    assert_eq!(
        classifier.classify(&genome(1.0)).unwrap().species,
        high.species
    );
    assert_eq!(
        classifier.active().collect::<Vec<_>>(),
        vec![(high.species, 3), (reborn.species, 1)]
    );
    assert_eq!(
        classifier.classify(&genome(10.0)),
        Err(Unclassified::Capacity)
    );
    assert_eq!(
        classifier.active().collect::<Vec<_>>(),
        vec![(high.species, 3), (reborn.species, 1)]
    );
    assert_eq!(
        classifier.classify(&genome(2.0)).unwrap().species,
        high.species
    );
    assert_eq!(
        classifier.remove_member(reborn.species),
        Ok(Departure::Extinct)
    );
    assert_eq!(
        classifier.classify(&genome(10.0)).unwrap().species,
        SpeciesId::new(3)
    );
}

pub fn check_marker_turnover() {
    let mut classifier = Classifier::try_new(8, 3, 0.5, DistanceParams::default(), 4096).unwrap();
    let original = genome(0.75);
    let root = classifier.classify(&original).unwrap();
    let mut toggled = original;
    let Gene::Connection(connection) = &mut toggled[2] else {
        unreachable!()
    };
    connection.enabled = false;
    assert_eq!(classifier.classify(&toggled).unwrap().species, root.species);
    for (offset, fresh_id) in [3, 4, 100, u32::MAX - 1].into_iter().enumerate() {
        let mut recreated = original;
        let Gene::Connection(connection) = &mut recreated[2] else {
            unreachable!()
        };
        connection.id = InnovationId::new(fresh_id);
        let assignment = classifier.classify(&recreated).unwrap();
        assert!(assignment.created);
        assert_eq!(assignment.species.raw(), offset as u32 + 1);
    }
    assert_eq!(
        classifier.active().map(|(_, members)| members).sum::<u32>(),
        6
    );
}
