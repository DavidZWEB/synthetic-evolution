//! Serializable headless telemetry sampled from completed world ticks.
//!
//! Sampling walks live agents in index order and mutates nothing. It is called only by
//! the native shell when metrics are requested, so ordinary simulation ticks pay no
//! instrumentation cost.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use crate::Result;
use serde::{Deserialize, Serialize};
use sim_core::genome::Gene;
use sim_core::ids::SpeciesId;
use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use sim_core::spawn::{ArenaUsage, SpawnFailureCounts};
use sim_core::species::SpeciesEventCounts;
use sim_core::state_hash::genome_fingerprint;
use sim_core::world::World;

#[path = "../../shared/complexity.rs"]
mod complexity;
pub use complexity::{ComplexityMetrics, SizeDistribution};
#[cfg(test)]
#[path = "../../shared/complexity_case.rs"]
mod complexity_case;

pub const SCHEMA_VERSION: u32 = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum MetricsRecord {
    Header(Box<RunHeader>),
    Sample(Box<RunSample>),
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
    /// A mid-run intervention: from `at_tick` on, both worlds ran with `params`.
    /// Absent for an ordinary run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retune: Option<Retune>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Retune {
    pub at_tick: u64,
    pub params: SimParams,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesPopulation {
    pub species_id: SpeciesId,
    pub population: u32,
    /// The share of its living members' lifetime intake eaten from other agents, by
    /// bites and carrion (spec §7.9). Absent when they have eaten nothing yet, or in a
    /// file written before diets were recorded.
    #[serde(default)]
    pub meat_share: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeciesMetrics {
    /// Active immutable representatives, sorted by historical ID rather than slot.
    pub populations: Vec<SpeciesPopulation>,
    pub unclassified_population: u32,
    /// Cumulative shell observations, not authoritative classifier state.
    pub events: Option<SpeciesEventCounts>,
}

/// Cumulative history-capture counts at a sample. Gaps are archived drop ranges, so
/// `dropped_events > 0` means ancestry after that point is incomplete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryAvailability {
    pub capacity: u32,
    pub retained_events: u64,
    pub dropped_events: u64,
    pub gaps: u64,
}

impl HistoryAvailability {
    pub fn is_complete(&self) -> bool {
        self.dropped_events == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldMetrics {
    pub population: u32,
    /// Living non-founder agents.
    pub descendants: u32,
    /// Exact genome fingerprints among living agents, not M4 species clusters.
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
    /// Owned energy below the corresponding visible `f32` value's resolution.
    pub energy_rounding_reserve: f64,
    pub energy_drift: f64,
    /// Current element usage, sampled outside the tick; not a memory/RSS estimate.
    pub arena_usage: Vec<ArenaUsage>,
    /// Cumulative shell observations, or unavailable when stepping was not observed.
    pub spawn_failures: Option<SpawnFailureCounts>,
    /// Cumulative candidate-edit observations, not a count of successful births.
    /// Schema 3 and unobserved runs have no structural observations; schema 4 has
    /// neural counts but no organ observations.
    #[serde(default)]
    pub structural_mutations: Option<StructuralMutationCounts>,
    /// Schemas 3–5 did not classify agents; their absent data must stay unknown.
    #[serde(default)]
    pub species: Option<SpeciesMetrics>,
    /// Exact live genome-size distributions. Schemas 3–7 did not record them; their
    /// absence is unknown, not an empty population.
    #[serde(default)]
    pub complexity: Option<ComplexityMetrics>,
    /// Schema 8 writes `null` when history capture was off. Older schemas did not
    /// record availability, so readers must use the schema to tell the two apart.
    #[serde(default)]
    pub history: Option<HistoryAvailability>,
    /// Plant ecology (spec §5.1, §5.3). Files written before M9 did not record it;
    /// its absence is unknown, not a world without plants.
    #[serde(default)]
    pub plants: Option<PlantMetrics>,
    /// Evolvable body traits (spec §3.5). Files written before Phase 3 did not record
    /// them; their absence is unknown, not a population of reference bodies.
    #[serde(default)]
    pub bodies: Option<BodyMetrics>,
    /// Who eats whom (spec §7.9). Files written before the bite did not record it; its
    /// absence is unknown, not a world without predation.
    #[serde(default)]
    pub predation: Option<PredationMetrics>,
}

/// Bites, kills, carrion, and diet, at one sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PredationMetrics {
    /// Swings, hits, and kills since the world began.
    pub swings: u64,
    pub hits: u64,
    pub kills: u64,
    /// Corpses on the ground, and the energy they hold.
    pub corpses: u32,
    pub corpse_energy: f64,
    /// What the living have eaten over their lives: from plants, and from other agents
    /// through bites and carrion.
    pub eaten_plants: f64,
    pub eaten_animals: f64,
    /// Energy held by living agents that took most of their food from other agents,
    /// by those that took at least half from plants, and by those that have eaten
    /// nothing yet: the trophic tiers spec §7.9 diagnoses, with newborns kept out of
    /// both until they eat.
    pub carnivore_biomass: f64,
    pub herbivore_biomass: f64,
    pub unfed_biomass: f64,
}

/// Each evolvable body trait across living agents, at one sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BodyMetrics {
    /// Radius in world units, the value `body.size_range` bounds.
    pub size: TraitDistribution,
    /// Multiples of the reference, as `body.muscle_range` bounds them.
    pub muscle: TraitDistribution,
    pub mouth: TraitDistribution,
}

/// Nearest-rank order statistics over one value per living agent, as
/// [`SizeDistribution`] gives them for counts. An empty population reports zeros; its
/// population count, not these values, establishes that nothing lived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TraitDistribution {
    pub min: f64,
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
    pub max: f64,
    pub mean: f64,
}

impl TraitDistribution {
    fn from_values(values: &mut [f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable_by(f64::total_cmp);
        let n = values.len();
        let rank = |numerator: usize| values[(n * numerator).div_ceil(4).max(1) - 1];
        Self {
            min: values[0],
            p25: rank(1),
            median: rank(2),
            p75: rank(3),
            max: values[n - 1],
            mean: values.iter().sum::<f64>() / n as f64,
        }
    }
}

/// How the plants are spread and how fast they turn over, at one sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlantMetrics {
    /// Plants that have died and reseeded since the world was built.
    pub reseeded: u64,
    /// Clark–Evans ratio: the mean distance from each plant to its nearest neighbour,
    /// over that expected for the same density scattered at random on the torus. Near
    /// 1 is random, below 1 clustered; absent with fewer than two plants.
    pub clustering: Option<f64>,
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
    spawn_failures: Option<[SpawnFailureCounts; 2]>,
    structural_mutations: Option<[StructuralMutationCounts; 2]>,
    species_events: Option<[SpeciesEventCounts; 2]>,
    history: Option<[HistoryAvailability; 2]>,
    include_state_hashes: bool,
) -> Result<RunSample> {
    debug_assert_eq!(evolving.tick_count(), random_control.tick_count());
    let mut evolving_metrics = sample_world(evolving)?;
    let mut control_metrics = sample_world(random_control)?;
    if let Some([evolving, control]) = spawn_failures {
        evolving_metrics.spawn_failures = Some(evolving);
        control_metrics.spawn_failures = Some(control);
    }
    if let Some([evolving, control]) = structural_mutations {
        evolving_metrics.structural_mutations = Some(evolving);
        control_metrics.structural_mutations = Some(control);
    }
    if let Some([evolving, control]) = species_events {
        evolving_metrics
            .species
            .as_mut()
            .expect("current world")
            .events = Some(evolving);
        control_metrics
            .species
            .as_mut()
            .expect("current world")
            .events = Some(control);
    }
    if let Some([evolving, control]) = history {
        evolving_metrics.history = Some(evolving);
        control_metrics.history = Some(control);
    }
    Ok(RunSample {
        tick: evolving.tick_count(),
        evolving: evolving_metrics,
        random_control: control_metrics,
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
    let descendants = world.living_descendants();

    for id in world.pool().iter_live() {
        let i = id.index();
        let agents = world.agents();
        energy.add(
            agents.energy[i] as f64 + agents.energy_reserve[i],
            "agent energy",
        )?;
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
    }

    let diets = species_diets(world);
    let ledger = world.ledger();
    let plant_energy = finite(world.plants().total_energy(), "plant energy")?;
    let total_energy = finite(world.total_energy(), "total energy")?;
    let cumulative_energy_input = finite(ledger.input(), "cumulative energy input")?;
    let cumulative_dissipation = finite(ledger.dissipated(), "cumulative dissipation")?;
    let energy_rounding_reserve = finite(world.energy_reserve(), "energy rounding reserve")?;
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
        energy_rounding_reserve,
        energy_drift,
        arena_usage: world.storage_usage().to_vec(),
        spawn_failures: None,
        structural_mutations: None,
        species: Some(SpeciesMetrics {
            populations: world
                .species()
                .active()
                .map(|(species_id, population)| SpeciesPopulation {
                    species_id,
                    population,
                    meat_share: diets
                        .get(&species_id.raw())
                        .and_then(|&diet| meat_share(diet)),
                })
                .collect(),
            unclassified_population: world.unclassified_population(),
            events: None,
        }),
        complexity: Some(complexity::sample_complexity(world)),
        history: None,
        plants: Some(PlantMetrics {
            reseeded: world.plants().reseeded(),
            clustering: plant_clustering(world.plants(), world.params().world.size),
        }),
        bodies: Some(body_metrics(world)),
        predation: Some(predation_metrics(world)?),
    })
}

/// The share of `(plants, animals)` eaten from other agents, absent when nothing was.
pub(crate) fn meat_share((plants, animals): (f64, f64)) -> Option<f64> {
    let eaten = plants + animals;
    (eaten > 0.0).then(|| animals / eaten)
}

/// Each classified species' living members' lifetime intake, from plants and from
/// other agents.
fn species_diets(world: &World) -> BTreeMap<u32, (f64, f64)> {
    let agents = world.agents();
    let mut diets = BTreeMap::new();
    for id in world.pool().iter_live() {
        let i = id.index();
        let diet: &mut (f64, f64) = diets.entry(agents.species_id[i]).or_default();
        diet.0 += agents.eaten_plants[i];
        diet.1 += agents.eaten_animals[i];
    }
    diets
}

fn predation_metrics(world: &World) -> Result<PredationMetrics> {
    let counts = world.bite_counts();
    let agents = world.agents();
    let mut metrics = PredationMetrics {
        swings: counts.swings,
        hits: counts.hits,
        kills: counts.kills,
        corpses: world.corpses().count() as u32,
        corpse_energy: finite(world.corpses().total_energy(), "corpse energy")?,
        ..PredationMetrics::default()
    };
    for id in world.pool().iter_live() {
        let i = id.index();
        let plants = agents.eaten_plants[i];
        let animals = agents.eaten_animals[i];
        metrics.eaten_plants += plants;
        metrics.eaten_animals += animals;
        let held = f64::from(agents.energy[i]) + agents.energy_reserve[i];
        // Most of its food from other agents: what spec §7.9 calls a carnivore.
        if plants + animals == 0.0 {
            metrics.unfed_biomass += held;
        } else if animals > plants {
            metrics.carnivore_biomass += held;
        } else {
            metrics.herbivore_biomass += held;
        }
    }
    Ok(metrics)
}

fn body_metrics(world: &World) -> BodyMetrics {
    let agents = world.agents();
    let trait_of = |values: &[f32]| {
        let mut live: Vec<f64> = world
            .pool()
            .iter_live()
            .map(|id| f64::from(values[id.index()]))
            .collect();
        TraitDistribution::from_values(&mut live)
    };
    BodyMetrics {
        size: trait_of(&agents.size),
        muscle: trait_of(&agents.muscle),
        mouth: trait_of(&agents.mouth),
    }
}

/// The Clark–Evans aggregation ratio of `plants` on a torus of side `size`.
///
/// Each nearest neighbour is found through the plant grid, widening the search until
/// something answers, so a sample costs about one grid query per plant rather than
/// comparing every pair.
fn plant_clustering(plants: &sim_core::Plants, size: f32) -> Option<f64> {
    let positions = plants.position();
    if positions.len() < 2 {
        return None;
    }
    let first = plants.hash().cell_size().min(size * 0.5);
    let mut total = 0.0f64;
    for (index, &at) in positions.iter().enumerate() {
        let mut radius = first;
        let nearest = loop {
            let mut best = f32::INFINITY;
            plants
                .hash()
                .for_each_within(positions, at, radius, |other, _, d2| {
                    if other as usize != index {
                        best = best.min(d2);
                    }
                });
            if best.is_finite() || radius >= size * 0.5 {
                break best;
            }
            radius = (radius * 2.0).min(size * 0.5);
        };
        // Beyond half the world on both axes the grid cannot look; compare directly.
        let nearest = if nearest.is_finite() {
            nearest
        } else {
            positions
                .iter()
                .enumerate()
                .filter(|&(other, _)| other != index)
                .map(|(_, &p)| sim_core::spatial::min_image(p - at, size).length_squared())
                .fold(f32::INFINITY, f32::min)
        };
        total += f64::from(nearest).sqrt();
    }
    let observed = total / positions.len() as f64;
    let density = positions.len() as f64 / (f64::from(size) * f64::from(size));
    Some(observed / (0.5 / density.sqrt()))
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

    use super::complexity_case;

    #[test]
    fn plant_clustering_reads_one_for_scatter_and_less_for_patches() {
        let ratio = |patchiness: f32| {
            let mut params = SimParams::default();
            params.plants.patchiness = patchiness;
            let world = World::new(3, params).unwrap();
            sample_world(&world)
                .unwrap()
                .plants
                .unwrap()
                .clustering
                .unwrap()
        };
        let scattered = ratio(0.0);
        assert!(
            (0.9..1.1).contains(&scattered),
            "uniform scatter read {scattered}"
        );
        let patchy = ratio(6.0);
        assert!(patchy < 0.8, "patchy plants read {patchy}");
    }

    #[test]
    fn plant_metrics_count_reseeds_and_skip_clustering_below_two_plants() {
        let mut params = SimParams::default();
        params.plants.max_plants = 50;
        params.plants.initial_fill = 0.0;
        params.plants.grazing_lag = 0.9;
        params.plants.death_stock = 0.5;
        params.plants.death_seconds = 0.1;
        let mut world = World::new(4, params.clone()).unwrap();
        for _ in 0..30 {
            world.step();
        }
        let plants = sample_world(&world).unwrap().plants.unwrap();
        assert_eq!(plants.reseeded, world.plants().reseeded());
        assert!(
            plants.reseeded > 0,
            "nothing starved, so nothing was counted"
        );

        params.plants.max_plants = 1;
        let lone = World::new(4, params).unwrap();
        assert_eq!(
            sample_world(&lone).unwrap().plants.unwrap().clustering,
            None
        );
    }

    #[test]
    fn complexity_matches_the_shared_cross_target_reference() {
        let params: SimParams = serde_json::from_str(complexity_case::PARAMS).unwrap();
        let mut world = World::new(complexity_case::SEED, params).unwrap();
        assert_eq!(
            world.seed_founders(complexity_case::FOUNDERS),
            complexity_case::FOUNDERS
        );
        for _ in 0..complexity_case::TICKS {
            world.step();
        }
        let complexity = sample_world(&world).unwrap().complexity.unwrap();
        assert!(
            complexity.genome_genes.min < complexity.genome_genes.max,
            "the reference must exercise a nontrivial distribution"
        );
        assert!(complexity.enabled_connections.mean < complexity.connections.mean);
        assert_eq!(
            serde_json::to_string(&complexity).unwrap(),
            complexity_case::EXPECTED
        );
    }

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
        assert_eq!(metrics.energy_rounding_reserve, 0.0);
        assert_eq!(metrics.arena_usage, world.storage_usage());
        assert_eq!(metrics.spawn_failures, None);
        assert_eq!(metrics.structural_mutations, None);
        assert_eq!(metrics.species, Some(SpeciesMetrics::default()));
        let complexity = metrics.complexity.expect("recorded");
        assert!(
            complexity.is_empty(),
            "an empty world reports zeroes, recorded"
        );
        assert!(complexity.wiring.is_some());
        assert_eq!(metrics.history, None);
        assert_eq!(metrics.bodies, Some(BodyMetrics::default()));
    }

    #[test]
    fn trait_quartiles_are_nearest_rank() {
        let mut values = [4.0, 1.0, 3.0, 2.0, 8.0];
        assert_eq!(
            TraitDistribution::from_values(&mut values),
            TraitDistribution {
                min: 1.0,
                p25: 2.0,
                median: 3.0,
                p75: 4.0,
                max: 8.0,
                mean: 3.6,
            }
        );
        assert_eq!(
            TraitDistribution::from_values(&mut []),
            TraitDistribution::default()
        );
    }

    #[test]
    fn body_traits_are_sampled_from_living_agents() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 2;
        let mut world = World::new(1, params).expect("valid params");
        let ids: Vec<_> = (0..3)
            .map(|i| {
                world
                    .spawn_founder(Vec3::new(i as f32, 0.0, 0.0))
                    .expect("room")
            })
            .collect();
        for (&id, (size, muscle, mouth)) in
            ids.iter()
                .zip([(2.0, 0.5, 4.0), (5.0, 2.0, 0.25), (9.0, 9.0, 9.0)])
        {
            let agents = world.agents_mut();
            agents.size[id.index()] = size;
            agents.muscle[id.index()] = muscle;
            agents.mouth[id.index()] = mouth;
        }
        world.despawn(ids[2]);
        let bodies = sample_world(&world)
            .expect("samples")
            .bodies
            .expect("recorded");
        assert_eq!((bodies.size.min, bodies.size.max), (2.0, 5.0));
        assert_eq!((bodies.muscle.min, bodies.muscle.max), (0.5, 2.0));
        assert_eq!((bodies.mouth.min, bodies.mouth.max), (0.25, 4.0));
        assert_eq!(bodies.mouth.mean, 2.125);
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
        let species = metrics.species.unwrap();
        assert_eq!(species.populations.len() as u32, world.species_count());
        assert_eq!(
            species
                .populations
                .iter()
                .map(|row| row.population)
                .sum::<u32>()
                + species.unclassified_population,
            1
        );
        assert_eq!(species.events, None);
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

        let sample =
            sample_pair(&evolving, &control, None, None, None, None, true).expect("samples");
        assert_eq!(sample.evolving, sample.random_control);
        assert_eq!(
            sample.final_state_hashes.as_ref().unwrap().evolving,
            sample.final_state_hashes.as_ref().unwrap().random_control,
            "same-seed founders should begin in the same state"
        );
    }

    #[test]
    fn paired_observations_preserve_cohort_identity_and_unavailable_counts() {
        let params: SimParams =
            serde_json::from_str(r#"{"world":{"max_agents":2},"plants":{"max_plants":1}}"#)
                .unwrap();
        let evolving = World::new(1, params.clone()).unwrap();
        let control = World::new(1, params).unwrap();
        let counts = [
            SpawnFailureCounts {
                arena_capacity: 3,
                ..Default::default()
            },
            SpawnFailureCounts {
                arena_fragmentation: 7,
                ..Default::default()
            },
        ];
        let mut mutations = [StructuralMutationCounts::default(); 2];
        mutations[0].add_neuron.attempted = 3;
        mutations[0].add_neuron.genome_limit = 3;
        mutations[1].remove_connection.attempted = 7;
        mutations[1].remove_connection.applied = 7;
        let species = [
            SpeciesEventCounts {
                unclassified_capacity: 5,
                ..Default::default()
            },
            SpeciesEventCounts {
                unclassified_id_exhausted: 9,
                ..Default::default()
            },
        ];
        let history = [
            HistoryAvailability {
                capacity: 4,
                retained_events: 2,
                ..Default::default()
            },
            HistoryAvailability {
                capacity: 4,
                retained_events: 4,
                dropped_events: 3,
                gaps: 1,
            },
        ];
        let sample = sample_pair(
            &evolving,
            &control,
            Some(counts),
            Some(mutations),
            Some(species),
            Some(history),
            false,
        )
        .unwrap();
        assert_eq!(sample.evolving.history, Some(history[0]));
        assert_eq!(sample.random_control.history, Some(history[1]));
        assert_eq!(sample.evolving.spawn_failures, Some(counts[0]));
        assert_eq!(sample.random_control.spawn_failures, Some(counts[1]));
        assert_eq!(sample.evolving.structural_mutations, Some(mutations[0]));
        assert_eq!(
            sample.random_control.structural_mutations,
            Some(mutations[1])
        );
        assert_eq!(sample.evolving.species.unwrap().events, Some(species[0]));
        assert_eq!(
            sample.random_control.species.unwrap().events,
            Some(species[1])
        );
        let unobserved = sample_pair(&evolving, &control, None, None, None, None, false).unwrap();
        let json = serde_json::to_value(unobserved).unwrap();
        for cohort in ["evolving", "random_control"] {
            assert_eq!(
                json[cohort].get("spawn_failures"),
                Some(&serde_json::Value::Null)
            );
            assert_eq!(
                json[cohort].get("structural_mutations"),
                Some(&serde_json::Value::Null)
            );
            assert_eq!(
                json[cohort]["species"]["populations"],
                serde_json::json!([])
            );
            assert_eq!(json[cohort]["species"]["unclassified_population"], 0);
            assert_eq!(
                json[cohort]["species"].get("events"),
                Some(&serde_json::Value::Null)
            );
            assert_eq!(json[cohort].get("history"), Some(&serde_json::Value::Null));
            assert_eq!(json[cohort]["complexity"]["neurons"]["median"], 0);
        }
    }

    #[test]
    fn disabled_classification_reports_living_unclassified_agents_not_zero_population() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 2;
        params.species.capacity = 0;
        let mut world = World::new(1, params).unwrap();
        assert_eq!(world.seed_founders(3), 3);
        let species = sample_world(&world).unwrap().species.unwrap();
        assert!(species.populations.is_empty());
        assert_eq!(species.unclassified_population, 3);
        let first = world.pool().iter_live().next().unwrap();
        world.despawn(first);
        assert_eq!(
            sample_world(&world)
                .unwrap()
                .species
                .unwrap()
                .unclassified_population,
            2
        );
    }

    #[test]
    fn extinct_species_leave_no_zero_population_rows() {
        let mut params = SimParams::default();
        params.world.max_agents = 2;
        params.plants.max_plants = 1;
        let mut world = World::new(1, params).unwrap();
        let id = world.spawn_founder(Vec3::ZERO).unwrap();
        assert_eq!(
            sample_world(&world)
                .unwrap()
                .species
                .unwrap()
                .populations
                .len(),
            1
        );
        world.despawn(id);
        assert_eq!(
            sample_world(&world).unwrap().species,
            Some(SpeciesMetrics::default())
        );
    }

    #[test]
    fn sampling_preserves_historical_species_ids_when_slots_are_reused() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 1;
        params.species.capacity = 2;
        params.species.threshold = 1e-12;
        let mut world = World::new(1, params).unwrap();
        let first = world.spawn_founder(Vec3::ZERO).unwrap();
        let second = world.spawn_founder(Vec3::ZERO).unwrap();
        let second_species = world.agents().species_id[second.index()];
        world.despawn(first);
        let third = world.spawn_founder(Vec3::ZERO).unwrap();
        let third_species = world.agents().species_id[third.index()];
        let metrics = sample_world(&world).unwrap().species.unwrap();
        assert_eq!(
            metrics.populations,
            vec![
                SpeciesPopulation {
                    species_id: SpeciesId::new(second_species),
                    population: 1,
                    meat_share: None,
                },
                SpeciesPopulation {
                    species_id: SpeciesId::new(third_species),
                    population: 1,
                    meat_share: None,
                },
            ]
        );
        assert!(third_species > second_species);
        assert_eq!(metrics.unclassified_population, 0);
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
