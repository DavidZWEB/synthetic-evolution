//! Configurable founding genomes: one shared topology, instantiated with random weights.
//!
//! Sensor and neuron counts and optional sparse wiring define the world's template.
//! Shipped defaults retain Phase 1's dense topology and random-draw sequence (spec §3.3).
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
use crate::params::{ParamError, SensingParams, SimParams};
use crate::rng::Rng;

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

/// Initializes an organ; mutation selection and refusal policy live elsewhere.
pub(crate) fn sensor_parameters(
    modality: Modality,
    params: &SensingParams,
    rng: &mut Rng,
) -> [f32; 4] {
    match modality {
        // Retain the elevation slot, clamped on the simulation plane (spec §4.1, §9.1).
        Modality::VisionRay => [
            rng.range(-core::f32::consts::PI, core::f32::consts::PI),
            0.0,
            params.vision_range,
            params.vision_fov,
        ],
        Modality::Chemo => [0.0, params.chemo_radius, 0.0, 0.0],
        Modality::Interoception => [0.0; 4],
    }
}

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
        let chemo = u64::from(params.sensing.chemo_sensors);
        let energy = u64::from(params.sensing.energy_sensors);
        let sensors = rays.checked_add(chemo)?.checked_add(energy)?;
        let sensor_channels = rays
            .checked_mul(Modality::VisionRay.channels() as u64)?
            .checked_add(chemo.checked_mul(Modality::Chemo.channels() as u64)?)?
            .checked_add(energy.checked_mul(Modality::Interoception.channels() as u64)?)?;
        let sources = sensor_channels
            .checked_add(u64::from(params.brain.hidden_neurons))?
            .checked_add(u64::from(params.brain.oscillators))?;
        let sinks = (EFFECTORS.len() as u64).checked_add(u64::from(params.brain.hidden_neurons))?;
        let neurons = sources.checked_add(EFFECTORS.len() as u64)?;
        let fan_in = params
            .brain
            .connections_per_target
            .map_or(sources, |count| u64::from(count).min(sources));
        let connections = fan_in.checked_mul(sinks)?;
        let other_genes = sensors
            .checked_add((EFFECTORS.len() + BODY_TRAITS.len() + META_TRAITS.len()) as u64)?;
        let genes = neurons.checked_add(connections)?.checked_add(other_genes)?;
        Some(FounderCounts {
            sensor_channels: u32::try_from(sensor_channels).ok()?,
            neurons: u32::try_from(neurons).ok()?,
            genes: u32::try_from(genes).ok()?,
            sensors: u32::try_from(sensors).ok()?,
            synapses: u32::try_from(connections).ok()?,
            effectors: EFFECTORS.len() as u32,
        })
    }

    /// Chooses one world's founding topology and issues its innovation IDs.
    ///
    /// `next_id` is the world's innovation counter — a closure rather than a `&mut
    /// World`, so this is testable without one and cannot reach anything else.
    /// The caller seeds plants first, using this same world RNG (spec §3.3).
    /// Dense topology consumes no draws. Invalid params are rejected before
    /// allocating a plan, drawing randomness, or requesting any IDs.
    pub fn new(
        params: &SimParams,
        rng: &mut Rng,
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

        // Inputs, hidden neurons and oscillators can source connections; only outputs
        // and hidden neurons receive them in the founding template (spec §3.3).
        let input_end = sensor_channels;
        let output_end = input_end + EFFECTORS.len();
        let hidden_range = output_end..output_end + hidden;
        let oscillator_range = output_end + hidden..neurons;

        let mut sources: Vec<usize> = (0..input_end)
            .chain(hidden_range.clone())
            .chain(oscillator_range)
            .collect();
        let sinks: Vec<usize> = (input_end..output_end).chain(hidden_range).collect();

        let fan_in = params
            .brain
            .connections_per_target
            .map_or(sources.len(), |count| (count as usize).min(sources.len()));
        if fan_in == sources.len() {
            // Keep source-major IDs and consume no topology RNG for dense requests,
            // including explicitly saturated fan-in, preserving prior runs (spec §3.3).
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
        } else {
            for &to in &sinks {
                // A partial shuffle samples without replacement. Reuse its permutation
                // for the next target so sparse selection needs no additional buffer.
                for index in 0..fan_in {
                    let selected = index + rng.below((sources.len() - index) as u32) as usize;
                    sources.swap(index, selected);
                    genes.push(Gene::Connection(ConnectionGene {
                        id: next_id(),
                        from: neuron_ids[sources[index]],
                        to: neuron_ids[to],
                        weight: 0.0,
                        enabled: true,
                    }));
                }
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

    /// Preserve the original vision/chemo/energy gene and scalar-draw order.
    fn sensor_layout(params: &SimParams) -> impl Iterator<Item = Modality> {
        core::iter::repeat_n(Modality::VisionRay, params.sensing.vision_rays as usize)
            .chain(core::iter::repeat_n(
                Modality::Chemo,
                params.sensing.chemo_sensors as usize,
            ))
            .chain(core::iter::repeat_n(
                Modality::Interoception,
                params.sensing.energy_sensors as usize,
            ))
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
                    s.params = sensor_parameters(s.modality, &params.sensing, rng);
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
        plan_with_rng(params, &mut Rng::from_seed(0))
    }

    fn plan_with_rng(params: &SimParams, rng: &mut Rng) -> FounderPlan {
        let mut next = 0u32;
        FounderPlan::new(params, rng, || {
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
    fn invalid_params_are_rejected_before_issuing_innovations_or_drawing_randomness() {
        type InvalidParams = (&'static str, fn(&mut SimParams));
        let cases: [InvalidParams; 8] = [
            ("unrepresentable topology", |p| {
                p.brain.hidden_neurons = u32::MAX
            }),
            ("oversized arena", |p| p.brain.hidden_neurons = 200),
            ("invalid timestep", |p| p.world.dt = 0.0),
            ("unrepresentable chemo channels", |p| {
                p.sensing.chemo_sensors = u32::MAX;
                p.brain.connections_per_target = Some(0);
            }),
            ("unrepresentable energy inputs", |p| {
                p.sensing.energy_sensors = u32::MAX;
                p.brain.connections_per_target = Some(0);
            }),
            ("too many sensors", |p| p.sensing.chemo_sensors = 33),
            ("sparse connections exceed limit", |p| {
                p.brain.connections_per_target = Some(1);
                p.storage.max_connections = 9;
            }),
            ("sparse plan exceeds budget", |p| {
                p.brain.connections_per_target = Some(1);
                p.storage.max_memory_bytes = 1;
            }),
        ];
        for (name, invalidate) in cases {
            // Dense, so the arena and connection-limit cases are actually exceeded.
            let mut params = SimParams::default().with_dense_founder();
            invalidate(&mut params);
            let mut requested = 0;
            let mut rng = Rng::from_seed(42);
            let before = rng.clone();
            let result = FounderPlan::new(&params, &mut rng, || {
                let id = InnovationId::new(requested);
                requested += 1;
                id
            });
            assert!(result.is_err(), "{name} was accepted");
            assert_eq!(requested, 0, "{name} consumed innovation ids");
            assert_eq!(rng, before, "{name} consumed RNG");
        }
    }

    #[test]
    fn checked_counts_match_constructed_topologies() {
        for (rays, chemo, energy, hidden, oscillators) in [
            (0, 0, 0, 0, 0),
            (0, 1, 0, 0, 0),
            (0, 0, 2, 0, 0),
            (0, 0, 0, 3, 2),
            (0, 1, 1, 0, 0),
            (1, 1, 1, 3, 0),
            (3, 1, 1, 6, 2),
            (2, 3, 2, 2, 1),
            (12, 1, 1, 32, 4),
        ] {
            for fan_in in [None, Some(0), Some(1), Some(2), Some(u32::MAX)] {
                let mut params = SimParams::default();
                params.species.capacity = 0;
                params.storage.max_genes = 4_096;
                params.storage.max_connections = 4_096;
                params.sensing.vision_rays = rays;
                params.sensing.chemo_sensors = chemo;
                params.sensing.energy_sensors = energy;
                params.brain.hidden_neurons = hidden;
                params.brain.oscillators = oscillators;
                params.brain.connections_per_target = fan_in;
                assert_counts_and_wiring(&params, 42);
            }
        }
    }

    fn assert_counts_and_wiring(params: &SimParams, seed: u64) {
        let counts = FounderPlan::checked_counts(params).expect("counts fit");
        let plan = plan_with_rng(params, &mut Rng::from_seed(seed));
        let genes = instantiate(&plan, params, seed);
        assert_eq!(genome::validate_architecture(&genes), Ok(()));
        assert_eq!(counts.genes as usize, plan.len());
        assert_eq!(counts.neurons as usize, plan.neuron_count());

        let mut channels = Vec::new();
        let mut sensors = Vec::new();
        let mut effectors = Vec::new();
        let mut pairs = Vec::new();
        for gene in &genes {
            match gene {
                Gene::Sensor(sensor) => {
                    sensors.push(sensor.modality);
                    channels.extend_from_slice(&sensor.targets[..sensor.modality.channels()]);
                    assert!(
                        sensor.targets[sensor.modality.channels()..]
                            .iter()
                            .all(|target| target.is_null())
                    );
                }
                Gene::Effector(effector) => effectors.push(effector.action),
                Gene::Connection(connection) => {
                    assert!(connection.enabled);
                    pairs.push((connection.from, connection.to));
                }
                _ => {}
            }
        }
        assert_eq!(counts.sensor_channels as usize, channels.len());
        channels.sort();
        channels.dedup();
        assert_eq!(counts.sensor_channels as usize, channels.len());
        assert_eq!(counts.sensors as usize, sensors.len());
        assert_eq!(
            sensors,
            [
                vec![Modality::VisionRay; params.sensing.vision_rays as usize],
                vec![Modality::Chemo; params.sensing.chemo_sensors as usize],
                vec![Modality::Interoception; params.sensing.energy_sensors as usize],
            ]
            .concat()
        );
        assert_eq!(counts.effectors as usize, effectors.len());
        assert_eq!(effectors, EFFECTORS);
        assert_eq!(counts.synapses as usize, pairs.len());
        pairs.sort();
        pairs.dedup();
        assert_eq!(counts.synapses as usize, pairs.len(), "duplicate edge");

        let inputs = channels.len();
        let output_end = inputs + EFFECTORS.len();
        let hidden_end = output_end + params.brain.hidden_neurons as usize;
        let sources = plan.neuron_count() - EFFECTORS.len();
        let fan_in = params
            .brain
            .connections_per_target
            .map_or(sources, |count| (count as usize).min(sources));
        for (slot, neuron) in genes[..plan.neuron_count()].iter().enumerate() {
            let incoming = pairs
                .iter()
                .filter(|(_, to)| Some(*to) == neuron.innovation())
                .count();
            assert_eq!(
                incoming,
                if (inputs..hidden_end).contains(&slot) {
                    fan_in
                } else {
                    0
                },
                "wrong fan-in at neuron {slot}"
            );
        }
        for (from, to) in pairs {
            let source = genome::neuron_index(&genes, from).unwrap();
            let target = genome::neuron_index(&genes, to).unwrap();
            assert!(source < inputs || source >= output_end);
            assert!((inputs..hidden_end).contains(&target));
        }
        assert_eq!(plan.fan_in_scale.len(), counts.synapses as usize);
        assert!(
            plan.fan_in_scale
                .iter()
                .all(|&scale| scale == 1.0 / math::sqrt(fan_in as f32))
        );
    }

    proptest::proptest! {
        #[test]
        fn varied_founder_topologies_have_exact_distinct_fan_in(
            rays in 0u32..6,
            chemo in 0u32..4,
            energy in 0u32..4,
            hidden in 0u32..8,
            oscillators in 0u32..4,
            fan_in in proptest::option::of(0u32..50),
            seed in proptest::prelude::any::<u64>(),
        ) {
            let mut params = SimParams::default();
            params.sensing.vision_rays = rays;
            params.sensing.chemo_sensors = chemo;
            params.sensing.energy_sensors = energy;
            params.brain.hidden_neurons = hidden;
            params.brain.oscillators = oscillators;
            params.brain.connections_per_target = fan_in;
            assert_counts_and_wiring(&params, seed);
        }
    }

    #[test]
    fn full_connectivity_is_dense_with_identical_ids_scalars_and_rng() {
        let params = SimParams::default().with_dense_founder();
        let sources =
            FounderPlan::checked_counts(&params).unwrap().neurons - EFFECTORS.len() as u32;
        for seed in [0, 1, 42] {
            let mut dense_rng = Rng::from_seed(seed);
            let before = dense_rng.clone();
            let dense = plan_with_rng(&params, &mut dense_rng);
            assert_eq!(dense_rng, before, "dense construction consumed RNG");
            let mut dense_genes = vec![Gene::default(); dense.len()];
            dense.instantiate(&mut dense_rng, &params, &mut dense_genes);
            for fan_in in [sources, sources + 1, u32::MAX] {
                let mut explicit = params.clone();
                explicit.brain.connections_per_target = Some(fan_in);
                let mut rng = Rng::from_seed(seed);
                let full = plan_with_rng(&explicit, &mut rng);
                assert_eq!(rng, before, "full construction consumed RNG");
                assert_eq!(full.genes(), dense.genes());
                assert_eq!(full.fan_in_scale, dense.fan_in_scale);
                let mut genes = vec![Gene::default(); full.len()];
                full.instantiate(&mut rng, &explicit, &mut genes);
                assert_eq!(genes, dense_genes);
                assert_eq!(rng, dense_rng);
            }
        }
    }

    #[test]
    fn default_dense_connections_keep_source_major_innovation_order() {
        let params = SimParams::default().with_dense_founder();
        let plan = plan(&params);
        assert_eq!(plan.len(), 284);
        assert_eq!(plan.neuron_count(), 28);
        let expected: Vec<_> = (0..16)
            .chain(20..28)
            .flat_map(|from| (16..26).map(move |to| (from, to)))
            .enumerate()
            .map(|(offset, (from, to))| (37 + offset as u32, from, to))
            .collect();
        let actual: Vec<_> = plan
            .genes()
            .iter()
            .filter_map(|gene| match gene {
                Gene::Connection(connection) => Some((
                    connection.id.raw(),
                    connection.from.raw(),
                    connection.to.raw(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn sparse_template_is_deterministic_and_shared_across_founders() {
        let mut params = SimParams::default();
        params.brain.connections_per_target = Some(2);
        let mut rng = Rng::from_seed(42);
        let before = rng.clone();
        let first = plan_with_rng(&params, &mut rng);
        let mut repeated_rng = Rng::from_seed(42);
        let repeated = plan_with_rng(&params, &mut repeated_rng);
        assert_ne!(
            rng, before,
            "sparse wiring must sample its source membership"
        );
        assert_eq!(rng, repeated_rng);
        assert_eq!(first.genes(), repeated.genes());
        assert_eq!(first.fan_in_scale, repeated.fan_in_scale);
        assert_ne!(first.genes(), plan(&params).genes());

        let mut a = vec![Gene::default(); first.len()];
        let mut b = a.clone();
        first.instantiate(&mut rng, &params, &mut a);
        first.instantiate(&mut rng, &params, &mut b);
        assert_ne!(a, b);
        let topology = |genes: &[Gene]| {
            genes
                .iter()
                .filter_map(|gene| match gene {
                    Gene::Connection(c) => Some((c.id, c.from, c.to)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(topology(&a), topology(first.genes()));
        assert_eq!(topology(&b), topology(first.genes()));
    }

    #[test]
    fn zero_connectivity_and_empty_source_sets_consume_no_topology_rng() {
        for sensors in [false, true] {
            for fan_in in [None, Some(0), Some(1), Some(u32::MAX)] {
                if sensors && fan_in != Some(0) {
                    continue;
                }
                let mut params = SimParams::default();
                if !sensors {
                    params.sensing.vision_rays = 0;
                    params.sensing.chemo_sensors = 0;
                    params.sensing.energy_sensors = 0;
                    params.brain.hidden_neurons = 0;
                    params.brain.oscillators = 0;
                }
                params.brain.connections_per_target = fan_in;
                let mut rng = Rng::from_seed(42);
                let before = rng.clone();
                let plan = plan_with_rng(&params, &mut rng);
                assert_eq!(rng, before);
                assert!(plan.fan_in_scale.is_empty());
                assert_eq!(crate::brain::synapse_count(plan.genes()), 0);
                assert_eq!(validate(&instantiate(&plan, &params, 7)), Ok(()));
            }
        }
    }

    #[test]
    fn chemo_led_minimal_candidate_retains_effectors_body_and_compatibility_fields() {
        let mut params = SimParams::default();
        params.sensing.vision_rays = 0;
        params.sensing.energy_sensors = 0;
        params.brain.hidden_neurons = 0;
        params.brain.oscillators = 0;
        params.brain.connections_per_target = Some(1);
        let plan = plan(&params);
        assert_eq!(plan.neuron_count(), 7);
        assert_eq!(plan.len(), 23);
        assert_counts_and_wiring(&params, 42);
        let genes = instantiate(&plan, &params, 42);
        for trait_ in BODY_TRAITS {
            assert!(
                genes
                    .iter()
                    .any(|gene| matches!(gene, Gene::Body(b) if b.trait_ == trait_))
            );
        }
        for trait_ in META_TRAITS {
            assert!(
                genes
                    .iter()
                    .any(|gene| matches!(gene, Gene::Meta(m) if m.trait_ == trait_))
            );
        }
        for gene in genes {
            match gene {
                Gene::Sensor(sensor) => {
                    assert_eq!(sensor.modality, Modality::Chemo);
                    assert_eq!(sensor.params, [0.0, params.sensing.chemo_radius, 0.0, 0.0]);
                    assert_eq!(sensor.targets[3], InnovationId::NULL);
                }
                Gene::Effector(e) if e.action == Action::Turn => {
                    assert_eq!(e.params, [0.0, 0.0, 1.0, 0.0]);
                }
                Gene::Meta(m) if m.trait_ == MetaTrait::CrossoverRate => {
                    assert_eq!(m.value, 0.0);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn sensor_initialization_uses_live_parameters_and_only_vision_draws() {
        let params = SensingParams {
            vision_range: 7.0,
            vision_fov: 0.75,
            chemo_radius: 3.0,
            ..SensingParams::default()
        };
        let mut rng = Rng::from_seed(42);
        let mut expected = rng.clone();
        assert_eq!(
            sensor_parameters(Modality::VisionRay, &params, &mut rng),
            [
                expected.range(-core::f32::consts::PI, core::f32::consts::PI),
                0.0,
                7.0,
                0.75,
            ]
        );
        assert_eq!(
            sensor_parameters(Modality::Chemo, &params, &mut rng),
            [0.0, 3.0, 0.0, 0.0]
        );
        assert_eq!(
            sensor_parameters(Modality::Interoception, &params, &mut rng),
            [0.0; 4]
        );
        assert_eq!(rng, expected);
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
    fn the_shipped_sensor_and_effector_set_is_present() {
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
        let params = SimParams::default().with_dense_founder();
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
        let mut params = SimParams::default().with_dense_founder();
        params.brain.hidden_neurons = 3;
        params.brain.oscillators = 2;
        params.sensing.vision_rays = 2;
        let p = plan(&params);
        // 2 eyes x 4 channels + chemo 3 + interoception 1 = 12 inputs, 4 outputs.
        assert_eq!(p.neuron_count(), 12 + 4 + 3 + 2);
    }
}
