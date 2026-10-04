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
const BODY_TRAITS: [BodyTrait; 6] = [
    BodyTrait::Size,
    BodyTrait::SignatureR,
    BodyTrait::SignatureG,
    BodyTrait::SignatureB,
    BodyTrait::Muscle,
    BodyTrait::Mouth,
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
    /// The output neuron a founder's bite reads, when founders carry one. Its bias
    /// starts at `combat.dormant_bias` rather than a draw (spec §4.2).
    bite: Option<InnovationId>,
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
        // A founder's bite reads one more output neuron, which no founder wiring reaches.
        let outputs = (EFFECTORS.len() as u64).checked_add(u64::from(params.founder.bite))?;
        let neurons = sources.checked_add(outputs)?;
        let fan_in = params
            .brain
            .connections_per_target
            .map_or(sources, |count| u64::from(count).min(sources));
        let connections = fan_in.checked_mul(sinks)?;
        let other_genes = sensors
            .checked_add(outputs)?
            .checked_add((BODY_TRAITS.len() + META_TRAITS.len()) as u64)?;
        let genes = neurons.checked_add(connections)?.checked_add(other_genes)?;
        Some(FounderCounts {
            sensor_channels: u32::try_from(sensor_channels).ok()?,
            neurons: u32::try_from(neurons).ok()?,
            genes: u32::try_from(genes).ok()?,
            sensors: u32::try_from(sensors).ok()?,
            synapses: u32::try_from(connections).ok()?,
            effectors: u32::try_from(outputs).ok()?,
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
        let outputs = counts.effectors as usize;
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

        // Effectors read the output neurons that follow the inputs, the bite last.
        let bite = (outputs > EFFECTORS.len()).then_some(Action::Bite);
        for (i, action) in EFFECTORS.into_iter().chain(bite).enumerate() {
            genes.push(Gene::Effector(EffectorGene {
                id: next_id(),
                action,
                // V1 uses yaw, but the gene must retain its turn axis (spec §9.1). A
                // bite's reach is live, so like a sensor's range it is written by
                // `instantiate`: the template a restore rebuilds from retuned params
                // must be the one this world was built with.
                params: if action == Action::Turn {
                    [0.0, 0.0, 1.0, 0.0]
                } else {
                    [0.0; 4]
                },
                source: neuron_ids[sensor_channels + i],
            }));
        }

        // Inputs, hidden neurons and oscillators can source connections; only outputs
        // and hidden neurons receive them in the founding template (spec §3.3). The
        // bite's neuron receives none: a founder's bite starts dormant (spec §4.2).
        let input_end = sensor_channels;
        let output_end = input_end + outputs;
        let hidden_range = output_end..output_end + hidden;
        let oscillator_range = output_end + hidden..neurons;

        let mut sources: Vec<usize> = (0..input_end)
            .chain(hidden_range.clone())
            .chain(oscillator_range)
            .collect();
        let sinks: Vec<usize> = (input_end..input_end + EFFECTORS.len())
            .chain(hidden_range)
            .collect();

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
            bite: bite.map(|_| neuron_ids[output_end - 1]),
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

    /// `founding` is a new founder rather than a control's redraw: it also initializes
    /// sensors, body, meta genes, and the bite, and starts the bite's neuron dormant.
    fn randomize_scalars(
        &self,
        rng: &mut Rng,
        params: &SimParams,
        out: &mut [Gene],
        founding: bool,
        fan_in: Option<&[u32]>,
    ) {
        let brain = &params.brain;
        let count = genome::neuron_count(out);
        let (neurons, rest) = out.split_at_mut(count);
        for gene in neurons.iter_mut() {
            if let Gene::Neuron(n) = gene {
                // Keyed off the template's id, never a value, so draws stay in step. The
                // random control redraws it with every other bias, as it redraws every
                // neural scalar (spec §7.8).
                n.bias = if founding && Some(n.id) == self.bite {
                    params.combat.dormant_bias
                } else {
                    rng.range(-1.0, 1.0)
                };
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
                    if !founding {
                        continue;
                    }
                    s.params = sensor_parameters(s.modality, &params.sensing, rng);
                }
                Gene::Body(b) => {
                    if !founding {
                        continue;
                    }
                    b.value = match b.trait_ {
                        BodyTrait::Size => params.body.size,
                        // Founders all start at the reference body, with no draw, so the
                        // random sequence is the one Phase 2 founders drew (spec §3.5).
                        BodyTrait::Muscle | BodyTrait::Mouth => 1.0,
                        // Founders differ in colour so lineages are distinguishable on
                        // screen from the first frame, and so `vision_ray` has
                        // something to discriminate before `set_signature` exists.
                        _ => rng.range(0.15, 1.0),
                    };
                }
                Gene::Meta(m) => {
                    if !founding {
                        continue;
                    }
                    m.value = match m.trait_ {
                        MetaTrait::MutationRate => params.mutation.weight_perturb_rate,
                        MetaTrait::WeightSigma => params.mutation.weight_perturb_sigma,
                        MetaTrait::CrossoverRate => 0.0,
                    }
                }
                Gene::Effector(e) => {
                    // Live, as a sensor's range is: a retuned reach reaches later
                    // founders, not the living.
                    if founding && e.action == Action::Bite {
                        e.params = [0.0, 0.0, params.combat.reach, 0.0];
                    }
                }
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
            // Dense wiring, so the arena and connection-limit cases are exceeded.
            let mut params = SimParams::default();
            params.brain.hidden_neurons = 6;
            params.brain.connections_per_target = None;
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
                for bite in [false, true] {
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
                    params.founder.bite = bite;
                    assert_counts_and_wiring(&params, 42);
                }
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
        let bite = params.founder.bite.then_some(Action::Bite);
        assert_eq!(
            effectors,
            EFFECTORS.into_iter().chain(bite).collect::<Vec<_>>()
        );
        assert_eq!(counts.synapses as usize, pairs.len());
        pairs.sort();
        pairs.dedup();
        assert_eq!(counts.synapses as usize, pairs.len(), "duplicate edge");

        // Wired sinks: the four standing outputs and the hidden layer. A founder's bite
        // reads the last output, which receives nothing (spec §4.2).
        let inputs = channels.len();
        let output_end = inputs + effectors.len();
        let hidden_end = output_end + params.brain.hidden_neurons as usize;
        let sinks = |slot: usize| {
            (inputs..inputs + EFFECTORS.len()).contains(&slot)
                || (output_end..hidden_end).contains(&slot)
        };
        let sources = plan.neuron_count() - effectors.len();
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
                if sinks(slot) { fan_in } else { 0 },
                "wrong fan-in at neuron {slot}"
            );
        }
        for (from, to) in pairs {
            let source = genome::neuron_index(&genes, from).unwrap();
            let target = genome::neuron_index(&genes, to).unwrap();
            assert!(source < inputs || source >= output_end);
            assert!(sinks(target));
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
        // 23 through Phase 2; Phase 3 adds the muscle and mouth genes (spec §3.5).
        assert_eq!(plan.len(), 25);
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
        assert_eq!(
            sensors.iter().filter(|m| **m == Modality::Chemo).count(),
            params.sensing.chemo_sensors as usize
        );
        assert_eq!(
            sensors
                .iter()
                .filter(|m| **m == Modality::Interoception)
                .count(),
            params.sensing.energy_sensors as usize
        );
        // The shipped founder is chemo-led (spec §3.1); every action is still wired.
        assert_eq!(sensors, [Modality::Chemo]);

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
    fn a_founders_bite_starts_dormant_with_the_live_reach() {
        // The bite reads its own neuron, biased below the gate and wired to nothing,
        // so no founder swings until mutation finds it (spec §4.2).
        let mut params = SimParams::default();
        params.founder.bite = true;
        let plan = plan(&params);
        params.combat.reach = 6.0;
        let genes = instantiate(&plan, &params, 9);
        let bite = genes
            .iter()
            .find_map(|gene| match gene {
                Gene::Effector(effector) if effector.action == Action::Bite => Some(*effector),
                _ => None,
            })
            .expect("a bite");
        assert_eq!(bite.params, [0.0, 0.0, 6.0, 0.0], "built at the live reach");
        let neuron = |genes: &[Gene]| {
            genes
                .iter()
                .find_map(|gene| match gene {
                    Gene::Neuron(neuron) if neuron.id == bite.source => Some(*neuron),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(neuron(&genes).bias, params.combat.dormant_bias);
        assert_eq!(neuron(&genes).activation, Activation::Sigmoid);
        assert!(
            genes.iter().all(|gene| !matches!(gene,
                Gene::Connection(c) if c.from == bite.source || c.to == bite.source)),
            "a founder wired its bite"
        );
        // The random control redraws every neural scalar, the bite's bias included.
        let mut redrawn = genes.clone();
        let mut fan_in = vec![0; genes.len()];
        plan.randomize_brain(&mut Rng::from_seed(3), &params, &mut redrawn, &mut fan_in);
        assert_ne!(neuron(&redrawn).bias, params.combat.dormant_bias);
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
        for t in [BodyTrait::Muscle, BodyTrait::Mouth] {
            assert_eq!(
                crate::genome::body_trait(&genes, t),
                Some(1.0),
                "founders start at the reference body"
            );
        }
    }

    #[test]
    fn reference_traits_draw_nothing() {
        // The muscle and mouth genes must not consume founder draws, or every Phase 2
        // seed would produce different founders (spec §3.5). Randomizing the template
        // without them must leave every other gene, and the stream, where the full
        // template leaves them.
        let params = SimParams::default();
        let plan = plan(&params);
        let mut full = plan.genes().to_vec();
        let mut full_rng = Rng::from_seed(4);
        plan.randomize_scalars(&mut full_rng, &params, &mut full, true, None);

        let reference = |g: &Gene| matches!(g, Gene::Body(b) if matches!(b.trait_, BodyTrait::Muscle | BodyTrait::Mouth));
        let mut without: Vec<Gene> = plan
            .genes()
            .iter()
            .copied()
            .filter(|g| !reference(g))
            .collect();
        assert_eq!(without.len() + 2, plan.len());
        let mut without_rng = Rng::from_seed(4);
        plan.randomize_scalars(&mut without_rng, &params, &mut without, true, None);

        let kept: Vec<Gene> = full.iter().copied().filter(|g| !reference(g)).collect();
        assert_eq!(kept, without);
        assert_eq!(full_rng.unit(), without_rng.unit(), "the streams diverged");
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
        // 2 eyes x 4 channels + chemo 3 = 11 inputs, 4 outputs, 3 hidden, 2 clocks.
        assert_eq!(p.neuron_count(), 11 + 4 + 3 + 2);
    }
}
