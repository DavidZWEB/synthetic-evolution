//! Bounded sensor addition and removal on caller-owned genome storage.
//!
//! Owns organ selection and channel/target creation, not wiring or neural pruning.
//! The heredity pipeline schedules this pass before neural edits (spec section 3.3).

use crate::founder::sensor_parameters;
use crate::genome::{self, Activation, Gene, Modality, NeuronGene, SENSOR_CHANNELS, SensorGene};
use crate::ids::{InnovationId, reserve_innovations};
use crate::params::{OrganMutationParams, SimParams};
use crate::rng::Rng;

use super::edit::{Growth, insert_gene, preflight_growth, select_index};
use super::{MutationState, StructuralMutationEvent, StructuralMutationResult, StructuralOperator};

pub(crate) fn apply(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut MutationState<'_>,
    mut on_event: impl FnMut(StructuralMutationEvent),
) {
    for (operator, rate) in [
        (
            StructuralOperator::RemoveSensor,
            params.mutation.organs.remove_sensor_rate,
        ),
        (
            StructuralOperator::AddSensor,
            params.mutation.organs.add_sensor_rate,
        ),
    ] {
        if rate <= 0.0 || !state.rng.chance(rate) {
            continue;
        }
        let outcome = match operator {
            StructuralOperator::RemoveSensor => remove_sensor(genes, state.rng),
            StructuralOperator::AddSensor => add_sensor(genes, params, state),
            _ => unreachable!("only organ operators are scheduled here"),
        };
        debug_assert!(genome::validate_architecture(genes).is_ok());
        on_event(StructuralMutationEvent { operator, outcome });
    }
}

fn remove_sensor(genes: &mut Vec<Gene>, rng: &mut Rng) -> StructuralMutationResult {
    let Some(index) = select_index(genes, rng, |gene| matches!(gene, Gene::Sensor(_))) else {
        return StructuralMutationResult::NoCandidate;
    };
    // Targets may already participate in recurrent wiring; losing an organ does not
    // imply losing those neurons or their other functions (spec section 2.2c).
    genes.remove(index);
    StructuralMutationResult::Applied
}

fn choose_modality(params: &OrganMutationParams, rng: &mut Rng) -> Modality {
    let choices = [
        (Modality::VisionRay, f64::from(params.vision_weight)),
        (Modality::Chemo, f64::from(params.chemo_weight)),
        (Modality::Interoception, f64::from(params.energy_weight)),
    ];
    let total: f64 = choices.iter().map(|(_, weight)| weight).sum();
    debug_assert!(
        total.is_finite() && total > 0.0,
        "validated modality weights"
    );
    let mut pick = f64::from(rng.unit()) * total;
    for (modality, weight) in choices {
        if pick < weight {
            return modality;
        }
        pick -= weight;
    }
    unreachable!("a unit draw selects a positive-weight modality")
}

fn add_sensor(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut MutationState<'_>,
) -> StructuralMutationResult {
    let modality = choose_modality(&params.mutation.organs, state.rng);
    let channels = modality.channels();
    if let Err(outcome) = preflight_growth(
        genes,
        &params.storage,
        state.neuron_scratch,
        Growth {
            neurons: channels,
            sensors: 1,
            vision_rays: usize::from(modality == Modality::VisionRay),
            ..Growth::default()
        },
    ) {
        return outcome;
    }
    let Ok(first) = reserve_innovations(state.next_innovation, channels as u32 + 1) else {
        return StructuralMutationResult::InnovationExhausted;
    };
    let mut targets = [InnovationId::NULL; SENSOR_CHANNELS];
    for (channel, target) in targets.iter_mut().take(channels).enumerate() {
        *target = InnovationId::new(first.raw() + channel as u32);
        insert_gene(
            genes,
            Gene::Neuron(NeuronGene {
                id: *target,
                bias: params.mutation.organs.neuron_bias,
                tau: state.rng.range(params.brain.tau_min, params.brain.tau_max),
                activation: Activation::Sigmoid,
                period: 0.0,
            }),
        );
    }
    let sensor = SensorGene {
        id: InnovationId::new(first.raw() + channels as u32),
        modality,
        params: sensor_parameters(modality, &params.sensing, state.rng),
        targets,
    };
    insert_gene(genes, Gene::Sensor(sensor));
    StructuralMutationResult::Applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{ConnectionGene, fixtures::tiny};
    use crate::ids::NULL_ID;
    use crate::rng::Rng;
    use proptest::prelude::*;

    fn prepared(capacity: usize) -> Vec<Gene> {
        let mut genes = Vec::with_capacity(capacity);
        genes.push(Gene::Neuron(NeuronGene {
            id: InnovationId::new(0),
            bias: 0.0,
            tau: 1.0,
            activation: Activation::Sigmoid,
            period: 0.0,
        }));
        genes
    }

    fn params_for(modality: Modality) -> SimParams {
        let mut params = SimParams::default();
        let organs = &mut params.mutation.organs;
        organs.add_sensor_rate = 1.0;
        organs.vision_weight = if modality == Modality::VisionRay {
            1.0
        } else {
            0.0
        };
        organs.chemo_weight = if modality == Modality::Chemo {
            1.0
        } else {
            0.0
        };
        organs.energy_weight = if modality == Modality::Interoception {
            1.0
        } else {
            0.0
        };
        organs.neuron_bias = 0.375;
        params
    }

    fn run(
        genes: &mut Vec<Gene>,
        params: &SimParams,
        rng: &mut Rng,
        next: &mut u32,
        scratch: &mut [u32],
    ) -> Vec<StructuralMutationEvent> {
        let mut events = Vec::new();
        apply(
            genes,
            params,
            &mut MutationState {
                rng,
                next_innovation: next,
                neuron_scratch: scratch,
            },
            |event| events.push(event),
        );
        events
    }

    #[test]
    fn each_modality_adds_exact_targets_and_parameters_without_wiring() {
        for modality in [
            Modality::VisionRay,
            Modality::Chemo,
            Modality::Interoception,
        ] {
            let mut params = params_for(modality);
            params.brain.tau_min = 0.375;
            params.brain.tau_max = 1.625;
            params.sensing.vision_range = 37.0;
            params.sensing.vision_fov = 0.375;
            params.sensing.chemo_radius = 23.0;
            let mut genes = prepared(32);
            let original = genes[0];
            let pointer = genes.as_ptr();
            let mut next = 10;
            let mut rng = Rng::from_seed(42);
            let mut reference = rng.clone();
            reference.chance(1.0);
            reference.unit();
            let taus: Vec<_> = (0..modality.channels())
                .map(|_| reference.range(params.brain.tau_min, params.brain.tau_max))
                .collect();
            let expected_params = match modality {
                Modality::VisionRay => [
                    reference.range(-core::f32::consts::PI, core::f32::consts::PI),
                    0.0,
                    params.sensing.vision_range,
                    params.sensing.vision_fov,
                ],
                Modality::Chemo => [0.0, params.sensing.chemo_radius, 0.0, 0.0],
                Modality::Interoception => [0.0; 4],
            };
            let mut scratch = [91; 16];
            let events = run(&mut genes, &params, &mut rng, &mut next, &mut scratch);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].outcome, StructuralMutationResult::Applied);
            assert_eq!(genes.len(), modality.channels() + 2);
            assert_eq!(genome::neuron_count(&genes), modality.channels() + 1);
            assert!(!genes.iter().any(|g| matches!(g, Gene::Connection(_))));
            let sensor = genes
                .iter()
                .find_map(|g| match g {
                    Gene::Sensor(s) => Some(s),
                    _ => None,
                })
                .unwrap();
            assert_eq!(sensor.modality, modality);
            assert_eq!(sensor.id.raw(), 10 + modality.channels() as u32);
            assert_eq!(sensor.params, expected_params);
            for (channel, &target) in sensor.targets.iter().enumerate() {
                if channel < modality.channels() {
                    assert_eq!(target.raw(), 10 + channel as u32);
                    let n = genes[genome::neuron_index(&genes, target).unwrap()]
                        .as_neuron()
                        .unwrap();
                    assert_eq!(n.bias, params.mutation.organs.neuron_bias);
                    assert_eq!(n.tau, taus[channel]);
                    assert_eq!(n.activation, Activation::Sigmoid);
                    assert_eq!(n.period, 0.0);
                    assert!((params.brain.tau_min..=params.brain.tau_max).contains(&n.tau));
                } else {
                    assert!(target.is_null());
                }
            }
            match modality {
                Modality::VisionRay => {
                    assert_eq!(sensor.params[1], 0.0);
                    assert_eq!(sensor.params[2], params.sensing.vision_range);
                    assert_eq!(sensor.params[3], params.sensing.vision_fov);
                }
                Modality::Chemo => {
                    assert_eq!(sensor.params, [0.0, params.sensing.chemo_radius, 0.0, 0.0])
                }
                Modality::Interoception => assert_eq!(sensor.params, [0.0; 4]),
            }
            assert_eq!(next, 11 + modality.channels() as u32);
            assert_eq!(rng, reference);
            assert_eq!(scratch, [91; 16]);
            assert_eq!(genes[0], original);
            assert_eq!(genes.as_ptr(), pointer);
            genome::validate_architecture(&genes).unwrap();
        }
    }

    #[test]
    fn removal_preserves_neurons_wiring_and_other_genes() {
        let mut genes = tiny();
        let expected: Vec<_> = genes
            .iter()
            .copied()
            .filter(|g| !matches!(g, Gene::Sensor(_)))
            .collect();
        let mut params = SimParams::default();
        params.mutation.organs.remove_sensor_rate = 1.0;
        let mut next = NULL_ID;
        let events = run(
            &mut genes,
            &params,
            &mut Rng::from_seed(3),
            &mut next,
            &mut [],
        );
        assert_eq!(events[0].outcome, StructuralMutationResult::Applied);
        assert_eq!(genes, expected);
        assert_eq!(next, NULL_ID);
    }

    #[test]
    fn zero_rates_preserve_every_mutation_resource() {
        let mut genes = tiny();
        let before = genes.clone();
        let mut rng = Rng::from_seed(7);
        let original = rng.clone();
        let mut next = NULL_ID;
        let mut scratch = [91; 8];
        assert!(
            run(
                &mut genes,
                &SimParams::default(),
                &mut rng,
                &mut next,
                &mut scratch
            )
            .is_empty()
        );
        assert_eq!(genes, before);
        assert_eq!(rng, original);
        assert_eq!(next, NULL_ID);
        assert_eq!(scratch, [91; 8]);
    }

    #[test]
    fn refusal_is_atomic_and_consumes_no_initialization_draws() {
        for (modality, case) in [
            Modality::VisionRay,
            Modality::Chemo,
            Modality::Interoception,
        ]
        .into_iter()
        .flat_map(|modality| (0..8).map(move |case| (modality, case)))
        {
            if case == 3 && modality != Modality::VisionRay {
                continue;
            }
            let channels = modality.channels();
            let mut params = params_for(modality);
            let mut genes = prepared(if case == 5 { channels + 1 } else { 32 });
            let pointer = genes.as_ptr();
            let mut scratch = vec![91; if case == 4 { channels } else { 16 }];
            let original_scratch = scratch.clone();
            let mut next = match case {
                6 => NULL_ID - channels as u32,
                7 => NULL_ID,
                _ => 10,
            };
            match case {
                0 => params.storage.max_genes = channels as u32 + 1,
                1 => params.storage.max_neurons = channels as u32,
                2 => params.storage.max_sensors = 0,
                3 => params.storage.max_vision_rays = 0,
                _ => {}
            }
            let original = genes.clone();
            let original_id = next;
            let mut rng = Rng::from_seed(42);
            let mut reference = rng.clone();
            reference.chance(1.0);
            reference.unit();
            let events = run(&mut genes, &params, &mut rng, &mut next, &mut scratch);
            let expected = match case {
                0..=3 => StructuralMutationResult::GenomeLimit,
                4..=5 => StructuralMutationResult::ScratchLimit,
                _ => StructuralMutationResult::InnovationExhausted,
            };
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].outcome, expected);
            assert_eq!(genes, original);
            assert_eq!(next, original_id);
            assert_eq!(rng, reference);
            assert_eq!(scratch, original_scratch);
            assert_eq!(genes.as_ptr(), pointer);
        }
    }

    #[test]
    fn final_id_range_can_create_every_modality_without_null_ids() {
        for modality in [
            Modality::VisionRay,
            Modality::Chemo,
            Modality::Interoception,
        ] {
            let mut genes = prepared(32);
            let first = NULL_ID - modality.channels() as u32 - 1;
            let mut next = first;
            let params = params_for(modality);
            let mut rng = Rng::from_seed(1);
            let events = run(&mut genes, &params, &mut rng, &mut next, &mut [0; 16]);
            assert_eq!(events[0].outcome, StructuralMutationResult::Applied);
            assert_eq!(next, NULL_ID);
            let Gene::Sensor(sensor) = genes.last().unwrap() else {
                panic!("sensor sorts after the neurons");
            };
            assert_eq!(sensor.id.raw(), NULL_ID - 1);
            for (channel, target) in sensor.targets[..modality.channels()].iter().enumerate() {
                assert_eq!(target.raw(), first + channel as u32);
            }
            genome::validate_architecture(&genes).unwrap();
            let original = genes.clone();
            let mut reference = rng.clone();
            reference.chance(1.0);
            reference.unit();
            let events = run(&mut genes, &params, &mut rng, &mut next, &mut [0; 16]);
            assert_eq!(
                events[0].outcome,
                StructuralMutationResult::InnovationExhausted
            );
            assert_eq!(genes, original);
            assert_eq!(rng, reference);
            assert_eq!(next, NULL_ID);
        }
    }

    #[test]
    fn remove_then_add_is_the_fixed_order() {
        let mut genes = Vec::with_capacity(32);
        genes.extend_from_slice(&tiny());
        let mut params = params_for(Modality::VisionRay);
        params.mutation.organs.remove_sensor_rate = 1.0;
        params.storage.max_sensors = 1;
        params.storage.max_genes = genes.len() as u32 + 4;
        let events = run(
            &mut genes,
            &params,
            &mut Rng::from_seed(9),
            &mut 10,
            &mut [0; 16],
        );
        assert_eq!(
            events.iter().map(|e| e.operator).collect::<Vec<_>>(),
            [
                StructuralOperator::RemoveSensor,
                StructuralOperator::AddSensor
            ]
        );
        assert!(
            events
                .iter()
                .all(|e| e.outcome == StructuralMutationResult::Applied)
        );
        assert_eq!(
            genes
                .iter()
                .filter(|g| matches!(g, Gene::Sensor(_)))
                .count(),
            1
        );
        assert!(
            genes
                .iter()
                .any(|g| matches!(g, Gene::Connection(ConnectionGene { id, .. }) if id.raw() == 4))
        );
    }

    #[test]
    fn extreme_finite_weights_do_not_overflow_selection() {
        let weights = OrganMutationParams {
            vision_weight: f32::MAX,
            chemo_weight: f32::MAX,
            energy_weight: f32::MAX,
            ..OrganMutationParams::default()
        };
        let mut rng = Rng::from_seed(42);
        let mut seen = [false; 3];
        for _ in 0..100 {
            match choose_modality(&weights, &mut rng) {
                Modality::VisionRay => seen[0] = true,
                Modality::Chemo => seen[1] = true,
                Modality::Interoception => seen[2] = true,
            }
        }
        assert_eq!(seen, [true; 3]);
    }

    proptest! {
        #[test]
        fn repeated_edits_stay_coherent_bounded_and_reproducible(
            seed in any::<u64>(),
            edits in prop::collection::vec(any::<bool>(), 1..200),
        ) {
            let execute = || {
                let mut genes = prepared(64);
                let mut params = SimParams::default();
                params.storage.max_genes = 64;
                params.storage.max_neurons = 32;
                params.storage.max_sensors = 8;
                params.storage.max_vision_rays = 4;
                let mut rng = Rng::from_seed(seed);
                let mut next = 10;
                let mut scratch = [0; 32];
                for &add in &edits {
                    params.mutation.organs.add_sensor_rate = if add { 1.0 } else { 0.0 };
                    params.mutation.organs.remove_sensor_rate = if add { 0.0 } else { 1.0 };
                    let before = genes.clone();
                    let id = next;
                    let outcome = run(&mut genes, &params, &mut rng, &mut next, &mut scratch)[0].outcome;
                    genome::validate_architecture(&genes).unwrap();
                    crate::spawn::validate_limits(&genes, &params.storage).unwrap();
                    assert_eq!(genes.capacity(), 64);
                    assert!(genes.iter().filter_map(Gene::innovation).all(|gene| gene.raw() < next));
                    if outcome != StructuralMutationResult::Applied {
                        assert_eq!(genes, before);
                        assert_eq!(next, id);
                    }
                }
                (genes, rng, next)
            };
            prop_assert_eq!(execute(), execute());
        }
    }
}
