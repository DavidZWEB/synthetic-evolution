//! The Phase 1 founding genome: one fixed topology, instantiated with random weights.
//!
//! Phase 1 hardcodes the sensor and effector set and the neuron count. What is *not*
//! hardcoded is the representation — a founder is an ordinary gene list, and Phase 2
//! adds structural mutation operators without changing anything here.
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
use crate::params::SimParams;
use crate::rng::Rng;

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
    /// Draws the ids for one world's founding topology.
    ///
    /// `next_id` is the world's innovation counter — a closure rather than a `&mut
    /// World`, so this is testable without one and cannot reach anything else.
    pub fn new(params: &SimParams, mut next_id: impl FnMut() -> InnovationId) -> Self {
        let sensors = Self::sensor_layout(params);
        let sensor_channels: usize = sensors.iter().map(|m| m.channels()).sum();
        let effectors = [
            Action::Thrust,
            Action::Turn,
            Action::Ingest,
            Action::Reproduce,
        ];

        let hidden = params.brain.hidden_neurons as usize;
        let oscillators = params.brain.oscillators as usize;
        // One input neuron per sensor channel, one output neuron per effector.
        let neurons = sensor_channels + effectors.len() + hidden + oscillators;

        let mut genes = Vec::with_capacity(neurons * 2);
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
        for &modality in &sensors {
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
        for (i, &action) in effectors.iter().enumerate() {
            genes.push(Gene::Effector(EffectorGene {
                id: next_id(),
                action,
                params: [0.0; 4],
                source: neuron_ids[sensor_channels + i],
            }));
        }

        // Fully connected inputs and oscillators to outputs and hidden, and hidden to
        // outputs. Dense on purpose: Phase 1 has no add-connection operator, so any
        // connection absent here can never appear.
        let input_end = sensor_channels;
        let output_end = input_end + effectors.len();
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

        for trait_ in [
            BodyTrait::Size,
            BodyTrait::SignatureR,
            BodyTrait::SignatureG,
            BodyTrait::SignatureB,
        ] {
            genes.push(Gene::Body(BodyGene { trait_, value: 0.0 }));
        }

        for trait_ in [
            MetaTrait::MutationRate,
            MetaTrait::WeightSigma,
            MetaTrait::CrossoverRate,
        ] {
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

        Self {
            genes,
            neurons,
            fan_in_scale,
        }
    }

    /// Phase 1's hardcoded sensor set: `vision_rays` eyes, a nose, and one
    /// interoceptor for the agent's own energy (build plan task 5).
    fn sensor_layout(params: &SimParams) -> Vec<Modality> {
        let mut sensors = vec![Modality::VisionRay; params.sensing.vision_rays as usize];
        sensors.push(Modality::Chemo);
        sensors.push(Modality::Interoception);
        sensors
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
    /// One draw per gene, in gene order, whatever the gene holds. A draw made
    /// conditionally on a gene's contents would make the RNG stream depend on genome
    /// state, and two structurally identical lineages would desynchronise (spec §7.4).
    pub fn instantiate(&self, rng: &mut Rng, params: &SimParams, out: &mut [Gene]) {
        debug_assert_eq!(out.len(), self.genes.len(), "destination is the wrong size");
        out.copy_from_slice(&self.genes);

        let brain = &params.brain;
        let mut connection = 0usize;
        for gene in out.iter_mut() {
            match gene {
                Gene::Neuron(n) => {
                    n.bias = rng.range(-1.0, 1.0);
                    n.tau = rng.range(brain.tau_min, brain.tau_max);
                    if n.activation == Activation::Oscillator {
                        n.period =
                            rng.range(brain.oscillator_period_min, brain.oscillator_period_max);
                    }
                }
                Gene::Connection(c) => {
                    // Scaled by fan-in, not drawn from `weight_limit`: at 24 inputs per
                    // neuron the full bound saturates every sigmoid on tick one and the
                    // brain never responds to a sensor again.
                    let scale = brain.weight_init_scale * self.fan_in_scale[connection];
                    connection += 1;
                    c.weight = rng.range(-scale, scale);
                }
                Gene::Sensor(s) => {
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
                    b.value = match b.trait_ {
                        BodyTrait::Size => params.body.size,
                        // Founders differ in colour so lineages are distinguishable on
                        // screen from the first frame, and so `vision_ray` has
                        // something to discriminate before `set_signature` exists.
                        _ => rng.range(0.15, 1.0),
                    };
                }
                Gene::Meta(m) => {
                    m.value = match m.trait_ {
                        MetaTrait::MutationRate => params.mutation.weight_perturb_rate,
                        MetaTrait::WeightSigma => params.mutation.weight_perturb_sigma,
                        MetaTrait::CrossoverRate => 0.0,
                    }
                }
                Gene::Effector(_) => {}
            }
        }
        debug_assert_eq!(
            connection,
            self.fan_in_scale.len(),
            "fan-in scales are out of step with the connection genes"
        );
        debug_assert!(genome::validate(out).is_ok(), "founder is not coherent");
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
