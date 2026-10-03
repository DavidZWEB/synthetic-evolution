//! Bounded, deterministic neural topology edits on caller-owned genome storage.
//! This module preserves architecture and reports attempted edits; scalar mutation,
//! birth admission, world-owned resources, and heredity policy live elsewhere.
//! Sensor/organ operators live in `mutate/organs.rs`; this module stays neural-only.

use super::edit::{Growth, insert_gene, preflight_growth, select_index};
use crate::genome::{self, Activation, ConnectionGene, Gene, NeuronGene};
use crate::ids::{InnovationId, reserve_innovations};
use crate::math;
use crate::params::SimParams;
use crate::rng::Rng;

// Preserve the M2 observation paths while both operator families share their types.
pub use super::{
    OperatorCounts, StructuralMutationCounts, StructuralMutationEvent, StructuralMutationResult,
    StructuralOperator,
};

pub(crate) fn apply(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut super::MutationState<'_>,
    mut on_event: impl FnMut(StructuralMutationEvent),
) {
    let rates = &params.mutation.structural;
    // The order and zero-rate short circuit preserve the scalar-only RNG stream
    // while allowing each later edit to see earlier edits on this child (spec §3.3).
    for (operator, rate) in [
        (
            StructuralOperator::RemoveConnection,
            rates.remove_connection_rate,
        ),
        (StructuralOperator::RemoveNeuron, rates.remove_neuron_rate),
        (
            StructuralOperator::ToggleConnection,
            rates.toggle_connection_rate,
        ),
        (StructuralOperator::AddConnection, rates.add_connection_rate),
        (StructuralOperator::AddNeuron, rates.add_neuron_rate),
        (StructuralOperator::AddOscillator, rates.add_oscillator_rate),
    ] {
        if rate <= 0.0 || !state.rng.chance(rate) {
            continue;
        }
        let outcome = match operator {
            StructuralOperator::RemoveConnection => remove_connection(genes, state.rng),
            StructuralOperator::RemoveNeuron => {
                remove_neuron(genes, state.rng, state.neuron_scratch)
            }
            StructuralOperator::ToggleConnection => toggle_connection(genes, state.rng),
            StructuralOperator::AddConnection => add_connection(genes, params, state),
            StructuralOperator::AddNeuron => add_neuron(genes, params, state),
            StructuralOperator::AddOscillator => add_oscillator(genes, params, state),
            StructuralOperator::RemoveSensor | StructuralOperator::AddSensor => {
                unreachable!("organ operators have their own pass")
            }
        };
        debug_assert!(genome::validate_architecture(genes).is_ok());
        on_event(StructuralMutationEvent { operator, outcome });
    }
}

fn remove_connection(genes: &mut Vec<Gene>, rng: &mut Rng) -> StructuralMutationResult {
    let Some(index) = select_index(genes, rng, |gene| matches!(gene, Gene::Connection(_))) else {
        return StructuralMutationResult::NoCandidate;
    };
    genes.remove(index);
    StructuralMutationResult::Applied
}

fn remove_neuron(
    genes: &mut Vec<Gene>,
    rng: &mut Rng,
    scratch: &mut [u32],
) -> StructuralMutationResult {
    let neurons = genome::neuron_count(genes);
    if neurons == 0 {
        return StructuralMutationResult::NoCandidate;
    }
    if scratch.len() < neurons {
        return StructuralMutationResult::ScratchLimit;
    }
    let protected = &mut scratch[..neurons];
    protected.fill(0);
    for (index, gene) in genes.iter().enumerate() {
        match gene {
            Gene::Neuron(n) if n.activation == Activation::Oscillator => protected[index] = 1,
            Gene::Sensor(s) => {
                for &target in &s.targets[..s.modality.channels()] {
                    protected
                        [genome::neuron_index(genes, target).expect("validated sensor target")] = 1;
                }
            }
            Gene::Effector(e) => {
                protected
                    [genome::neuron_index(genes, e.source).expect("validated effector source")] = 1;
            }
            _ => {}
        }
    }
    // Oscillators are scaffold, and every active organ binding must survive
    // pruning; inactive sensor padding does not bind a neuron (spec §3.2–§3.3).
    let Some(index) = select_index(protected, rng, |&bound| bound == 0) else {
        return StructuralMutationResult::NoCandidate;
    };
    let id = genes[index].as_neuron().expect("neuron prefix").id;
    genes.retain(|gene| match gene {
        Gene::Neuron(n) => n.id != id,
        Gene::Connection(c) => c.from != id && c.to != id,
        _ => true,
    });
    StructuralMutationResult::Applied
}

fn toggle_connection(genes: &mut [Gene], rng: &mut Rng) -> StructuralMutationResult {
    let Some(index) = select_index(genes, rng, |gene| matches!(gene, Gene::Connection(_))) else {
        return StructuralMutationResult::NoCandidate;
    };
    if let Gene::Connection(c) = &mut genes[index] {
        c.enabled = !c.enabled;
    }
    StructuralMutationResult::Applied
}

fn pair_rank(rng: &mut Rng, eligible: u64) -> u64 {
    debug_assert!(eligible > 0);
    if eligible <= u64::from(u32::MAX) {
        u64::from(rng.below(eligible as u32))
    } else {
        rng.next_u64() % eligible
    }
}

fn add_connection(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut super::MutationState<'_>,
) -> StructuralMutationResult {
    let neurons = genome::neuron_count(genes);
    let active = genes
        .iter()
        .filter(|gene| matches!(gene, Gene::Connection(c) if c.enabled))
        .count();
    let eligible = (neurons as u64) * (neurons as u64) - active as u64;
    if eligible == 0 {
        return StructuralMutationResult::NoCandidate;
    }
    if state.neuron_scratch.len() < neurons {
        return StructuralMutationResult::ScratchLimit;
    }
    let scratch = &mut state.neuron_scratch[..neurons];
    scratch.fill(0);
    for gene in genes.iter() {
        if let Gene::Connection(c) = gene
            && c.enabled
        {
            scratch[genome::neuron_index(genes, c.from).expect("validated connection source")] += 1;
        }
    }
    // Rank the complement of enabled edges without materializing n² pairs. Reuse
    // outgoing degrees as a selected-row target mask; disabled edges remain eligible.
    let mut rank = pair_rank(state.rng, eligible);
    let source = scratch
        .iter()
        .position(|&degree| {
            let width = neurons as u64 - u64::from(degree);
            if rank < width {
                true
            } else {
                rank -= width;
                false
            }
        })
        .expect("eligible pair has a source row");
    let from = genes[source].as_neuron().expect("neuron prefix").id;
    scratch.fill(0);
    for gene in genes.iter() {
        if let Gene::Connection(c) = gene
            && c.enabled
            && c.from == from
        {
            scratch[genome::neuron_index(genes, c.to).expect("validated connection target")] = 1;
        }
    }
    let target = scratch
        .iter()
        .enumerate()
        .filter(|(_, marked)| **marked == 0)
        .nth(rank as usize)
        .map(|(index, _)| index)
        .expect("eligible pair has a target");
    let to = genes[target].as_neuron().expect("neuron prefix").id;
    let mut retained = None;
    let mut fan_in = 0u64;
    for (index, gene) in genes.iter().enumerate() {
        if let Gene::Connection(c) = gene {
            if c.enabled && c.to == to {
                fan_in += 1;
            }
            if c.from == from && c.to == to {
                retained = Some(index);
            }
        }
    }
    if let Some(index) = retained {
        if let Gene::Connection(c) = &mut genes[index] {
            c.enabled = true;
        }
        return StructuralMutationResult::Applied;
    }
    if let Err(outcome) = preflight_growth(
        genes,
        &params.storage,
        state.neuron_scratch,
        Growth {
            connections: 1,
            ..Growth::default()
        },
    ) {
        return outcome;
    }
    let Ok(id) = reserve_innovations(state.next_innovation, 1) else {
        return StructuralMutationResult::InnovationExhausted;
    };
    let half_width = params
        .brain
        .weight_init_scale
        .min(params.mutation.weight_limit)
        / math::sqrt((fan_in + 1) as f32);
    let weight = state.rng.range(-half_width, half_width);
    insert_gene(
        genes,
        Gene::Connection(ConnectionGene {
            id,
            from,
            to,
            weight,
            enabled: true,
        }),
    );
    StructuralMutationResult::Applied
}

fn add_neuron(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut super::MutationState<'_>,
) -> StructuralMutationResult {
    let Some(index) = select_index(
        genes,
        state.rng,
        |gene| matches!(gene, Gene::Connection(c) if c.enabled),
    ) else {
        return StructuralMutationResult::NoCandidate;
    };
    if let Err(outcome) = preflight_growth(
        genes,
        &params.storage,
        state.neuron_scratch,
        Growth {
            neurons: 1,
            connections: 2,
            ..Growth::default()
        },
    ) {
        return outcome;
    }
    let Ok(id) = reserve_innovations(state.next_innovation, 3) else {
        return StructuralMutationResult::InnovationExhausted;
    };
    let Gene::Connection(old) = &mut genes[index] else {
        unreachable!("selected a connection");
    };
    old.enabled = false;
    let old = *old;
    let neuron = NeuronGene {
        id,
        bias: params.mutation.structural.split_neuron_bias,
        tau: state.rng.range(params.brain.tau_min, params.brain.tau_max),
        activation: Activation::Sigmoid,
        period: 0.0,
    };
    insert_gene(genes, Gene::Neuron(neuron));
    insert_gene(
        genes,
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(id.raw() + 1),
            from: old.from,
            to: id,
            weight: params.mutation.structural.split_input_weight,
            enabled: true,
        }),
    );
    insert_gene(
        genes,
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(id.raw() + 2),
            from: id,
            to: old.to,
            weight: old.weight,
            enabled: true,
        }),
    );
    StructuralMutationResult::Applied
}

/// Adds a free-running oscillator and one enabled connection from it to a uniformly
/// chosen non-oscillator neuron. Oscillators ignore their inputs, so an unwired clock
/// would be inert until a later edit wired it; this one can act at once (spec §3.2).
fn add_oscillator(
    genes: &mut Vec<Gene>,
    params: &SimParams,
    state: &mut super::MutationState<'_>,
) -> StructuralMutationResult {
    let is_target =
        |gene: &Gene| matches!(gene, Gene::Neuron(n) if n.activation != Activation::Oscillator);
    let Some(index) = select_index(genes, state.rng, is_target) else {
        return StructuralMutationResult::NoCandidate;
    };
    let to = genes[index].as_neuron().expect("neuron prefix").id;
    if let Err(outcome) = preflight_growth(
        genes,
        &params.storage,
        state.neuron_scratch,
        Growth {
            neurons: 1,
            connections: 1,
            ..Growth::default()
        },
    ) {
        return outcome;
    }
    let Ok(id) = reserve_innovations(state.next_innovation, 2) else {
        return StructuralMutationResult::InnovationExhausted;
    };
    let fan_in = genes
        .iter()
        .filter(|gene| matches!(gene, Gene::Connection(c) if c.enabled && c.to == to))
        .count() as u64;
    let brain = &params.brain;
    // Drawn like a founder oscillator and an added connection, in that order.
    let neuron = NeuronGene {
        id,
        bias: state.rng.range(-1.0, 1.0),
        tau: state.rng.range(brain.tau_min, brain.tau_max),
        activation: Activation::Oscillator,
        period: state
            .rng
            .range(brain.oscillator_period_min, brain.oscillator_period_max),
    };
    let half_width =
        brain.weight_init_scale.min(params.mutation.weight_limit) / math::sqrt((fan_in + 1) as f32);
    let weight = state.rng.range(-half_width, half_width);
    insert_gene(genes, Gene::Neuron(neuron));
    insert_gene(
        genes,
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(id.raw() + 1),
            from: id,
            to,
            weight,
            enabled: true,
        }),
    );
    StructuralMutationResult::Applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{
        Action, BodyGene, BodyTrait, EffectorGene, MetaGene, MetaTrait, Modality, SensorGene,
    };
    use crate::ids::NULL_ID;
    use crate::mutate::MutationState;
    use crate::rng::Rng;
    use proptest::prelude::*;

    const OPERATORS: [StructuralOperator; 6] = [
        StructuralOperator::RemoveConnection,
        StructuralOperator::RemoveNeuron,
        StructuralOperator::ToggleConnection,
        StructuralOperator::AddConnection,
        StructuralOperator::AddNeuron,
        StructuralOperator::AddOscillator,
    ];
    const OUTCOMES: [StructuralMutationResult; 5] = [
        StructuralMutationResult::Applied,
        StructuralMutationResult::NoCandidate,
        StructuralMutationResult::GenomeLimit,
        StructuralMutationResult::ScratchLimit,
        StructuralMutationResult::InnovationExhausted,
    ];

    fn neuron(id: u32, activation: Activation) -> Gene {
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(id),
            bias: -0.25,
            tau: SimParams::default().brain.tau_min,
            activation,
            period: if activation == Activation::Oscillator {
                SimParams::default().brain.oscillator_period_min
            } else {
                0.0
            },
        })
    }

    fn connection(id: u32, from: u32, to: u32, weight: f32, enabled: bool) -> Gene {
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(id),
            from: InnovationId::new(from),
            to: InnovationId::new(to),
            weight,
            enabled,
        })
    }

    fn reserved(genes: impl IntoIterator<Item = Gene>, capacity: usize) -> Vec<Gene> {
        let mut result = Vec::with_capacity(capacity);
        result.extend(genes);
        result.sort_unstable_by_key(Gene::sort_key);
        assert_eq!(genome::validate_architecture(&result), Ok(()));
        result
    }

    fn set_rate(params: &mut SimParams, operator: StructuralOperator, rate: f32) {
        let structural = &mut params.mutation.structural;
        match operator {
            StructuralOperator::RemoveConnection => structural.remove_connection_rate = rate,
            StructuralOperator::RemoveNeuron => structural.remove_neuron_rate = rate,
            StructuralOperator::ToggleConnection => structural.toggle_connection_rate = rate,
            StructuralOperator::AddConnection => structural.add_connection_rate = rate,
            StructuralOperator::AddNeuron => structural.add_neuron_rate = rate,
            StructuralOperator::AddOscillator => structural.add_oscillator_rate = rate,
            StructuralOperator::RemoveSensor | StructuralOperator::AddSensor => {
                unreachable!("neural test helper")
            }
        }
    }

    fn params_for(operator: StructuralOperator) -> SimParams {
        let mut params = SimParams::default();
        set_rate(&mut params, operator, 1.0);
        params
    }

    fn run(
        genes: &mut Vec<Gene>,
        params: &SimParams,
        rng: &mut Rng,
        next: &mut u32,
        scratch: &mut [u32],
    ) -> StructuralMutationEvent {
        let mut event = None;
        apply(
            genes,
            params,
            &mut MutationState {
                rng,
                next_innovation: next,
                neuron_scratch: scratch,
            },
            |observed| assert!(event.replace(observed).is_none(), "more than one attempt"),
        );
        event.expect("unit rate must attempt")
    }

    fn bound_genome(capacity: usize) -> Vec<Gene> {
        let mut genes = reserved(
            (0..9).map(|id| {
                neuron(
                    id,
                    if id == 7 {
                        Activation::Oscillator
                    } else {
                        Activation::Sigmoid
                    },
                )
            }),
            capacity,
        );
        genes.extend([
            Gene::Sensor(SensorGene {
                id: InnovationId::new(9),
                modality: Modality::VisionRay,
                targets: [0, 1, 2, 3].map(InnovationId::new),
                ..SensorGene::default()
            }),
            Gene::Sensor(SensorGene {
                id: InnovationId::new(10),
                modality: Modality::Chemo,
                targets: [
                    InnovationId::new(1),
                    InnovationId::new(4),
                    InnovationId::new(5),
                    InnovationId::NULL,
                ],
                ..SensorGene::default()
            }),
            Gene::Effector(EffectorGene {
                id: InnovationId::new(11),
                action: Action::Thrust,
                source: InnovationId::new(6),
                ..EffectorGene::default()
            }),
            connection(12, 0, 6, 0.75, true),
            connection(13, 8, 0, 0.25, true),
            connection(14, 0, 8, -0.5, false),
            connection(15, 8, 8, -0.75, false),
            connection(16, 8, 6, 1.25, false),
            Gene::Body(BodyGene {
                trait_: BodyTrait::Size,
                value: 1.0,
            }),
            Gene::Meta(MetaGene {
                trait_: MetaTrait::MutationRate,
                value: 0.5,
            }),
        ]);
        assert_eq!(genome::validate_architecture(&genes), Ok(()));
        genes
    }

    fn find_connection(genes: &[Gene], id: u32) -> ConnectionGene {
        genes
            .iter()
            .find_map(|gene| match gene {
                Gene::Connection(c) if c.id.raw() == id => Some(*c),
                _ => None,
            })
            .expect("connection exists")
    }

    #[test]
    fn zero_rates_preserve_genes_rng_counter_and_scratch() {
        let mut genes = bound_genome(32);
        let before = genes.clone();
        let mut rng = Rng::from_seed(51);
        let before_rng = rng.clone();
        let mut next = NULL_ID;
        let mut scratch = [123; 1];
        apply(
            &mut genes,
            &SimParams::default(),
            &mut MutationState {
                rng: &mut rng,
                next_innovation: &mut next,
                neuron_scratch: &mut scratch,
            },
            |_| panic!("zero rate emitted an attempt"),
        );
        assert_eq!(genes, before);
        assert_eq!(rng, before_rng);
        assert_eq!(next, NULL_ID);
        assert_eq!(scratch, [123]);
    }

    #[test]
    fn failed_chance_draws_once_without_an_attempt_or_candidate_selection() {
        for operator in OPERATORS {
            let mut genes = bound_genome(32);
            let before = genes.clone();
            let mut rng = Rng::from_seed(51);
            let mut expected_rng = rng.clone();
            let draw = expected_rng.unit();
            assert!(draw > 0.0);
            let mut params = SimParams::default();
            set_rate(&mut params, operator, draw * 0.5);
            let mut next = 17;
            apply(
                &mut genes,
                &params,
                &mut MutationState {
                    rng: &mut rng,
                    next_innovation: &mut next,
                    neuron_scratch: &mut [],
                },
                |_| panic!("failed chance emitted an attempt"),
            );
            assert_eq!(rng, expected_rng);
            assert_eq!(genes, before);
            assert_eq!(next, 17);
        }
    }

    #[test]
    fn attempted_operators_run_once_in_order_on_the_previous_edits() {
        let mut genes = reserved(
            [
                neuron(0, Activation::Sigmoid),
                connection(1, 0, 0, 0.75, true),
            ],
            8,
        );
        let mut params = SimParams::default();
        for operator in OPERATORS {
            set_rate(&mut params, operator, 1.0);
        }
        let mut rng = Rng::from_seed(17);
        let mut expected_rng = rng.clone();
        let mut next = 2;
        let mut events = Vec::new();
        apply(
            &mut genes,
            &params,
            &mut MutationState {
                rng: &mut rng,
                next_innovation: &mut next,
                neuron_scratch: &mut [0; 4],
            },
            |event| events.push(event),
        );
        let expected: Vec<_> = OPERATORS
            .into_iter()
            .enumerate()
            .map(|(index, operator)| {
                assert!(expected_rng.chance(1.0));
                if index < 2 {
                    expected_rng.below(1);
                }
                StructuralMutationEvent {
                    operator,
                    outcome: if index < 2 {
                        StructuralMutationResult::Applied
                    } else {
                        StructuralMutationResult::NoCandidate
                    },
                }
            })
            .collect();
        assert_eq!(events, expected);
        assert_eq!(rng, expected_rng);
        assert!(genes.is_empty());
        assert_eq!(next, 2);
    }

    #[test]
    fn remove_connection_physically_deletes_one_enabled_or_disabled_edge() {
        for enabled in [false, true] {
            let mut genes = reserved(
                [
                    neuron(0, Activation::Sigmoid),
                    neuron(1, Activation::Sigmoid),
                    connection(2, 0, 0, 0.5, enabled),
                    connection(3, 0, 1, -0.75, enabled),
                    connection(4, 1, 0, 1.5, enabled),
                ],
                5,
            );
            let before = genes.clone();
            let mut next = NULL_ID;
            assert_eq!(
                run(
                    &mut genes,
                    &params_for(StructuralOperator::RemoveConnection),
                    &mut Rng::from_seed(19),
                    &mut next,
                    &mut [],
                )
                .outcome,
                StructuralMutationResult::Applied
            );
            assert_eq!(genes.len(), before.len() - 1);
            assert_eq!(&genes[..2], &before[..2]);
            assert!(genes.iter().all(|gene| before.contains(gene)));
            assert_eq!(next, NULL_ID);
        }
    }

    #[test]
    fn remove_neuron_protects_every_active_binding_and_oscillator_and_removes_incidents() {
        let mut genes = bound_genome(32);
        let before = genes.clone();
        let expected: Vec<_> = before
            .iter()
            .copied()
            .filter(|gene| match gene {
                Gene::Neuron(n) => n.id.raw() != 8,
                Gene::Connection(c) => c.from.raw() != 8 && c.to.raw() != 8,
                _ => true,
            })
            .collect();
        let mut next = NULL_ID;
        let mut rng = Rng::from_seed(7);
        let params = params_for(StructuralOperator::RemoveNeuron);
        assert_eq!(
            run(&mut genes, &params, &mut rng, &mut next, &mut [0; 9]).outcome,
            StructuralMutationResult::Applied
        );
        assert_eq!(genes, expected);
        assert_eq!(next, NULL_ID);
        let mut expected_rng = rng.clone();
        expected_rng.chance(1.0);
        assert_eq!(
            run(&mut genes, &params, &mut rng, &mut next, &mut [0; 9]).outcome,
            StructuralMutationResult::NoCandidate
        );
        assert_eq!(genes, expected);
        assert_eq!(rng, expected_rng);
    }

    #[test]
    fn inactive_sensor_channels_do_not_protect_an_unbound_neuron() {
        let mut genes = bound_genome(32);
        for gene in &mut genes {
            if let Gene::Sensor(s) = gene
                && s.modality == Modality::Chemo
            {
                s.targets[3] = InnovationId::new(8);
            }
        }
        assert_eq!(genome::validate_architecture(&genes), Ok(()));
        assert_eq!(
            run(
                &mut genes,
                &params_for(StructuralOperator::RemoveNeuron),
                &mut Rng::from_seed(23),
                &mut 17,
                &mut [0; 9],
            )
            .outcome,
            StructuralMutationResult::Applied
        );
        assert!(genome::neuron_index(&genes, InnovationId::new(8)).is_none());
        assert_eq!(genome::validate_architecture(&genes), Ok(()));
    }

    #[test]
    fn remove_neuron_removes_at_most_one_unbound_neuron() {
        let mut genes = reserved((0..8).map(|id| neuron(id, Activation::Sigmoid)), 8);
        let mut next = NULL_ID;
        assert_eq!(
            run(
                &mut genes,
                &params_for(StructuralOperator::RemoveNeuron),
                &mut Rng::from_seed(23),
                &mut next,
                &mut [0; 8],
            )
            .outcome,
            StructuralMutationResult::Applied
        );
        assert_eq!(genes.len(), 7);
        assert_eq!(next, NULL_ID);
    }

    #[test]
    fn toggle_preserves_identity_and_weight_and_needs_no_ids_or_scratch() {
        let mut genes = reserved(
            [
                neuron(0, Activation::Sigmoid),
                connection(1, 0, 0, -1.75, true),
            ],
            2,
        );
        let original = genes.clone();
        let mut next = NULL_ID;
        let mut rng = Rng::from_seed(29);
        for enabled in [false, true] {
            assert_eq!(
                run(
                    &mut genes,
                    &params_for(StructuralOperator::ToggleConnection),
                    &mut rng,
                    &mut next,
                    &mut [],
                )
                .outcome,
                StructuralMutationResult::Applied
            );
            let mut expected = original.clone();
            if let Gene::Connection(c) = &mut expected[1] {
                c.enabled = enabled;
            }
            assert_eq!(genes, expected);
            assert_eq!(next, NULL_ID);
        }
    }

    #[test]
    fn reenable_uses_retained_id_and_weight_at_all_growth_limits_and_id_exhaustion() {
        for pair in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let mut genes = reserved(
                [
                    neuron(0, Activation::Sigmoid),
                    neuron(1, Activation::Sigmoid),
                ],
                6,
            );
            for (index, (from, to)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
                genes.push(connection(
                    index as u32 + 2,
                    from,
                    to,
                    -(index as f32 + 0.25),
                    (from, to) != pair,
                ));
            }
            let mut expected = genes.clone();
            for gene in &mut expected {
                if let Gene::Connection(c) = gene {
                    c.enabled = true;
                }
            }
            let mut params = params_for(StructuralOperator::AddConnection);
            params.storage.max_genes = 6;
            params.storage.max_neurons = 2;
            params.storage.max_connections = 4;
            let mut rng = Rng::from_seed(31);
            let mut expected_rng = rng.clone();
            expected_rng.chance(1.0);
            expected_rng.below(1);
            let mut next = NULL_ID;
            assert_eq!(
                run(&mut genes, &params, &mut rng, &mut next, &mut [0; 2]).outcome,
                StructuralMutationResult::Applied
            );
            assert_eq!(genes, expected);
            assert_eq!(
                rng, expected_rng,
                "re-enabling must not initialize a weight"
            );
            assert_eq!(next, NULL_ID);
            assert_eq!(genes.capacity(), 6);
        }
    }

    #[test]
    fn add_connection_fills_all_ordered_pairs_with_configured_fan_in_scaled_weights() {
        for (scale, limit) in [(2.5, 0.65), (0.65, 2.5)] {
            let mut params = params_for(StructuralOperator::AddConnection);
            params.brain.weight_init_scale = scale;
            params.mutation.weight_limit = limit;
            let mut genes = reserved((0..3).map(|id| neuron(id, Activation::Sigmoid)), 12);
            let pointer = genes.as_ptr();
            let mut next = 3;
            let mut rng = Rng::from_seed(37);
            for active in 0..9 {
                let before = genes.clone();
                let mut expected_rng = rng.clone();
                expected_rng.chance(1.0);
                expected_rng.below(9 - active);
                assert_eq!(
                    run(&mut genes, &params, &mut rng, &mut next, &mut [0; 3]).outcome,
                    StructuralMutationResult::Applied
                );
                let added = find_connection(&genes, next - 1);
                let previous_fan_in = before
                    .iter()
                    .filter(
                        |gene| matches!(gene, Gene::Connection(c) if c.enabled && c.to == added.to),
                    )
                    .count();
                let half_width = params
                    .brain
                    .weight_init_scale
                    .min(params.mutation.weight_limit)
                    / math::sqrt((previous_fan_in + 1) as f32);
                assert_eq!(added.weight, expected_rng.range(-half_width, half_width));
                assert!(added.weight.abs() <= half_width);
                assert_eq!(rng, expected_rng);
                assert_eq!(genes.len(), before.len() + 1);
                assert_eq!(genes.as_ptr(), pointer);
                assert_eq!(genome::validate_architecture(&genes), Ok(()));
            }
            for from in 0..3 {
                for to in 0..3 {
                    assert!(genes.iter().any(|gene| matches!(gene, Gene::Connection(c)
                        if c.from.raw() == from && c.to.raw() == to && c.enabled)));
                }
            }
            let before = genes.clone();
            let mut expected_rng = rng.clone();
            expected_rng.chance(1.0);
            assert_eq!(
                run(&mut genes, &params, &mut rng, &mut next, &mut [0; 3]).outcome,
                StructuralMutationResult::NoCandidate
            );
            assert_eq!(genes, before);
            assert_eq!(rng, expected_rng);
            assert_eq!(next, 12);
        }
    }

    #[test]
    fn split_retains_the_old_edge_and_uses_configured_scalars_for_self_and_recurrent_edges() {
        for (from, to) in [(0, 0), (0, 1), (1, 0)] {
            let mut params = params_for(StructuralOperator::AddNeuron);
            params.mutation.structural.split_neuron_bias = -2.25;
            params.mutation.structural.split_input_weight = -0.75;
            params.brain.tau_min = 0.3;
            params.brain.tau_max = 0.8;
            let mut genes = reserved(
                [
                    neuron(0, Activation::Sigmoid),
                    neuron(1, Activation::Oscillator),
                    connection(2, from, to, -1.25, true),
                    connection(3, 1, 1, 0.75, false),
                ],
                7,
            );
            let before = genes.clone();
            let mut rng = Rng::from_seed(41);
            let mut expected_rng = rng.clone();
            expected_rng.chance(1.0);
            expected_rng.below(1);
            let tau = expected_rng.range(params.brain.tau_min, params.brain.tau_max);
            let mut next = 4;
            assert_eq!(
                run(&mut genes, &params, &mut rng, &mut next, &mut [0; 3]).outcome,
                StructuralMutationResult::Applied
            );
            assert_eq!(rng, expected_rng);
            assert_eq!(next, 7);
            assert_eq!(genes.len(), before.len() + 3);
            assert_eq!(&genes[..2], &before[..2]);
            assert_eq!(
                genes[2],
                Gene::Neuron(NeuronGene {
                    id: InnovationId::new(4),
                    bias: params.mutation.structural.split_neuron_bias,
                    tau,
                    activation: Activation::Sigmoid,
                    period: 0.0,
                })
            );
            assert!((params.brain.tau_min..=params.brain.tau_max).contains(&tau));
            assert_eq!(
                Gene::Connection(find_connection(&genes, 2)),
                connection(2, from, to, -1.25, false)
            );
            assert_eq!(find_connection(&genes, 3), find_connection(&before, 3));
            assert_eq!(
                Gene::Connection(find_connection(&genes, 5)),
                connection(
                    5,
                    from,
                    4,
                    params.mutation.structural.split_input_weight,
                    true
                )
            );
            assert_eq!(
                Gene::Connection(find_connection(&genes, 6)),
                connection(6, 4, to, find_connection(&before, 2).weight, true)
            );
            assert_eq!(genome::validate_architecture(&genes), Ok(()));
        }
    }

    #[test]
    fn disabled_edges_are_not_split_candidates() {
        let mut genes = reserved(
            [
                neuron(0, Activation::Sigmoid),
                connection(1, 0, 0, 1.25, false),
            ],
            8,
        );
        let before = genes.clone();
        let mut rng = Rng::from_seed(43);
        let mut expected_rng = rng.clone();
        expected_rng.chance(1.0);
        let mut next = 2;
        assert_eq!(
            run(
                &mut genes,
                &params_for(StructuralOperator::AddNeuron),
                &mut rng,
                &mut next,
                &mut [0; 2],
            )
            .outcome,
            StructuralMutationResult::NoCandidate
        );
        assert_eq!(genes, before);
        assert_eq!(rng, expected_rng);
        assert_eq!(next, 2);
    }

    #[test]
    fn growth_refusals_are_atomic_and_consume_no_initialization_draws_or_ids() {
        for operator in [
            StructuralOperator::AddConnection,
            StructuralOperator::AddNeuron,
        ] {
            let splitting = operator == StructuralOperator::AddNeuron;
            let initial: Vec<_> = if splitting {
                vec![
                    neuron(0, Activation::Sigmoid),
                    connection(1, 0, 0, -0.75, true),
                ]
            } else {
                vec![neuron(0, Activation::Sigmoid)]
            };
            for refusal in 0..8 {
                if !splitting && matches!(refusal, 2 | 6 | 7) {
                    continue;
                }
                let mut params = params_for(operator);
                let growth = if splitting { 3 } else { 1 };
                let capacity = initial.len() + growth - usize::from(refusal == 3);
                let mut genes = reserved(initial.iter().copied(), capacity);
                let pointer = genes.as_ptr();
                let mut scratch = vec![0; if splitting { 2 } else { 1 }];
                let mut next = 2;
                let expected = match refusal {
                    0 => {
                        params.storage.max_genes = (initial.len() + growth - 1) as u32;
                        StructuralMutationResult::GenomeLimit
                    }
                    1 => {
                        params.storage.max_connections = if splitting { 2 } else { 0 };
                        StructuralMutationResult::GenomeLimit
                    }
                    2 => {
                        params.storage.max_neurons = 1;
                        StructuralMutationResult::GenomeLimit
                    }
                    3 => StructuralMutationResult::ScratchLimit,
                    4 => {
                        scratch.pop();
                        StructuralMutationResult::ScratchLimit
                    }
                    _ => {
                        next = NULL_ID - (refusal - 5);
                        StructuralMutationResult::InnovationExhausted
                    }
                };
                let before_next = next;
                let mut rng = Rng::from_seed(47);
                let mut expected_rng = rng.clone();
                expected_rng.chance(1.0);
                // Pair selection itself needs scratch; splitting does not.
                if splitting || refusal != 4 {
                    expected_rng.below(1);
                }
                assert_eq!(
                    run(&mut genes, &params, &mut rng, &mut next, &mut scratch).outcome,
                    expected,
                    "{operator:?}, refusal {refusal}"
                );
                assert_eq!(genes, initial, "{operator:?}, refusal {refusal}");
                assert_eq!(next, before_next);
                assert_eq!(rng, expected_rng);
                assert_eq!(genes.capacity(), capacity);
                assert_eq!(genes.as_ptr(), pointer);
            }
        }
    }

    #[test]
    fn remove_neuron_scratch_refusal_preserves_candidate_rng_and_ids() {
        let mut genes = bound_genome(32);
        let before = genes.clone();
        let mut rng = Rng::from_seed(53);
        let mut expected_rng = rng.clone();
        expected_rng.chance(1.0);
        let mut next = NULL_ID;
        assert_eq!(
            run(
                &mut genes,
                &params_for(StructuralOperator::RemoveNeuron),
                &mut rng,
                &mut next,
                &mut [0; 8],
            )
            .outcome,
            StructuralMutationResult::ScratchLimit
        );
        assert_eq!(genes, before);
        assert_eq!(rng, expected_rng);
        assert_eq!(next, NULL_ID);
    }

    #[test]
    fn exact_growth_limits_and_last_available_ids_succeed_without_null_innovations() {
        for operator in [
            StructuralOperator::AddConnection,
            StructuralOperator::AddNeuron,
        ] {
            let splitting = operator == StructuralOperator::AddNeuron;
            let mut params = params_for(operator);
            params.storage.max_genes = if splitting { 5 } else { 2 };
            params.storage.max_neurons = if splitting { 2 } else { 1 };
            params.storage.max_connections = if splitting { 3 } else { 1 };
            let mut genes = reserved(
                [neuron(0, Activation::Sigmoid)],
                params.storage.max_genes as usize,
            );
            if splitting {
                genes.push(connection(1, 0, 0, 0.5, true));
            }
            let mut scratch = vec![0; params.storage.max_neurons as usize];
            let mut next = NULL_ID - if splitting { 3 } else { 1 };
            assert_eq!(
                run(
                    &mut genes,
                    &params,
                    &mut Rng::from_seed(59),
                    &mut next,
                    &mut scratch,
                )
                .outcome,
                StructuralMutationResult::Applied
            );
            assert_eq!(next, NULL_ID);
            assert_eq!(genes.len(), params.storage.max_genes as usize);
            assert_eq!(
                genome::neuron_count(&genes),
                params.storage.max_neurons as usize
            );
            assert!(
                genes
                    .iter()
                    .filter_map(Gene::innovation)
                    .all(|id| !id.is_null())
            );
            assert_eq!(genome::validate_architecture(&genes), Ok(()));
        }
    }

    #[test]
    fn pair_rank_handles_u32_and_larger_spaces_without_large_allocations() {
        for eligible in [
            1,
            u64::from(u32::MAX),
            u64::from(u32::MAX) + 1,
            u64::from(u32::MAX) * u64::from(u32::MAX),
        ] {
            let mut rng = Rng::from_seed(61);
            let mut expected_rng = rng.clone();
            for _ in 0..128 {
                let expected = if eligible <= u64::from(u32::MAX) {
                    u64::from(expected_rng.below(eligible as u32))
                } else {
                    expected_rng.next_u64() % eligible
                };
                let rank = pair_rank(&mut rng, eligible);
                assert_eq!(rank, expected);
                assert!(rank < eligible);
            }
            assert_eq!(rng, expected_rng);
        }
    }

    #[test]
    fn added_oscillators_get_a_period_and_drive_one_non_oscillator() {
        let params = params_for(StructuralOperator::AddOscillator);
        for seed in 0..32 {
            let mut genes = reserved(
                [
                    neuron(1, Activation::Sigmoid),
                    neuron(2, Activation::Oscillator),
                    neuron(3, Activation::Sigmoid),
                    connection(4, 1, 3, 0.5, true),
                ],
                params.storage.max_genes as usize,
            );
            let before = genes.clone();
            let mut rng = Rng::from_seed(seed);
            let mut next = 10;
            let mut scratch = vec![0; params.storage.max_neurons as usize];
            let mut outcome = None;
            apply(
                &mut genes,
                &params,
                &mut MutationState {
                    rng: &mut rng,
                    next_innovation: &mut next,
                    neuron_scratch: &mut scratch,
                },
                |event| outcome = Some(event),
            );
            assert_eq!(
                outcome,
                Some(StructuralMutationEvent {
                    operator: StructuralOperator::AddOscillator,
                    outcome: StructuralMutationResult::Applied,
                })
            );
            assert_eq!(next, 12, "a neuron and a connection");
            assert_eq!(genes.len(), before.len() + 2);
            let Some(Gene::Neuron(added)) = genes
                .iter()
                .find(|gene| gene.innovation() == Some(InnovationId::new(10)))
            else {
                panic!("no added neuron");
            };
            assert_eq!(added.activation, Activation::Oscillator);
            assert!(
                (params.brain.oscillator_period_min..=params.brain.oscillator_period_max)
                    .contains(&added.period)
            );
            let Some(Gene::Connection(wire)) = genes
                .iter()
                .find(|gene| gene.innovation() == Some(InnovationId::new(11)))
            else {
                panic!("no added connection");
            };
            assert_eq!(wire.from, added.id);
            assert!(wire.enabled);
            assert!(
                [1, 3].contains(&wire.to.raw()),
                "an oscillator ignores its inputs"
            );
            assert!(genome::validate_architecture(&genes).is_ok());
        }
    }

    #[test]
    fn oscillator_addition_needs_a_non_oscillator_target() {
        let params = params_for(StructuralOperator::AddOscillator);
        let mut genes = reserved(
            [neuron(1, Activation::Oscillator)],
            params.storage.max_genes as usize,
        );
        let mut outcome = None;
        apply(
            &mut genes,
            &params,
            &mut MutationState {
                rng: &mut Rng::from_seed(1),
                next_innovation: &mut 10,
                neuron_scratch: &mut vec![0; params.storage.max_neurons as usize],
            },
            |event| outcome = Some(event.outcome),
        );
        assert_eq!(outcome, Some(StructuralMutationResult::NoCandidate));
        assert_eq!(genes.len(), 1);
    }

    #[test]
    fn counters_record_each_outcome_and_operator_and_round_trip() {
        let mut counts = StructuralMutationCounts::default();
        for operator in OPERATORS {
            for outcome in OUTCOMES {
                let event = StructuralMutationEvent { operator, outcome };
                counts.record(event);
                assert_eq!(
                    serde_json::from_str::<StructuralMutationEvent>(
                        &serde_json::to_string(&event).unwrap()
                    )
                    .unwrap(),
                    event
                );
            }
        }
        let expected = OperatorCounts {
            attempted: OUTCOMES.len() as u64,
            applied: 1,
            no_candidate: 1,
            genome_limit: 1,
            scratch_limit: 1,
            innovation_exhausted: 1,
        };
        assert_eq!(
            counts,
            StructuralMutationCounts {
                remove_connection: expected,
                remove_neuron: expected,
                toggle_connection: expected,
                add_connection: expected,
                add_neuron: expected,
                add_oscillator: Some(expected),
                ..StructuralMutationCounts::default()
            }
        );
        assert_eq!(
            serde_json::from_str::<StructuralMutationCounts>(
                &serde_json::to_string(&counts).unwrap()
            )
            .unwrap(),
            counts
        );
    }

    #[test]
    fn neural_observer_counters_saturate() {
        let almost = OperatorCounts {
            attempted: u64::MAX - 1,
            applied: u64::MAX - 1,
            no_candidate: u64::MAX - 1,
            genome_limit: u64::MAX - 1,
            scratch_limit: u64::MAX - 1,
            innovation_exhausted: u64::MAX - 1,
        };
        let mut counts = StructuralMutationCounts {
            remove_connection: almost,
            remove_neuron: almost,
            toggle_connection: almost,
            add_connection: almost,
            add_neuron: almost,
            add_oscillator: Some(almost),
            ..StructuralMutationCounts::default()
        };
        for _ in 0..2 {
            for operator in OPERATORS {
                for outcome in OUTCOMES {
                    counts.record(StructuralMutationEvent { operator, outcome });
                }
            }
        }
        let saturated = OperatorCounts {
            attempted: u64::MAX,
            applied: u64::MAX,
            no_candidate: u64::MAX,
            genome_limit: u64::MAX,
            scratch_limit: u64::MAX,
            innovation_exhausted: u64::MAX,
        };
        assert_eq!(
            counts,
            StructuralMutationCounts {
                remove_connection: saturated,
                remove_neuron: saturated,
                toggle_connection: saturated,
                add_connection: saturated,
                add_neuron: saturated,
                add_oscillator: Some(saturated),
                ..StructuralMutationCounts::default()
            }
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn long_edit_sequences_preserve_architecture_limits_source_and_determinism(
            seed in any::<u64>(),
            rates in prop::array::uniform5(0u8..=100),
        ) {
            let mut params = SimParams::default();
            params.storage.max_genes = 128;
            params.storage.max_neurons = 32;
            params.storage.max_connections = 96;
            for (operator, rate) in OPERATORS.into_iter().zip(rates) {
                set_rate(&mut params, operator, f32::from(rate) / 100.0);
            }
            let source = bound_genome(32);
            let original_source = source.clone();
            let mut a = reserved(source.iter().copied(), params.storage.max_genes as usize);
            let mut b = reserved(source.iter().copied(), params.storage.max_genes as usize);
            let mut rng_a = Rng::from_seed(seed);
            let mut rng_b = rng_a.clone();
            let mut next_a = 17;
            let mut next_b = 17;
            let mut scratch_a = vec![0; params.storage.max_neurons as usize];
            let mut scratch_b = scratch_a.clone();
            let pointer_a = a.as_ptr();
            let pointer_b = b.as_ptr();
            for _ in 0..512 {
                let previous_next = next_a;
                let mut counts_a = StructuralMutationCounts::default();
                let mut counts_b = StructuralMutationCounts::default();
                apply(
                    &mut a,
                    &params,
                    &mut MutationState {
                        rng: &mut rng_a,
                        next_innovation: &mut next_a,
                        neuron_scratch: &mut scratch_a,
                    },
                    |event| counts_a.record(event),
                );
                apply(
                    &mut b,
                    &params,
                    &mut MutationState {
                        rng: &mut rng_b,
                        next_innovation: &mut next_b,
                        neuron_scratch: &mut scratch_b,
                    },
                    |event| counts_b.record(event),
                );
                prop_assert_eq!(genome::validate_architecture(&a), Ok(()));
                prop_assert_eq!(&a, &b);
                prop_assert_eq!(&rng_a, &rng_b);
                prop_assert_eq!(next_a, next_b);
                prop_assert_eq!(counts_a, counts_b);
                prop_assert!(next_a >= previous_next);
                prop_assert!(a.iter().filter_map(Gene::innovation).all(|id| id.raw() < next_a));
                prop_assert!(a.len() <= params.storage.max_genes as usize);
                prop_assert!(genome::neuron_count(&a) <= params.storage.max_neurons as usize);
                let connections = a.iter().filter(|gene| matches!(gene, Gene::Connection(_))).count();
                prop_assert!(connections <= params.storage.max_connections as usize);
                prop_assert_eq!(a.capacity(), params.storage.max_genes as usize);
                prop_assert_eq!(b.capacity(), params.storage.max_genes as usize);
                prop_assert_eq!(a.as_ptr(), pointer_a);
                prop_assert_eq!(b.as_ptr(), pointer_b);
                for gene in &source {
                    if matches!(gene, Gene::Sensor(_) | Gene::Effector(_) | Gene::Body(_) | Gene::Meta(_))
                        || matches!(gene, Gene::Neuron(n) if n.id.raw() < 8)
                    {
                        prop_assert!(a.contains(gene), "protected gene changed: {:?}", gene);
                    }
                }
            }
            prop_assert_eq!(source, original_source);
        }
    }
}
