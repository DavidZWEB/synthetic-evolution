//! In-memory checkpoints of a between-ticks [`World`] (spec §7.10).
//!
//! Encodes every authoritative value a run needs to continue exactly, and restores
//! untrusted bytes only after bounded decoding and validation. It does no I/O and
//! knows nothing of saved-run bundles, history archives, or the shells around it.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::arena::{Arena, Block};
use crate::brain;
use crate::command::{Command, Kind};
use crate::control::BrainInheritance;
use crate::effectors;
use crate::genome::{self, Gene};
use crate::ids::{BirthId, NULL_ID, SpeciesId};
use crate::ledger::EnergyLedger;
use crate::params::SimParams;
use crate::perceive;
use crate::plants::SavedPlants;
use crate::rng::Rng;
use crate::spatial::SpatialHash;
use crate::spawn;
use crate::world::{World, WorldBuildError};

/// Leading bytes that identify a checkpoint before any decoding.
pub const CHECKPOINT_MAGIC: [u8; 8] = *b"SEVCKPT\0";

/// Simulation-compatibility identity: bump with any change to this encoding or to the
/// state continuation requires. Phase 2 rejects other versions rather than migrating.
pub const CHECKPOINT_FORMAT: u32 = 5;

const HEADER_BYTES: usize = CHECKPOINT_MAGIC.len() + size_of::<u32>();

/// The receiving host's resource limits for an untrusted checkpoint.
#[derive(Clone, Copy, Debug)]
pub struct CheckpointLimits {
    /// Encoded size, checked before any decoding.
    pub max_bytes: usize,
    /// Ceiling on the saved params' `storage.max_memory_bytes`, the core construction
    /// budget a restore would reserve. A saved run larger than this host accepts is
    /// refused before allocating its world.
    pub max_core_bytes: u64,
}

/// Why a checkpoint was refused. A refusal never touches any existing world.
#[derive(Debug)]
pub enum CheckpointError {
    /// Larger than the caller's byte limit, before any decoding.
    TooLarge,
    /// The saved world's core budget exceeds what this host accepts.
    WorldTooLarge,
    NotACheckpoint,
    UnsupportedFormat(u32),
    Malformed(postcard::Error),
    Build(WorldBuildError),
    Invalid(&'static str),
    /// Every component validated, yet the restored world is not the one that was
    /// saved. The hash is folded over the restored state, so nothing is assumed.
    HashMismatch,
}

impl core::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("checkpoint exceeds the byte limit"),
            Self::WorldTooLarge => f.write_str("saved world exceeds this host's memory budget"),
            Self::NotACheckpoint => f.write_str("not a checkpoint"),
            Self::UnsupportedFormat(version) => {
                write!(
                    f,
                    "unsupported checkpoint format {version} (expected {CHECKPOINT_FORMAT})"
                )
            }
            Self::Malformed(error) => write!(f, "malformed checkpoint: {error}"),
            Self::Build(error) => write!(f, "checkpoint world: {error}"),
            Self::Invalid(reason) => write!(f, "invalid checkpoint: {reason}"),
            Self::HashMismatch => f.write_str("restored checkpoint does not match its state hash"),
        }
    }
}

impl core::error::Error for CheckpointError {}

/// One placed arena block and the values it held.
#[derive(Serialize, Deserialize)]
struct Placed<T> {
    block: Block,
    values: Vec<T>,
}

/// A neuron's live values: the parts a genome cannot reconstruct (spec §3.2).
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Recurrent {
    state: f32,
    output: f32,
    input: f32,
}

#[derive(Serialize, Deserialize)]
struct AgentState {
    slot: u32,
    position: Vec3,
    velocity: Vec3,
    orientation: Quat,
    energy: f32,
    energy_reserve: f64,
    health: f32,
    age: u32,
    species_id: u32,
    signature: Vec3,
    size: f32,
    parent_a: u32,
    parent_b: u32,
    birth_id: u64,
    parent_birth_a: u64,
    parent_birth_b: u64,
    grid_cell: u32,
    parts: Block,
    genome: Placed<Gene>,
    /// Compiled neurons, synapses, sensors, and effectors are rebuilt from the
    /// genome; only placement and recurrent values are saved.
    brain: Placed<Recurrent>,
    synapses: Block,
    sensors: Block,
    effectors: Block,
}

#[derive(Serialize, Deserialize)]
struct SpeciesState {
    id: u32,
    members: u32,
    block: Block,
    representative: Vec<Gene>,
}

/// Intents are not saved: every tick writes them at step 4 before any system reads
/// them (spec §2.4). The continuation tests would diverge if that order changed.
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    seed: u64,
    params: SimParams,
    brain_inheritance: u8,
    state_hash: u64,
    rng: Rng,
    tick: u64,
    next_innovation: u32,
    next_birth: u64,
    unclassified: u32,
    ledger: EnergyLedger,
    grid_cells_per_axis: u32,
    pool_incarnations: Vec<u32>,
    pool_free: Vec<u32>,
    agents: Vec<AgentState>,
    parts: Arena<f32>,
    plant_position: Vec<Vec3>,
    plant_energy: Vec<f32>,
    plant_reserve: Vec<f64>,
    plant_starved: Vec<u32>,
    plants_reseeded: u64,
    chemo: Vec<f32>,
    species_next_id: u32,
    species: Vec<SpeciesState>,
    commands: Vec<Command>,
}

impl World {
    /// Encodes this world at its current between-ticks boundary.
    ///
    /// Reads only: saving neither advances time nor consumes randomness (spec §7.10).
    /// Allocates outside the tick, so shells call it between steps.
    pub fn checkpoint(&self) -> Vec<u8> {
        debug_assert!(
            self.dying.is_empty() && self.breeding.is_empty() && self.due_commands.is_empty(),
            "checkpoints are taken between ticks"
        );
        let agents = self
            .pool
            .iter_live()
            .map(|id| {
                let i = id.index();
                let a = &self.agents;
                AgentState {
                    slot: id.raw(),
                    position: a.position[i],
                    velocity: a.velocity[i],
                    orientation: a.orientation[i],
                    energy: a.energy[i],
                    energy_reserve: a.energy_reserve[i],
                    health: a.health[i],
                    age: a.age[i],
                    species_id: a.species_id[i],
                    signature: a.signature[i],
                    size: a.size[i],
                    parent_a: a.parent_a[i],
                    parent_b: a.parent_b[i],
                    birth_id: a.birth_id[i].raw(),
                    parent_birth_a: a.parent_birth_a[i].raw(),
                    parent_birth_b: a.parent_birth_b[i].raw(),
                    grid_cell: a.grid_cell[i],
                    parts: a.parts[i],
                    genome: Placed {
                        block: a.genome[i],
                        values: self.genes.get(a.genome[i]).to_vec(),
                    },
                    brain: Placed {
                        block: a.brain[i],
                        values: self
                            .brains
                            .get(a.brain[i])
                            .iter()
                            .map(|n| Recurrent {
                                state: n.state,
                                output: n.output,
                                input: n.input,
                            })
                            .collect(),
                    },
                    synapses: a.synapses[i],
                    sensors: a.sensors[i],
                    effectors: a.effectors[i],
                }
            })
            .collect();
        let species = self
            .classifier
            .active()
            .zip(self.classifier.representative_blocks())
            .map(|((id, members), block)| SpeciesState {
                id: id.raw(),
                members,
                block,
                representative: self
                    .classifier
                    .representative(id)
                    .expect("active species have representatives")
                    .to_vec(),
            })
            .collect();
        let checkpoint = Checkpoint {
            seed: self.seed,
            params: self.params.clone(),
            brain_inheritance: self.brain_inheritance as u8,
            state_hash: self.state_hash(),
            rng: self.rng.clone(),
            tick: self.tick,
            next_innovation: self.next_innovation,
            next_birth: self.next_birth,
            unclassified: self.unclassified,
            ledger: self.ledger.clone(),
            grid_cells_per_axis: self.hash.cells_per_axis(),
            pool_incarnations: self.pool.incarnations().to_vec(),
            pool_free: self.pool.free_indices().to_vec(),
            agents,
            parts: self.parts.clone(),
            plant_position: self.plants.position().to_vec(),
            plant_energy: self.plants.energy().to_vec(),
            plant_reserve: self.plants.energy_reserve().to_vec(),
            plant_starved: self.plants.starved().to_vec(),
            plants_reseeded: self.plants.reseeded(),
            chemo: self.field.concentrations().to_vec(),
            species_next_id: self.classifier.next_id(),
            species,
            commands: self.commands.clone(),
        };
        let mut bytes = Vec::from(CHECKPOINT_MAGIC);
        bytes.extend_from_slice(&CHECKPOINT_FORMAT.to_le_bytes());
        postcard::to_extend(&checkpoint, bytes).expect("checkpoint state serializes")
    }

    /// Restores an untrusted checkpoint into a new world within `limits`.
    ///
    /// The world is constructed through the ordinary budgeted constructor, after its
    /// budget is checked against the host's ceiling. Every component is validated,
    /// derived buffers are recompiled from validated genomes, and the result must
    /// reproduce the saved state hash.
    pub fn from_checkpoint(
        bytes: &[u8],
        limits: CheckpointLimits,
    ) -> Result<World, CheckpointError> {
        if bytes.len() > limits.max_bytes {
            return Err(CheckpointError::TooLarge);
        }
        if bytes.len() < HEADER_BYTES || bytes[..CHECKPOINT_MAGIC.len()] != CHECKPOINT_MAGIC {
            return Err(CheckpointError::NotACheckpoint);
        }
        let format = u32::from_le_bytes(
            bytes[CHECKPOINT_MAGIC.len()..HEADER_BYTES]
                .try_into()
                .expect("four header bytes"),
        );
        if format != CHECKPOINT_FORMAT {
            return Err(CheckpointError::UnsupportedFormat(format));
        }
        let (checkpoint, rest) = postcard::take_from_bytes::<Checkpoint>(&bytes[HEADER_BYTES..])
            .map_err(CheckpointError::Malformed)?;
        if !rest.is_empty() {
            return Err(CheckpointError::Invalid(
                "trailing bytes after the checkpoint",
            ));
        }
        if checkpoint.params.storage.max_memory_bytes > limits.max_core_bytes {
            return Err(CheckpointError::WorldTooLarge);
        }
        restore(checkpoint)
    }
}

fn invalid(reason: &'static str) -> CheckpointError {
    CheckpointError::Invalid(reason)
}

fn finite3(v: Vec3) -> bool {
    v.is_finite()
}

fn restore(c: Checkpoint) -> Result<World, CheckpointError> {
    let brain_inheritance = match c.brain_inheritance {
        0 => BrainInheritance::Evolving,
        1 => BrainInheritance::RandomizedAtBirth,
        3 => BrainInheritance::StructuralNull,
        _ => return Err(invalid("unknown brain inheritance")),
    };
    // Construction validates params and the core budget, and regenerates the
    // seed-derived fertility map and founding topology that the hash then confirms.
    // Plant sites are saved: turnover moves them (spec §5.1).
    let mut world = World::new_with_brain_inheritance(c.seed, c.params, brain_inheritance)
        .map_err(CheckpointError::Build)?;
    let capacity = world.pool.capacity();

    // A retune may have lowered the sensing radius after construction; the world
    // keeps its original grid, which can be coarser but never finer (spec §2.3).
    let fresh_cells = world.hash.cells_per_axis();
    if c.grid_cells_per_axis == 0 || c.grid_cells_per_axis > fresh_cells {
        return Err(invalid("spatial grid is finer than these params allow"));
    }
    world.hash =
        SpatialHash::with_cells_per_axis(world.params.world.size, c.grid_cells_per_axis, capacity);

    world
        .pool
        .restore(&c.pool_incarnations, &c.pool_free)
        .map_err(invalid)?;
    if !world
        .pool
        .iter_live()
        .map(|id| id.raw())
        .eq(c.agents.iter().map(|a| a.slot))
    {
        return Err(invalid(
            "agents do not match the pool's live slots in order",
        ));
    }

    // Validate every genome and derived length before placing anything.
    let grid_cell = world.hash.cell_size();
    let channels = world.field.channels();
    for agent in &c.agents {
        let genes = &agent.genome.values;
        if genome::validate(genes).is_err()
            || spawn::validate_limits(genes, &world.params.storage).is_err()
            || spawn::validate_sensor_parameters(genes, grid_cell, channels).is_err()
        {
            return Err(invalid(
                "an agent genome is incoherent or exceeds its limits",
            ));
        }
        let lengths = [
            (agent.genome.block, genes.len()),
            (agent.brain.block, genome::neuron_count(genes)),
            (agent.synapses, brain::synapse_count(genes)),
            (agent.sensors, perceive::sensor_count(genes)),
            (agent.effectors, effectors::effector_count(genes)),
        ];
        if lengths
            .iter()
            .any(|&(block, len)| block.len() as usize != len)
            || agent.brain.values.len() != genome::neuron_count(genes)
        {
            return Err(invalid("an agent block does not match its genome"));
        }
        if !agent
            .brain
            .values
            .iter()
            .all(|n| n.state.is_finite() && n.output.is_finite() && n.input.is_finite())
        {
            return Err(invalid("recurrent neural state must be finite"));
        }
        if ![agent.position, agent.velocity, agent.signature]
            .into_iter()
            .all(finite3)
            || !agent.orientation.is_finite()
            // Neither half of an energy pair is ever negative (`energy::add`); a negative
            // one would hand dissipation a negative amount on death.
            || !(agent.energy.is_finite() && agent.energy >= 0.0)
            || !(agent.energy_reserve.is_finite() && agent.energy_reserve >= 0.0)
            || !agent.health.is_finite()
            || !(agent.size.is_finite() && agent.size > 0.0)
        {
            return Err(invalid(
                "agent physical state must be finite, with non-negative energy",
            ));
        }
        if agent.birth_id != BirthId::NULL.raw() && agent.birth_id >= c.next_birth {
            return Err(invalid("an agent birth ID was never issued"));
        }
    }

    let placements =
        |pick: fn(&AgentState) -> Block| -> Vec<Block> { c.agents.iter().map(pick).collect() };
    world
        .genes
        .restore_placement(&placements(|a| a.genome.block))
        .map_err(invalid)?;
    world
        .brains
        .restore_placement(&placements(|a| a.brain.block))
        .map_err(invalid)?;
    world
        .synapses
        .restore_placement(&placements(|a| a.synapses))
        .map_err(invalid)?;
    world
        .sensors
        .restore_placement(&placements(|a| a.sensors))
        .map_err(invalid)?;
    world
        .effectors
        .restore_placement(&placements(|a| a.effectors))
        .map_err(invalid)?;
    if !c.parts.is_coherent(
        capacity,
        crate::world::PARTS_PER_AGENT,
        &placements(|a| a.parts),
    ) {
        return Err(invalid("parts storage does not match the live agents"));
    }
    world.parts = c.parts;

    for agent in &c.agents {
        let i = agent.slot as usize;
        let genes = &agent.genome.values;
        world
            .genes
            .get_mut(agent.genome.block)
            .copy_from_slice(genes);
        let neurons = world.brains.get_mut(agent.brain.block);
        brain::compile(genes, neurons, world.synapses.get_mut(agent.synapses));
        for (neuron, saved) in world
            .brains
            .get_mut(agent.brain.block)
            .iter_mut()
            .zip(&agent.brain.values)
        {
            neuron.state = saved.state;
            neuron.output = saved.output;
            neuron.input = saved.input;
        }
        perceive::compile(genes, world.sensors.get_mut(agent.sensors));
        effectors::compile(genes, world.effectors.get_mut(agent.effectors));
        let a = &mut world.agents;
        a.position[i] = agent.position;
        a.velocity[i] = agent.velocity;
        a.orientation[i] = agent.orientation;
        a.energy[i] = agent.energy;
        a.energy_reserve[i] = agent.energy_reserve;
        a.health[i] = agent.health;
        a.age[i] = agent.age;
        a.species_id[i] = agent.species_id;
        a.signature[i] = agent.signature;
        a.size[i] = agent.size;
        a.parent_a[i] = agent.parent_a;
        a.parent_b[i] = agent.parent_b;
        a.birth_id[i] = BirthId::new(agent.birth_id);
        a.parent_birth_a[i] = BirthId::new(agent.parent_birth_a);
        a.parent_birth_b[i] = BirthId::new(agent.parent_birth_b);
        a.grid_cell[i] = agent.grid_cell;
        a.brain_units[i] = genome::brain_complexity(genes);
        a.sensor_load[i] = genome::sensor_load(genes);
        a.genome[i] = agent.genome.block;
        a.brain[i] = agent.brain.block;
        a.synapses[i] = agent.synapses;
        a.sensors[i] = agent.sensors;
        a.effectors[i] = agent.effectors;
        a.parts[i] = agent.parts;
    }

    world
        .plants
        .restore_state(SavedPlants {
            position: &c.plant_position,
            energy: &c.plant_energy,
            reserve: &c.plant_reserve,
            starved: &c.plant_starved,
            reseeded: c.plants_reseeded,
        })
        .map_err(invalid)?;
    world
        .field
        .restore_concentrations(&c.chemo)
        .map_err(invalid)?;

    let species: Vec<(SpeciesId, u32, Block, &[Gene])> = c
        .species
        .iter()
        .map(|s| {
            (
                SpeciesId::new(s.id),
                s.members,
                s.block,
                s.representative.as_slice(),
            )
        })
        .collect();
    world
        .classifier
        .restore(c.species_next_id, &species)
        .map_err(invalid)?;
    // Membership is derived from the live agents; the counts must agree exactly.
    let mut unclassified = 0u32;
    for agent in &c.agents {
        if agent.species_id == NULL_ID {
            unclassified += 1;
        } else if !c.species.iter().any(|s| s.id == agent.species_id) {
            return Err(invalid("an agent belongs to an inactive species"));
        }
    }
    for s in &c.species {
        let members = c.agents.iter().filter(|a| a.species_id == s.id).count();
        if members != s.members as usize {
            return Err(invalid(
                "species member counts do not match the live agents",
            ));
        }
    }
    if unclassified != c.unclassified {
        return Err(invalid("unclassified count does not match the live agents"));
    }

    // Innovation IDs at or above the counter would be reissued by later mutations.
    let below_counter = |genes: &[Gene]| {
        genes
            .iter()
            .filter_map(Gene::innovation)
            .all(|id| id.raw() < c.next_innovation)
    };
    if !c.agents.iter().all(|a| below_counter(&a.genome.values))
        || !c.species.iter().all(|s| below_counter(&s.representative))
        || !below_counter(world.plan.genes())
    {
        return Err(invalid(
            "innovation IDs must be below the innovation counter",
        ));
    }

    for command in &c.commands {
        let Kind::SpawnFounder { position } = command.kind;
        if !position.is_finite() {
            return Err(invalid("queued command position must be finite"));
        }
    }
    if ![
        c.ledger.input(),
        c.ledger.dissipated(),
        c.ledger.expected_stock(),
    ]
    .iter()
    .all(|v| v.is_finite())
    {
        return Err(invalid("energy ledger must be finite"));
    }

    world.rng = c.rng;
    world.tick = c.tick;
    world.next_innovation = c.next_innovation;
    world.next_birth = c.next_birth;
    world.unclassified = c.unclassified;
    world.ledger = c.ledger;
    world.commands = c.commands;
    world.due_commands.reserve(world.commands.len());

    if world.state_hash() != c.state_hash {
        return Err(CheckpointError::HashMismatch);
    }
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Block;

    const UNLIMITED: CheckpointLimits = CheckpointLimits {
        max_bytes: usize::MAX,
        max_core_bytes: u64::MAX,
    };

    #[test]
    fn host_budget_is_enforced_before_construction() {
        let bytes = encode(&checkpoint());
        let budget = checkpoint().params.storage.max_memory_bytes;
        let tight = CheckpointLimits {
            max_bytes: usize::MAX,
            max_core_bytes: budget - 1,
        };
        assert!(matches!(
            World::from_checkpoint(&bytes, tight),
            Err(CheckpointError::WorldTooLarge)
        ));
    }

    fn checkpoint() -> Checkpoint {
        let mut params = SimParams::default();
        params.world.max_agents = 8;
        params.plants.max_plants = 4;
        params.species.threshold = 1e-6;
        let mut world = World::new(9, params).unwrap();
        world.seed_founders(4);
        for _ in 0..3 {
            world.step();
        }
        let bytes = world.checkpoint();
        postcard::from_bytes(&bytes[HEADER_BYTES..]).unwrap()
    }

    fn encode(checkpoint: &Checkpoint) -> Vec<u8> {
        let mut bytes = Vec::from(CHECKPOINT_MAGIC);
        bytes.extend_from_slice(&CHECKPOINT_FORMAT.to_le_bytes());
        postcard::to_extend(checkpoint, bytes).unwrap()
    }

    /// Each case must be refused by validation, not merely by the final hash, because
    /// validation is what keeps corrupt indices and sizes out of the tick.
    #[test]
    fn semantic_corruption_is_refused_before_the_hash_check() {
        assert!(World::from_checkpoint(&encode(&checkpoint()), UNLIMITED).is_ok());
        type Corrupt = fn(&mut Checkpoint);
        let cases: [(&str, Corrupt); 13] = [
            ("overlapping genomes", |c| {
                c.agents[1].genome.block = c.agents[0].genome.block;
            }),
            ("block shorter than genome", |c| {
                c.agents[0].synapses = Block::EMPTY
            }),
            ("agents out of pool order", |c| c.agents.swap(0, 1)),
            ("repeated free slot", |c| {
                let first = c.pool_free[0];
                c.pool_free.push(first);
            }),
            ("finer grid", |c| c.grid_cells_per_axis += 1),
            ("non-finite recurrent state", |c| {
                c.agents[0].brain.values[0].state = f32::NAN;
            }),
            ("innovation counter behind genomes", |c| {
                c.next_innovation = 0
            }),
            ("species membership mismatch", |c| c.species[0].members += 1),
            ("member of an inactive species", |c| {
                c.agents[0].species_id = 999
            }),
            ("unknown heredity", |c| c.brain_inheritance = 7),
            ("negative agent energy", |c| c.agents[0].energy = -1.0),
            ("negative agent reserve", |c| {
                c.agents[0].energy_reserve = -0.5
            }),
            ("unissued birth ID", |c| c.next_birth = 0),
        ];
        for (name, corrupt) in cases {
            let mut c = checkpoint();
            corrupt(&mut c);
            match World::from_checkpoint(&encode(&c), UNLIMITED) {
                Err(CheckpointError::Invalid(_)) => {}
                other => panic!(
                    "{name}: expected a validation refusal, got {:?}",
                    other.err()
                ),
            }
        }
    }

    #[test]
    fn a_consistent_but_different_world_fails_the_hash_check() {
        let mut c = checkpoint();
        c.tick += 1;
        assert!(matches!(
            World::from_checkpoint(&encode(&c), UNLIMITED),
            Err(CheckpointError::HashMismatch)
        ));
    }
}
