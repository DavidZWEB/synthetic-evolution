//! Shared native/WASM hand-worked distance and deletion-history comparisons.
//!
//! These pin measurement semantics, not species assignments or adaptive success.

use sim_core::InnovationId;
use sim_core::distance::between;
use sim_core::genome::{
    BodyGene, BodyTrait, ConnectionGene, EffectorGene, Gene, MetaGene, MetaTrait, Modality,
    NeuronGene, SensorGene,
};
use sim_core::params::DistanceParams;

pub fn neuron(id: u32) -> Gene {
    Gene::Neuron(NeuronGene {
        id: InnovationId::new(id),
        tau: 1.0,
        ..Default::default()
    })
}

pub fn connection(id: u32, from: u32, to: u32, weight: f32, enabled: bool) -> Gene {
    Gene::Connection(ConnectionGene {
        id: InnovationId::new(id),
        from: InnovationId::new(from),
        to: InnovationId::new(to),
        weight,
        enabled,
    })
}

pub fn hand_worked_pair() -> (Vec<Gene>, Vec<Gene>) {
    let mut a = vec![
        neuron(0),
        neuron(4),
        neuron(8),
        Gene::Sensor(SensorGene {
            id: InnovationId::new(20),
            modality: Modality::Interoception,
            targets: [
                InnovationId::new(0),
                InnovationId::NULL,
                InnovationId::NULL,
                InnovationId::NULL,
            ],
            ..Default::default()
        }),
        Gene::Effector(EffectorGene {
            id: InnovationId::new(30),
            source: InnovationId::new(4),
            ..Default::default()
        }),
        connection(10, 0, 4, 1.0, true),
        connection(14, 4, 8, -1.0, false),
        Gene::Body(BodyGene {
            trait_: BodyTrait::Size,
            value: 1.0,
        }),
        Gene::Meta(MetaGene {
            trait_: MetaTrait::MutationRate,
            value: 0.1,
        }),
    ];
    let mut b = vec![
        neuron(0),
        neuron(6),
        neuron(8),
        neuron(9),
        Gene::Effector(EffectorGene {
            id: InnovationId::new(30),
            source: InnovationId::new(8),
            ..Default::default()
        }),
        connection(10, 0, 8, 3.0, false),
        connection(12, 6, 8, 2.0, true),
        connection(15, 8, 9, 0.0, true),
    ];
    a.sort_by_key(Gene::sort_key);
    b.sort_by_key(Gene::sort_key);
    (a, b)
}

pub fn check_distance_cases() {
    let params = DistanceParams::default();
    let (a, b) = hand_worked_pair();
    let measured = between(&a, &b, &params);
    assert_eq!(measured.disjoint, 4);
    assert_eq!(measured.excess, 3);
    assert_eq!(measured.normalizer, 8);
    assert_eq!(measured.matching_connections, 1);
    assert_eq!(measured.mean_weight_difference, 2.0);
    // 4/8 + 3/8 + 2 * the configured f32 coefficient 0.4, evaluated as f64.
    assert_eq!(measured.value.to_bits(), 0x3ffa_cccc_d000_0000);
    assert_eq!(measured, between(&b, &a, &params));

    let retained = vec![neuron(0), neuron(1), connection(2, 0, 1, 0.75, true)];
    let mut toggled = retained.clone();
    if let Gene::Connection(c) = &mut toggled[2] {
        c.enabled = false;
    }
    assert_eq!(between(&retained, &toggled, &params).value, 0.0);
    let deleted = between(&retained, &retained[..2], &params);
    assert_eq!(
        (deleted.disjoint, deleted.excess, deleted.normalizer),
        (0, 1, 3)
    );
    assert_eq!(deleted.value.to_bits(), 0x3fd5_5555_5555_5555);
    for fresh_id in [3, 4, 100, u32::MAX - 1] {
        let recreated = [neuron(0), neuron(1), connection(fresh_id, 0, 1, 0.75, true)];
        let churn = between(&retained, &recreated, &params);
        assert_eq!((churn.disjoint, churn.excess, churn.normalizer), (1, 1, 3));
        assert_eq!(churn.matching_connections, 0);
        assert_eq!(churn.mean_weight_difference, 0.0);
        assert_eq!(churn.value.to_bits(), 0x3fe5_5555_5555_5555);
        assert_eq!(churn, between(&recreated, &retained, &params));
    }
}
