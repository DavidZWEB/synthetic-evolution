//! The Phase 1 founding genome: one fixed topology, instantiated with random weights.
//!
//! Phase 1 hardcodes the sensor and effector set and the neuron count. What is *not*
//! hardcoded is the representation — a founder is an ordinary gene list. Phase 2
//! revisits founder composition alongside its structural mutation operators.
//!
//! Every founder in a world shares **one set of innovation ids**, drawn once into a
//! [`FounderPlan`]. That is what makes shared ancestry real: two genomes that match on
//! an id inherited it from the same origin, which is the whole basis of NEAT-style
//! crossover and genetic distance (spec §3.1). Drawing fresh ids per founder would
//! make every agent its own lineage and quietly make crossover meaningless.
//!
//! Deliberately not here: mutation and recombination. This module builds the starting
//! point and nothing else.

use crate::genome::{
    self, Action, Activation, BodyGene, BodyTrait, ConnectionGene, EffectorGene, Gene, MetaGene,
    MetaTrait, Modality, NeuronGene, SENSOR_CHANNELS, SensorGene,
};
use crate::ids::InnovationId;
use crate::math;
use crate::params::{ParamError, SimParams};
use crate::rng::Rng;

const BASE_SENSORS: [Modality; 2] = [Modality::Chemo, Modality::Interoception];
const EFFECTORS: [Action; 4] = [
    Action::Thrust,
    Action::Turn,
    Action::Ingest,
    Action::Reproduce,
];
const BODY_TRAITS: [BodyTrait; 4] = [
    BodyTrait::Size,
    BodyTrait::SignatureR,
    BodyTrait::SignatureG,
    BodyTrait::SignatureB,
];
const META_TRAITS: [MetaTrait; 3] = [
    MetaTrait::MutationRate,
    MetaTrait::WeightSigma,
    MetaTrait::CrossoverRate,
];

pub(crate) struct FounderCounts {
    pub sensor_channels: u32,
    pub neurons: u32,
    pub genes: u32,
    pub sensors: u32,
    pub synapses: u32,
    pub effectors: u32,
}

/// The fixed topology, with its innovation ids assigned once per world.
///
/// Holds the gene list as a template. Instantiating a founder copies it and randomises
/// the scalars, so the structure is shared and the weights are not.
#[derive(Clone, Debug)]
pub struct FounderPlan {
    genes: Vec<Gene>,
    neurons: usize,
    /// `1/√fan_in` for each connection gene, in gene order.
    ///
    /// A property of the topology, which every founder in a world shares, so it is
    /// computed once here — and only the topology half: the scale itself comes from
    /// [`SimParams`] at instantiation, so changing it at runtime takes effect without
    /// rebuilding the plan.
    fan_in_scale: Vec<f32>,
}

impl FounderPlan {
    /// Allocation-free sizing shared by boundary validation and construction.
    pub(crate) fn checked_counts(params: &SimParams) -> Option<FounderCounts> {
        let rays = u64::from(params.sensing.vision_rays);
        let sensor_channels = rays * Modality::VisionRay.channels() as u64
            + BASE_SENSORS
                .iter()
                .map(|m| m.channels() as u64)
                .sum::<u64>();
        let sources = sensor_channels
            + u64::from(params.brain.hidden_neurons)
            + u64::from(params.brain.oscillators);
        let sinks = EFFECTORS.len() as u64 + u64::from(params.brain.hidden_neurons);
        let neurons = sources + EFFECTORS.len() as u64;
        let connections = sources.checked_mul(sinks)?;
        let other_genes = rays
            + (BASE_SENSORS.len() + EFFECTORS.len() + BODY_TRAITS.len() + META_TRAITS.len()) as u64;
        let genes = neurons.checked_add(connections)?.checked_add(other_genes)?;
        Some(FounderCounts {
            sensor_channels: u32::try_from(sensor_channels).ok()?,
            neurons: u32::try_from(neurons).ok()?,
            genes: u32::try_from(genes).ok()?,
            sensors: u32::try_from(rays + BASE_SENSORS.len() as u64).ok()?,
            synapses: u32::try_from(connections).ok()?,
            effectors: EFFECTORS.len() as u32,
        })
    }

    /// Draws the ids for one world's founding topology.
    ///
    /// `next_id` is the world's innovation counter — a closure rather than a `&mut
    /// World`, so this is testable without one and cannot reach anything else.
    /// Invalid params are rejected before allocating a plan or requesting any ids.
    pub fn new(
        params: &SimParams,
        mut next_id: impl FnMut() -> InnovationId,
    ) -> Result<Self, ParamError> {
        params.validate()?;
        let counts = Self::checked_counts(params).ok_or(ParamError(
            "founding topology exceeds representable gene counts",
        ))?;
        let sensor_channels = counts.sensor_channels as usize;
        let neurons = counts.neurons as usize;
        let hidden = params.brain.hidden_neurons as usize;
        let oscillators = params.brain.oscillators as usize;

        let mut genes = Vec::with_capacity(counts.genes as usize);
        let neuron_ids: Vec<InnovationId> = (0..neurons).map(|_| next_id()).collect();

        // Neurons first, so a forward pass sees every neuron before any reference.
        for (slot, &id) in neuron_ids.iter().enumerate() {
            let is_oscillator = slot >= neurons - oscillators;
            genes.push(Gene::Neuron(NeuronGene {
                id,
                bias: 0.0,
                tau: 1.0,
                activation: if is_oscillator {
                    Activation::Oscillator
                } else {
                    Activation::Sigmoid
                },
                period: 0.0,
            }));
        }

        // Sensors bind to the input neurons, one channel at a time.
        let mut channel = 0;
        for modality in Self::sensor_layout(params) {
            let mut targets = [InnovationId::NULL; SENSOR_CHANNELS];
            for target in targets.iter_mut().take(modality.channels()) {
                *target = neuron_ids[channel];
                channel += 1;
            }
            genes.push(Gene::Sensor(SensorGene {
                id: next_id(),
                modality,
                params: [0.0; 4],
                targets,
            }));
        }

        // Effectors read the output neurons that follow the inputs.
        for (i, &action) in EFFECTORS.iter().enumerate() {
            genes.push(Gene::Effector(EffectorGene {
                id: next_id(),
                action,
                // V1 uses yaw, but the gene must retain its turn axis (spec §9.1).
                params: if action == Action::Turn {
                    [0.0, 0.0, 1.0, 0.0]
                } else {
                    [0.0; 4]
                },
                source: neuron_ids[sensor_channels + i],
            }));
        }

        // Every input, hidden neuron and oscillator connects to every output and
        // hidden neuron. Phase 1 has no operator to add a missing connection (spec §3.3).
        let input_end = sensor_channels;
        let output_end = input_end + EFFECTORS.len();
        let hidden_range = output_end..output_end + hidden;
        let oscillator_range = output_end + hidden..neurons;

        let sources: Vec<usize> = (0..input_end)
            .chain(hidden_range.clone())
            .chain(oscillator_range)
            .collect();
        let sinks: Vec<usize> = (input_end..output_end).chain(hidden_range).collect();

        for &from in &sources {
            for &to in &sinks {
                genes.push(Gene::Connection(ConnectionGene {
                    id: next_id(),
                    from: neuron_ids[from],
                    to: neuron_ids[to],
                    weight: 0.0,
                    enabled: true,
                }));
            }
        }

        for trait_ in BODY_TRAITS {
            genes.push(Gene::Body(BodyGene { trait_, value: 0.0 }));
        }

        for trait_ in META_TRAITS {
            genes.push(Gene::Meta(MetaGene { trait_, value: 0.0 }));
        }

        genes.sort_by_key(Gene::sort_key);

        // Fan-in per neuron slot: how many connections land on it. Initial weights are
        // divided by its square root so that a neuron's summed input does not grow with
        // the number of things wired into it — see `BrainParams::weight_init_scale`.
        let mut fan_in = vec![0u32; neurons];
        let mut targets: Vec<usize> = Vec::with_capacity(genes.len());
        for gene in &genes {
            if let Gene::Connection(c) = gene {
                let slot = genome::neuron_index(&genes, c.to).expect("plan wired a real neuron");
                fan_in[slot] += 1;
                targets.push(slot);
            }
        }
        // Every target counted itself above, so no divisor here is zero.
        let fan_in_scale = targets
            .iter()
            .map(|&slot| 1.0 / math::sqrt(fan_in[slot] as f32))
            .collect();

        Ok(Self {
            genes,
            neurons,
            fan_in_scale,
        })
    }

    /// Phase 1's hardcoded sensor set: `vision_rays` eyes, a nose, and one
    /// interoceptor for the agent's own energy (build plan task 5).
    fn sensor_layout(params: &SimParams) -> impl Iterator<Item = Modality> {
        core::iter::repeat_n(Modality::VisionRay, params.sensing.vision_rays as usize)
            .chain(BASE_SENSORS)
    }

    pub fn len(&self) -> usize {
        self.genes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.genes.is_empty()
    }

    pub fn neuron_count(&self) -> usize {
        self.neurons
    }

    /// The template itself, for tests and for sizing the genome arena.
    pub fn genes(&self) -> &[Gene] {
        &self.genes
    }

    /// Writes one founder into `out`, randomising every scalar the plan left at zero.
    ///
    /// Structure comes from the plan and is identical across founders; weights, biases,
    /// taus, oscillator periods, and the signature colour are drawn fresh. Writes into
    /// a caller-owned slice so a birth allocates nothing.
    ///
    /// Draws happen in gene order, and which of them happen is decided entirely by the
    /// plan's template — a gene's *kind*, a neuron's activation, a body gene's trait.
    /// Nothing here may key a draw off a value it has just written, or the RNG stream
    /// would depend on genome state and two founders would desynchronise (spec §7.4).
    pub fn instantiate(&self, rng: &mut Rng, params: &SimParams, out: &mut [Gene]) {
        debug_assert_eq!(out.len(), self.genes.len(), "destination is the wrong size");
        out.copy_from_slice(&self.genes);
        self.randomize_scalars(rng, params, out, true, None);
        debug_assert!(genome::validate(out).is_ok(), "founder is not coherent");
    }

    /// Redraws neural scalars while preserving every inherited non-neural gene.
    ///
    /// Used by the random-brain control at birth. It breaks neural heredity without
    /// changing sensors, body traits, topology, parameters, or the world's economy.
    pub(crate) fn randomize_brain(
        &self,
        rng: &mut Rng,
        params: &SimParams,
        out: &mut [Gene],
        fan_in: &mut [u32],
    ) {
        let neurons = genome::neuron_count(out);
        let fan_in = &mut fan_in[..neurons];
        fan_in.fill(0);
        for gene in out.iter() {
            if let Gene::Connection(connection) = gene
                && connection.enabled
            {
                let target = genome::neuron_index(out, connection.to).expect("validated endpoint");
                fan_in[target] += 1;
            }
        }
        self.randomize_scalars(rng, params, out, false, Some(fan_in));
        debug_assert!(
            genome::validate(out).is_ok(),
            "randomized brain is incoherent"
        );
    }

    fn randomize_scalars(
        &self,
        rng: &mut Rng,
        params: &SimParams,
        out: &mut [Gene],
        include_non_neural: bool,
        fan_in: Option<&[u32]>,
    ) {
        let brain = &params.brain;
        let count = genome::neuron_count(out);
        let (neurons, rest) = out.split_at_mut(count);
        for gene in neurons.iter_mut() {
            if let Gene::Neuron(n) = gene {
                n.bias = rng.range(-1.0, 1.0);
                n.tau = rng.range(brain.tau_min, brain.tau_max);
                if n.activation == Activation::Oscillator {
                    n.period = rng.range(brain.oscillator_period_min, brain.oscillator_period_max);
                }
            }
        }
        let mut connection = 0usize;
        for gene in rest.iter_mut() {
            match gene {
                Gene::Neuron(_) => unreachable!("neurons are the leading gene run"),
                Gene::Connection(c) => {
                    // Scaled by fan-in, not drawn from `weight_limit`: at 24 inputs per
                    // neuron the full bound saturates every sigmoid on tick one and the
                    // brain never responds to a sensor again.
                    let normalized = if let Some(fan_in) = fan_in {
                        let target =
                            genome::neuron_index(neurons, c.to).expect("validated endpoint");
                        1.0 / math::sqrt(fan_in[target].max(1) as f32)
                    } else {
                        self.fan_in_scale[connection]
                    };
                    let scale = brain.weight_init_scale * normalized;
                    connection += 1;
                    c.weight = rng.range(-scale, scale);
                }
                Gene::Sensor(s) => {
                    if !include_non_neural {
                        continue;
                    }
                    if s.modality == Modality::VisionRay {
                        // Azimuth spread around the facing direction; elevation stays
                        // clamped at 0 for all of V1 (spec §4.1, §9.1).
                        s.params = [
                            rng.range(-core::f32::consts::PI, core::f32::consts::PI),
                            0.0,
                            params.sensing.vision_range,
                            params.sensing.vision_fov,
                        ];
                    } else if s.modality == Modality::Chemo {
                        s.params = [0.0, params.sensing.chemo_radius, 0.0, 0.0];
                    }
                }
                Gene::Body(b) => {
                    if !include_non_neural {
                        continue;
                    }
                    b.value = match b.trait_ {
                        BodyTrait::Size => params.body.size,
                        // Founders differ in colour so lineages are distinguishable on
                        // screen from the first frame, and so `vision_ray` has
                        // something to discriminate before `set_signature` exists.
                        _ => rng.range(0.15, 1.0),
                    };
                }
                Gene::Meta(m) => {
                    if !include_non_neural {
                        continue;
                    }
                    m.value = match m.trait_ {
                        MetaTrait::MutationRate => params.mutation.weight_perturb_rate,
                        MetaTrait::WeightSigma => params.mutation.weight_perturb_sigma,
                        MetaTrait::CrossoverRate => 0.0,
                    }
                }
                Gene::Effector(_) => {}
            }
        }
        if fan_in.is_none() {
            debug_assert_eq!(connection, self.fan_in_scale.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{Activation, BodyTrait, Gene, Modality, validate};

    fn plan(params: &SimParams) -> FounderPlan {
        let mut next = 0u32;
        FounderPlan::new(params, || {
            next += 1;
            InnovationId::new(next - 1)
        })
        .expect("valid founder params")
    }

    fn instantiate(p: &FounderPlan, params: &SimParams, seed: u64) -> Vec<Gene> {
        let mut out = vec![Gene::default(); p.len()];
        p.instantiate(&mut Rng::from_seed(seed), params, &mut out);
        out
    }

    #[test]
    fn a_founder_is_coherent() {
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 1);
        assert_eq!(validate(&genes), Ok(()));
    }

    #[test]
    fn invalid_params_are_rejected_before_issuing_innovations() {
        type InvalidParams = (&'static str, fn(&mut SimParams));
        let cases: [InvalidParams; 3] = [
            ("unrepresentable topology", |p| {
                p.brain.hidden_neurons = u32::MAX
            }),
            ("oversized arena", |p| p.brain.hidden_neurons = 200),
            ("invalid timestep", |p| p.world.dt = 0.0),
        ];
        for (name, invalidate) in cases {
            let mut params = SimParams::default();
            invalidate(&mut params);
            let mut requested = 0;
            let result = FounderPlan::new(&params, || {
                let id = InnovationId::new(requested);
                requested += 1;
                id
            });
            assert!(result.is_err(), "{name} was accepted");
            assert_eq!(requested, 0, "{name} consumed innovation ids");
        }
    }

    #[test]
    fn checked_counts_match_constructed_topologies() {
        for (rays, hidden, oscillators) in [(0, 0, 0), (1, 3, 0), (3, 6, 2), (12, 32, 4)] {
            let mut params = SimParams::default();
            params.storage.max_genes = 4_096;
            params.storage.max_connections = 4_096;
            params.sensing.vision_rays = rays;
            params.brain.hidden_neurons = hidden;
            params.brain.oscillators = oscillators;
            params.validate().expect("valid topology");
            let counts = FounderPlan::checked_counts(&params).expect("counts fit");
            let plan = plan(&params);
            assert_eq!(counts.genes as usize, plan.len());
            assert_eq!(counts.neurons as usize, plan.neuron_count());
            let channels: usize = plan
                .genes()
                .iter()
                .filter_map(|gene| match gene {
                    Gene::Sensor(sensor) => Some(sensor.modality.channels()),
                    _ => None,
                })
                .sum();
            assert_eq!(counts.sensor_channels as usize, channels);
        }
    }

    #[test]
    fn a_real_founder_preserves_the_z_turn_axis_through_compilation() {
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 1);
        let turn = genes
            .iter()
            .find_map(|gene| match gene {
                Gene::Effector(e) if e.action == Action::Turn => Some(e),
                _ => None,
            })
            .expect("a turn effector");
        assert_eq!(turn.params[..3], [0.0, 0.0, 1.0]);

        let mut effectors =
            vec![crate::effectors::Effector::default(); crate::effectors::effector_count(&genes)];
        crate::effectors::compile(&genes, &mut effectors);
        let turn = effectors
            .iter()
            .find(|e| e.action == Action::Turn)
            .expect("compiled turn");
        assert_eq!(turn.params[..3], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn innovation_ids_are_unique_and_dense() {
        let params = SimParams::default();
        let p = plan(&params);
        let mut ids: Vec<u32> = p
            .genes()
            .iter()
            .filter_map(|g| g.innovation())
            .map(|i| i.raw())
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count, "an innovation id was issued twice");
        assert_eq!(ids.first(), Some(&0));
        assert_eq!(
            ids.last(),
            Some(&(count as u32 - 1)),
            "ids should be contiguous"
        );
    }

    #[test]
    fn the_hardcoded_sensor_and_effector_set_is_present() {
        let params = SimParams::default();
        let p = plan(&params);
        let sensors: Vec<Modality> = p
            .genes()
            .iter()
            .filter_map(|g| match g {
                Gene::Sensor(s) => Some(s.modality),
                _ => None,
            })
            .collect();
        assert_eq!(
            sensors
                .iter()
                .filter(|m| **m == Modality::VisionRay)
                .count(),
            params.sensing.vision_rays as usize
        );
        assert_eq!(sensors.iter().filter(|m| **m == Modality::Chemo).count(), 1);
        assert_eq!(
            sensors
                .iter()
                .filter(|m| **m == Modality::Interoception)
                .count(),
            1
        );

        let actions: Vec<Action> = p
            .genes()
            .iter()
            .filter_map(|g| match g {
                Gene::Effector(e) => Some(e.action),
                _ => None,
            })
            .collect();
        for want in [
            Action::Thrust,
            Action::Turn,
            Action::Ingest,
            Action::Reproduce,
        ] {
            assert!(
                actions.contains(&want),
                "{want:?} missing from the founding set"
            );
        }
    }

    #[test]
    fn every_sensor_channel_binds_to_a_distinct_neuron() {
        // Two channels sharing an input neuron would silently sum, and the agent would
        // be unable to tell distance from colour.
        let params = SimParams::default();
        let p = plan(&params);
        let mut bound: Vec<u32> = Vec::new();
        for gene in p.genes() {
            if let Gene::Sensor(s) = gene {
                for t in &s.targets[..s.modality.channels()] {
                    assert!(!t.is_null());
                    bound.push(t.raw());
                }
            }
        }
        let count = bound.len();
        bound.sort();
        bound.dedup();
        assert_eq!(
            bound.len(),
            count,
            "two sensor channels share an input neuron"
        );
    }

    #[test]
    fn oscillators_are_present_and_get_a_period() {
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 2);
        let oscillators: Vec<_> = genes
            .iter()
            .filter_map(|g| match g {
                Gene::Neuron(n) if n.activation == Activation::Oscillator => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(oscillators.len(), params.brain.oscillators as usize);
        for n in oscillators {
            assert!(n.period >= params.brain.oscillator_period_min);
            assert!(n.period <= params.brain.oscillator_period_max);
        }
    }

    #[test]
    fn structure_is_shared_and_scalars_are_not() {
        let params = SimParams::default();
        let p = plan(&params);
        let a = instantiate(&p, &params, 10);
        let b = instantiate(&p, &params, 11);
        let keys_a: Vec<_> = a.iter().map(Gene::sort_key).collect();
        let keys_b: Vec<_> = b.iter().map(Gene::sort_key).collect();
        assert_eq!(keys_a, keys_b, "founders must share one topology");
        assert_ne!(a, b, "founders must not be clones");
    }

    #[test]
    fn instantiating_is_deterministic() {
        let params = SimParams::default();
        let p = plan(&params);
        assert_eq!(instantiate(&p, &params, 42), instantiate(&p, &params, 42));
    }

    #[test]
    fn random_brain_control_preserves_non_neural_genes() {
        let params = SimParams::default();
        let plan = plan(&params);
        let mut genes = instantiate(&plan, &params, 42);
        let before = genes.clone();
        let mut fan_in = vec![0; params.storage.max_neurons as usize];
        plan.randomize_brain(&mut Rng::from_seed(99), &params, &mut genes, &mut fan_in);

        let mut neural_change = false;
        for (before, after) in before.iter().zip(&genes) {
            match (before, after) {
                (Gene::Neuron(_), Gene::Neuron(_)) | (Gene::Connection(_), Gene::Connection(_)) => {
                    neural_change |= before != after;
                }
                _ => assert_eq!(before, after, "random control changed a non-neural gene"),
            }
        }
        assert!(neural_change, "random control left the brain unchanged");
    }

    #[test]
    fn elevation_stays_clamped_at_zero() {
        // V1 is a plane. The slot exists in the serialized genome so going 3D is an
        // unclamping rather than a format break (spec §4.1, §9.1).
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 3);
        for gene in &genes {
            if let Gene::Sensor(s) = gene
                && s.modality == Modality::VisionRay
            {
                assert_eq!(s.params[1], 0.0, "elevation varied");
            }
        }
    }

    #[test]
    fn dynamic_control_fan_in_preserves_fixed_topology_values_and_rng() {
        let params = SimParams::default();
        let plan = plan(&params);
        let mut cached = instantiate(&plan, &params, 3);
        let mut dynamic = cached.clone();
        let mut a = Rng::from_seed(77);
        let mut b = Rng::from_seed(77);
        plan.randomize_scalars(&mut a, &params, &mut cached, false, None);
        let mut fan_in = vec![0; params.storage.max_neurons as usize];
        plan.randomize_brain(&mut b, &params, &mut dynamic, &mut fan_in);
        assert_eq!(dynamic, cached);
        assert_eq!(a.state_fingerprint(), b.state_fingerprint());
    }

    #[test]
    fn control_randomizes_a_different_topology_without_changing_non_neural_genes() {
        let params = SimParams::default();
        let plan = plan(&params);
        let mut genes = genome::fixtures::tiny();
        let original = genes.clone();
        let mut fan_in = vec![0; params.storage.max_neurons as usize];
        plan.randomize_brain(&mut Rng::from_seed(11), &params, &mut genes, &mut fan_in);
        assert!(genome::validate(&genes).is_ok());
        assert_ne!(genes, original);
        for (before, after) in original.iter().zip(&genes) {
            if !matches!(before, Gene::Neuron(_) | Gene::Connection(_)) {
                assert_eq!(before, after);
            }
        }
    }

    #[test]
    fn body_traits_are_populated() {
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 4);
        assert_eq!(
            crate::genome::body_trait(&genes, BodyTrait::Size),
            Some(params.body.size)
        );
        for t in [
            BodyTrait::SignatureR,
            BodyTrait::SignatureG,
            BodyTrait::SignatureB,
        ] {
            let v = crate::genome::body_trait(&genes, t).expect("signature channel present");
            assert!((0.0..=1.0).contains(&v), "signature out of range: {v}");
        }
    }

    #[test]
    fn initial_weights_are_scaled_down_by_fan_in() {
        // The bound in `MutationParams::weight_limit` is where evolution may take a
        // weight; it is not where one should start. With 24 inputs per neuron the full
        // bound sums to order ±20 and every sigmoid saturates on tick one.
        let params = SimParams::default();
        let genes = instantiate(&plan(&params), &params, 5);
        let fan_in = 24.0;
        let expected = params.brain.weight_init_scale / crate::math::sqrt(fan_in);

        let weights: Vec<f32> = genes
            .iter()
            .filter_map(|g| match g {
                Gene::Connection(c) => Some(c.weight),
                _ => None,
            })
            .collect();
        assert_eq!(
            weights.len(),
            240,
            "the default topology is fully connected"
        );
        for w in &weights {
            assert!(
                w.abs() <= expected + 1e-6,
                "weight {w} exceeds the fan-in-scaled draw of ±{expected}"
            );
        }
        assert!(
            weights.iter().any(|w| w.abs() > expected * 0.9),
            "the draw is not using its whole range"
        );
    }

    #[test]
    fn the_init_scale_is_a_live_parameter() {
        // Only the topology half is baked into the plan, so a runtime change to
        // `weight_init_scale` takes effect without rebuilding it (spec §7.6).
        let params = SimParams::default();
        let p = plan(&params);
        let mut wider = params.clone();
        wider.brain.weight_init_scale = params.brain.weight_init_scale * 10.0;

        let spread = |ps: &SimParams| {
            instantiate(&p, ps, 6)
                .iter()
                .filter_map(|g| match g {
                    Gene::Connection(c) => Some(c.weight.abs()),
                    _ => None,
                })
                .fold(0.0f32, f32::max)
        };
        assert!(spread(&wider) > spread(&params) * 5.0);
    }

    #[test]
    fn brain_width_tracks_the_parameters() {
        let mut params = SimParams::default();
        params.brain.hidden_neurons = 3;
        params.brain.oscillators = 2;
        params.sensing.vision_rays = 2;
        let p = plan(&params);
        // 2 eyes x 4 channels + chemo 3 + interoception 1 = 12 inputs, 4 outputs.
        assert_eq!(p.neuron_count(), 12 + 4 + 3 + 2);
    }
}
