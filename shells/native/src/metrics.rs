//! Serializable headless telemetry sampled from completed world ticks.
//!
//! Sampling walks live agents in index order and mutates nothing. It is called only by
//! the native shell when metrics are requested, so ordinary simulation ticks pay no
//! instrumentation cost.

use std::collections::BTreeSet;
use std::io;

use serde::{Deserialize, Serialize};
use sim_core::NULL_ID;

use crate::Result;
use sim_core::genome::Gene;
use sim_core::params::SimParams;
use sim_core::state_hash::genome_fingerprint;
use sim_core::world::World;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum MetricsRecord {
    Header(RunHeader),
    Sample(RunSample),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunHeader {
    pub schema_version: u32,
    pub sim_version: String,
    /// Git revision for the native runtime sources, with `-dirty` when that scope
    /// differs from the commit.
    pub source_revision: String,
    pub phase: u32,
    /// Decimal text preserves the full u64 range in every JSON consumer.
    pub seed: String,
    pub ticks: u64,
    pub founders: u32,
    pub sample_every: u64,
    pub params: SimParams,
    pub control: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunSample {
    pub tick: u64,
    pub evolving: WorldMetrics,
    pub random_control: WorldMetrics,
    /// Present on the final sample so a reported run can be reproduced and checked.
    pub final_state_hashes: Option<StateHashes>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateHashes {
    pub evolving: String,
    pub random_control: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub mean: f64,
    pub max: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldMetrics {
    pub population: u32,
    /// Living non-founder agents.
    pub descendants: u32,
    /// Exact genome fingerprints among living agents. Phase 2 adds species clusters.
    pub genome_variants: u32,
    pub agent_energy: Summary,
    pub plant_energy: f64,
    pub total_energy: f64,
    pub speed: Summary,
    pub age: Summary,
    pub brain_units: Summary,
    pub sensor_load: Summary,
    pub genome_genes: Summary,
    /// A drift descriptor, not a fitness or heredity score.
    pub mean_abs_connection_weight: f64,
    pub cumulative_energy_input: f64,
    pub cumulative_dissipation: f64,
    pub energy_drift: f64,
}

#[derive(Default)]
struct Accumulator {
    sum: f64,
    max: f64,
    count: u64,
}

impl Accumulator {
    fn add(&mut self, value: f64, name: &str) -> Result<()> {
        if !value.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-finite telemetry value for {name}"),
            )
            .into());
        }
        self.sum += value;
        if !self.sum.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("telemetry sum overflowed for {name}"),
            )
            .into());
        }
        self.max = self.max.max(value);
        self.count += 1;
        Ok(())
    }

    fn summary(&self) -> Summary {
        Summary {
            mean: if self.count == 0 {
                0.0
            } else {
                self.sum / self.count as f64
            },
            max: self.max,
        }
    }
}

pub fn sample_pair(
    evolving: &World,
    random_control: &World,
    include_state_hashes: bool,
) -> Result<RunSample> {
    debug_assert_eq!(evolving.tick_count(), random_control.tick_count());
    Ok(RunSample {
        tick: evolving.tick_count(),
        evolving: sample_world(evolving)?,
        random_control: sample_world(random_control)?,
        final_state_hashes: include_state_hashes.then(|| StateHashes {
            evolving: format!("{:016x}", evolving.state_hash()),
            random_control: format!("{:016x}", random_control.state_hash()),
        }),
    })
}

pub fn sample_world(world: &World) -> Result<WorldMetrics> {
    let mut genomes = BTreeSet::new();
    let mut energy = Accumulator::default();
    let mut speed = Accumulator::default();
    let mut age = Accumulator::default();
    let mut brain_units = Accumulator::default();
    let mut sensor_load = Accumulator::default();
    let mut genome_genes = Accumulator::default();
    let mut connection_weight = Accumulator::default();
    let mut descendants = 0u32;

    for id in world.pool().iter_live() {
        let i = id.index();
        let agents = world.agents();
        energy.add(agents.energy[i] as f64, "agent energy")?;
        let velocity = agents.velocity[i];
        let speed_value = ((velocity.x as f64).powi(2)
            + (velocity.y as f64).powi(2)
            + (velocity.z as f64).powi(2))
        .sqrt();
        speed.add(speed_value, "speed")?;
        age.add(agents.age[i] as f64, "age")?;
        brain_units.add(agents.brain_units[i] as f64, "brain units")?;
        sensor_load.add(agents.sensor_load[i] as f64, "sensor load")?;

        let genome = world.genome(id);
        genomes.insert(genome_fingerprint(genome));
        genome_genes.add(genome.len() as f64, "genome genes")?;
        for gene in genome {
            if let Gene::Connection(connection) = gene {
                connection_weight
                    .add(connection.weight.abs() as f64, "absolute connection weight")?;
            }
        }

        if agents.parent_a[i] != NULL_ID {
            descendants += 1;
        }
    }

    let ledger = world.ledger();
    let plant_energy = finite(world.plants().total_energy(), "plant energy")?;
    let total_energy = finite(world.total_energy(), "total energy")?;
    let cumulative_energy_input = finite(ledger.input(), "cumulative energy input")?;
    let cumulative_dissipation = finite(ledger.dissipated(), "cumulative dissipation")?;
    let energy_drift = finite(world.energy_drift(), "energy ledger drift")?;
    Ok(WorldMetrics {
        population: world.population(),
        descendants,
        genome_variants: genomes.len() as u32,
        agent_energy: energy.summary(),
        plant_energy,
        total_energy,
        speed: speed.summary(),
        age: age.summary(),
        brain_units: brain_units.summary(),
        sensor_load: sensor_load.summary(),
        genome_genes: genome_genes.summary(),
        mean_abs_connection_weight: connection_weight.summary().mean,
        cumulative_energy_input,
        cumulative_dissipation,
        energy_drift,
    })
}

fn finite(value: f64, name: &str) -> Result<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("non-finite telemetry value for {name}"),
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use sim_core::control::BrainInheritance;
    use sim_core::params::SimParams;

    use super::*;

    #[test]
    fn an_empty_population_has_total_zero_summaries() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 2;
        let world = World::new(1, params).expect("valid params");
        let metrics = sample_world(&world).expect("samples");
        assert_eq!(metrics.population, 0);
        assert_eq!(metrics.agent_energy, Summary::default());
        assert_eq!(metrics.speed, Summary::default());
        assert!(metrics.plant_energy > 0.0);
    }

    #[test]
    fn sampling_reads_only_live_agents() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 2;
        let mut world = World::new(1, params).expect("valid params");
        let a = world.spawn_founder(Vec3::ZERO).expect("room");
        let b = world.spawn_founder(Vec3::new(1.0, 0.0, 0.0)).expect("room");
        world.agents_mut().energy[a.index()] = 10.0;
        world.agents_mut().energy[b.index()] = 30.0;
        world.despawn(a);

        let metrics = sample_world(&world).expect("samples");
        assert_eq!(metrics.population, 1);
        assert_eq!(metrics.agent_energy.mean, 30.0);
        assert_eq!(metrics.genome_variants, 1);
    }

    #[test]
    fn paired_worlds_start_with_identical_population_metrics() {
        let mut params = SimParams::default();
        params.world.max_agents = 8;
        params.plants.max_plants = 8;
        let mut evolving = World::new(11, params.clone()).expect("valid params");
        let mut control =
            World::new_with_brain_inheritance(11, params, BrainInheritance::RandomizedAtBirth)
                .expect("valid params");
        assert_eq!(evolving.seed_founders(4), 4);
        assert_eq!(control.seed_founders(4), 4);

        let sample = sample_pair(&evolving, &control, true).expect("samples");
        assert_eq!(sample.evolving, sample.random_control);
        assert_eq!(
            sample.final_state_hashes.as_ref().unwrap().evolving,
            sample.final_state_hashes.as_ref().unwrap().random_control,
            "same-seed founders should begin in the same state"
        );
    }

    #[test]
    fn opening_stock_and_telemetry_use_the_same_precision() {
        let mut params = SimParams::default();
        params.world.max_agents = 1;
        params.plants.max_plants = 10_000;
        params.plants.max_energy = 1.0;
        params.plants.initial_fill = 0.1;
        let world = World::new(3, params).expect("valid params");

        let metrics = sample_world(&world).expect("samples");
        assert_eq!(metrics.total_energy, metrics.plant_energy);
        assert_eq!(metrics.energy_drift, 0.0);
    }

    #[test]
    fn descendants_are_identified_by_parentage_not_age() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 8;
        params.reproduction.maturity_ticks = 0;
        let mut world = World::new(13, params).expect("valid params");
        let parent = world.spawn_founder(Vec3::ZERO).expect("room");
        let rich = world.params().reproduction.threshold + 100.0;
        world.agents_mut().energy[parent.index()] = rich;
        world.intents_mut().reproduce[parent.index()] = 1.0;
        assert_eq!(world.resolve_births(), 1);
        // A birth during tick zero has the same age as a founder after step 11, so age
        // cannot distinguish it. Parentage can.
        world.advance_tick();
        assert_eq!(sample_world(&world).expect("samples").descendants, 1);
    }

    #[test]
    fn aggregate_unsafe_plant_stock_is_rejected_at_the_boundary() {
        let mut params = SimParams::default();
        params.world.max_agents = 1;
        params.plants.max_plants = 8;
        params.plants.max_energy = f32::MAX;
        assert!(World::new(5, params).is_err());
    }

    #[test]
    fn non_finite_state_is_rejected_before_json_serialization() {
        let mut params = SimParams::default();
        params.world.max_agents = 1;
        params.plants.max_plants = 1;
        let mut world = World::new(5, params).expect("valid params");
        let agent = world.spawn_founder(Vec3::ZERO).expect("room");
        world.agents_mut().energy[agent.index()] = f32::INFINITY;
        assert!(sample_world(&world).is_err());
    }
}
