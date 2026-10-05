//! Exact live-genome size and wiring distributions shared by the native and WASM
//! shells.
//!
//! Sampling walks live agents outside the tick and mutates nothing, so native metrics
//! and browser diagnostics report identical values for the same completed tick. It is
//! observation only, not a complexity score or fitness signal.

use serde::{Deserialize, Serialize};
use sim_core::genome::{self, Activation, Gene, SensorGene};
use sim_core::world::World;

/// Nearest-rank order statistics over one count per living agent.
///
/// Integer ranks keep native and browser results exact. An empty population reports
/// zeros; its population count, not these values, establishes that nothing lived.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SizeDistribution {
    pub min: u32,
    pub p25: u32,
    pub median: u32,
    pub p75: u32,
    pub max: u32,
    pub mean: f64,
}

impl SizeDistribution {
    fn from_counts(counts: &mut [u32]) -> Self {
        if counts.is_empty() {
            return Self::default();
        }
        counts.sort_unstable();
        let n = counts.len();
        let rank = |numerator: usize| counts[(n * numerator).div_ceil(4).max(1) - 1];
        let sum: u64 = counts.iter().map(|&count| u64::from(count)).sum();
        Self {
            min: counts[0],
            p25: rank(1),
            median: rank(2),
            p75: rank(3),
            max: counts[n - 1],
            mean: sum as f64 / n as f64,
        }
    }

    /// Whether the order statistics are internally consistent.
    pub fn is_ordered(&self) -> bool {
        self.min <= self.p25
            && self.p25 <= self.median
            && self.median <= self.p75
            && self.p75 <= self.max
            && self.mean.is_finite()
            && self.mean >= f64::from(self.min)
            && self.mean <= f64::from(self.max)
    }

    /// Whether every order statistic is at most `other`'s, as it must be when each
    /// agent's count here never exceeds its count there.
    pub fn is_bounded_by(&self, other: &Self) -> bool {
        self.min <= other.min
            && self.p25 <= other.p25
            && self.median <= other.median
            && self.p75 <= other.p75
            && self.max <= other.max
            && self.mean <= other.mean
    }
}

/// Live genome sizes. Neuron and connection genes are what metabolism charges for;
/// disabled connections still cost, so enabled connections are reported separately.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    pub genome_genes: SizeDistribution,
    pub neurons: SizeDistribution,
    pub connections: SizeDistribution,
    pub enabled_connections: SizeDistribution,
    /// How much of that structure can affect behavior. Absent from files written
    /// before it was recorded, which is unknown, not zero.
    #[serde(default)]
    pub wiring: Option<WiringMetrics>,
}

/// Structure on a working path, per living agent (spec §7.9).
///
/// A neuron is on a path when an enabled-connection chain leads to it from an input
/// (a sensor target or an oscillator) and from it to an output (an effector source).
/// Gene counts include structure nothing reads; these do not. They describe wiring,
/// not whether that wiring is useful.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WiringMetrics {
    /// Hidden neurons (neither sensor targets, effector sources, nor oscillators) on a
    /// path.
    pub wired_hidden_neurons: SizeDistribution,
    /// Sensors with at least one target channel that reaches an effector.
    pub wired_sensors: SizeDistribution,
    /// Effectors whose source is reachable from an input or oscillator; the rest emit
    /// a constant.
    pub driven_effectors: SizeDistribution,
}

/// Per-genome wiring counts, in `WiringMetrics` order. `on_sensor` hears each sensor, in
/// gene order, with whether one of its target channels reaches an effector: the
/// per-organ view of `wired_sensors`.
pub fn wiring_counts(
    genes: &[Gene],
    from_inputs: &mut Vec<bool>,
    to_outputs: &mut Vec<bool>,
    mut on_sensor: impl FnMut(&SensorGene, bool),
) -> [u32; 3] {
    let neurons = genome::neuron_count(genes);
    let index = |id| genome::neuron_index(genes, id).expect("validated reference");
    from_inputs.clear();
    from_inputs.resize(neurons, false);
    to_outputs.clear();
    to_outputs.resize(neurons, false);
    let mut role = vec![false; neurons];
    for (position, gene) in genes.iter().enumerate() {
        match gene {
            Gene::Neuron(n) if n.activation == Activation::Oscillator => {
                from_inputs[position] = true;
                role[position] = true;
            }
            Gene::Sensor(s) => {
                for &target in &s.targets[..s.modality.channels()] {
                    from_inputs[index(target)] = true;
                    role[index(target)] = true;
                }
            }
            Gene::Effector(e) => {
                to_outputs[index(e.source)] = true;
                role[index(e.source)] = true;
            }
            _ => {}
        }
    }
    let edges: Vec<(usize, usize)> = genes
        .iter()
        .filter_map(|gene| match gene {
            Gene::Connection(c) if c.enabled => Some((index(c.from), index(c.to))),
            _ => None,
        })
        .collect();
    // Fixed points over a small edge list: genomes are bounded by storage.max_genes.
    for (reach, forward) in [(&mut *from_inputs, true), (&mut *to_outputs, false)] {
        let mut changed = true;
        while changed {
            changed = false;
            for &(from, to) in &edges {
                let (source, sink) = if forward { (from, to) } else { (to, from) };
                if reach[source] && !reach[sink] {
                    reach[sink] = true;
                    changed = true;
                }
            }
        }
    }
    let wired_hidden = (0..neurons)
        .filter(|&i| !role[i] && from_inputs[i] && to_outputs[i])
        .count() as u32;
    let mut wired_sensors = 0;
    let mut driven_effectors = 0;
    for gene in genes {
        match gene {
            Gene::Sensor(s) => {
                let wired = s.targets[..s.modality.channels()]
                    .iter()
                    .any(|&target| to_outputs[index(target)]);
                on_sensor(s, wired);
                wired_sensors += u32::from(wired);
            }
            Gene::Effector(e) => driven_effectors += u32::from(from_inputs[index(e.source)]),
            _ => {}
        }
    }
    [wired_hidden, wired_sensors, driven_effectors]
}

impl ComplexityMetrics {
    pub fn is_consistent(&self) -> bool {
        [
            &self.genome_genes,
            &self.neurons,
            &self.connections,
            &self.enabled_connections,
        ]
        .iter()
        .all(|distribution| distribution.is_ordered())
            && self.enabled_connections.is_bounded_by(&self.connections)
            && self.neurons.is_bounded_by(&self.genome_genes)
            && self.connections.is_bounded_by(&self.genome_genes)
            && self.wiring.as_ref().is_none_or(|wiring| {
                [
                    &wiring.wired_hidden_neurons,
                    &wiring.wired_sensors,
                    &wiring.driven_effectors,
                ]
                .iter()
                .all(|distribution| distribution.is_ordered())
                    && wiring.wired_hidden_neurons.is_bounded_by(&self.neurons)
            })
    }

    /// Whether every recorded distribution is the empty population's.
    pub fn is_empty(&self) -> bool {
        Self {
            wiring: None,
            ..self.clone()
        } == Self::default()
            && self
                .wiring
                .as_ref()
                .is_none_or(|wiring| *wiring == WiringMetrics::default())
    }
}

pub fn sample_complexity(world: &World) -> ComplexityMetrics {
    let population = world.population() as usize;
    let mut genes = Vec::with_capacity(population);
    let mut neurons = Vec::with_capacity(population);
    let mut connections = Vec::with_capacity(population);
    let mut enabled = Vec::with_capacity(population);
    let mut wired = [
        Vec::with_capacity(population),
        Vec::with_capacity(population),
        Vec::with_capacity(population),
    ];
    let (mut from_inputs, mut to_outputs) = (Vec::new(), Vec::new());
    for id in world.pool().iter_live() {
        let genome = world.genome(id);
        let (mut neuron_count, mut connection_count, mut enabled_count) = (0, 0, 0);
        for gene in genome {
            match gene {
                Gene::Neuron(_) => neuron_count += 1,
                Gene::Connection(connection) => {
                    connection_count += 1;
                    enabled_count += u32::from(connection.enabled);
                }
                _ => {}
            }
        }
        genes.push(genome.len() as u32);
        neurons.push(neuron_count);
        connections.push(connection_count);
        enabled.push(enabled_count);
        for (counts, count) in wired.iter_mut().zip(wiring_counts(
            genome,
            &mut from_inputs,
            &mut to_outputs,
            |_, _| {},
        )) {
            counts.push(count);
        }
    }
    let [mut hidden, mut sensors, mut effectors] = wired;
    ComplexityMetrics {
        genome_genes: SizeDistribution::from_counts(&mut genes),
        neurons: SizeDistribution::from_counts(&mut neurons),
        connections: SizeDistribution::from_counts(&mut connections),
        enabled_connections: SizeDistribution::from_counts(&mut enabled),
        wiring: Some(WiringMetrics {
            wired_hidden_neurons: SizeDistribution::from_counts(&mut hidden),
            wired_sensors: SizeDistribution::from_counts(&mut sensors),
            driven_effectors: SizeDistribution::from_counts(&mut effectors),
        }),
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use sim_core::params::SimParams;

    use super::*;

    #[test]
    fn nearest_rank_statistics_are_exact_integers() {
        let mut counts = [7, 1, 3, 5, 9];
        assert_eq!(
            SizeDistribution::from_counts(&mut counts),
            SizeDistribution {
                min: 1,
                p25: 3,
                median: 5,
                p75: 7,
                max: 9,
                mean: 5.0,
            }
        );
        let mut one = [4];
        let single = SizeDistribution::from_counts(&mut one);
        assert_eq!(
            (single.min, single.p25, single.p75, single.max),
            (4, 4, 4, 4)
        );
        let mut four = [10, 20, 30, 40];
        let even = SizeDistribution::from_counts(&mut four);
        assert_eq!((even.p25, even.median, even.p75), (10, 20, 30));
        assert_eq!(
            SizeDistribution::from_counts(&mut []),
            SizeDistribution::default()
        );
    }

    #[test]
    fn samples_live_genomes_and_separates_disabled_connections() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 1;
        let mut world = World::new(3, params).unwrap();
        assert!(sample_complexity(&world).is_empty());
        let a = world.spawn_founder(Vec3::ZERO).unwrap();
        let genome = world.genome(a);
        let neurons = genome
            .iter()
            .filter(|g| matches!(g, Gene::Neuron(_)))
            .count() as u32;
        let connections = genome
            .iter()
            .filter(|g| matches!(g, Gene::Connection(_)))
            .count() as u32;
        let metrics = sample_complexity(&world);
        assert_eq!(metrics.genome_genes.max, genome.len() as u32);
        assert_eq!(metrics.neurons.median, neurons);
        assert_eq!(metrics.connections.median, connections);
        assert_eq!(
            metrics.neurons.max + metrics.connections.max,
            world.agents().brain_units[a.index()]
        );
        assert!(metrics.is_consistent());

        let b = world.spawn_founder(Vec3::ONE).unwrap();
        world.despawn(a);
        let metrics = sample_complexity(&world);
        assert_eq!(metrics.genome_genes.max, world.genome(b).len() as u32);
        assert!(metrics.is_consistent());
    }

    #[test]
    fn wiring_counts_only_structure_on_an_input_to_output_path() {
        use sim_core::genome::{
            Action, ConnectionGene, EffectorGene, Modality, NeuronGene, SensorGene,
        };
        use sim_core::ids::InnovationId;
        let id = InnovationId::new;
        let neuron = |n, activation| {
            Gene::Neuron(NeuronGene {
                id: id(n),
                tau: 1.0,
                activation,
                period: if activation == Activation::Oscillator {
                    10.0
                } else {
                    0.0
                },
                ..Default::default()
            })
        };
        let wire = |n, from, to, enabled| {
            Gene::Connection(ConnectionGene {
                id: id(n),
                from: id(from),
                to: id(to),
                weight: 1.0,
                enabled,
            })
        };
        let effector = |n, source| {
            Gene::Effector(EffectorGene {
                id: id(n),
                action: Action::Thrust,
                source: id(source),
                ..Default::default()
            })
        };
        // 0: energy input, 1: wired hidden, 2: a second organ's dead-end input, 3: hidden
        // cut off by a disabled edge, 4: driven output, 5: undriven output.
        let organ = |n, target| {
            let mut targets = [InnovationId::NULL; 4];
            targets[0] = id(target);
            Gene::Sensor(SensorGene {
                id: id(n),
                modality: Modality::Interoception,
                params: [0.0; 4],
                targets,
            })
        };
        let genes = [
            neuron(0, Activation::Sigmoid),
            neuron(1, Activation::Sigmoid),
            neuron(2, Activation::Sigmoid),
            neuron(3, Activation::Sigmoid),
            neuron(4, Activation::Sigmoid),
            neuron(5, Activation::Sigmoid),
            organ(10, 0),
            organ(13, 2),
            effector(11, 4),
            effector(12, 5),
            wire(20, 0, 1, true),
            wire(21, 1, 4, true),
            wire(22, 0, 2, true),
            wire(23, 0, 3, false),
            wire(24, 3, 5, true),
        ];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut organs = Vec::new();
        assert_eq!(
            wiring_counts(&genes, &mut a, &mut b, |sensor, wired| organs
                .push((sensor.id.raw(), wired))),
            [1, 1, 1]
        );
        assert_eq!(
            organs,
            [(10, true), (13, false)],
            "each organ, in gene order"
        );
    }

    #[test]
    fn founders_report_their_wiring_and_metrics_stay_consistent() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 1;
        let mut world = World::new(3, params).unwrap();
        world.spawn_founder(Vec3::ZERO).unwrap();
        let metrics = sample_complexity(&world);
        let wiring = metrics.wiring.as_ref().unwrap();
        // Every founder output takes one input, from the nose or a clock.
        assert_eq!(wiring.driven_effectors.max, 4);
        assert_eq!(wiring.wired_hidden_neurons.max, 0);
        assert!(metrics.is_consistent());
        assert!(!metrics.is_empty());
    }

    #[test]
    fn inconsistent_distributions_are_detected() {
        let mut metrics = ComplexityMetrics {
            genome_genes: SizeDistribution {
                min: 2,
                p25: 2,
                median: 2,
                p75: 2,
                max: 2,
                mean: 2.0,
            },
            ..Default::default()
        };
        assert!(metrics.is_consistent());
        metrics.enabled_connections.max = 1;
        metrics.enabled_connections.mean = 0.5;
        assert!(!metrics.is_consistent(), "enabled exceeds all connections");
        metrics.enabled_connections = SizeDistribution::default();
        metrics.genome_genes.p25 = 3;
        assert!(!metrics.is_consistent(), "quantiles out of order");
    }
}
