//! Exact live-genome size distributions shared by the native and WASM shells.
//!
//! Sampling walks live agents outside the tick and mutates nothing, so native metrics
//! and browser diagnostics report identical values for the same completed tick. It is
//! observation only, not a complexity score or fitness signal.

use serde::{Deserialize, Serialize};
use sim_core::genome::Gene;
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
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

pub fn sample_complexity(world: &World) -> ComplexityMetrics {
    let population = world.population() as usize;
    let mut genes = Vec::with_capacity(population);
    let mut neurons = Vec::with_capacity(population);
    let mut connections = Vec::with_capacity(population);
    let mut enabled = Vec::with_capacity(population);
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
    }
    ComplexityMetrics {
        genome_genes: SizeDistribution::from_counts(&mut genes),
        neurons: SizeDistribution::from_counts(&mut neurons),
        connections: SizeDistribution::from_counts(&mut connections),
        enabled_connections: SizeDistribution::from_counts(&mut enabled),
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
