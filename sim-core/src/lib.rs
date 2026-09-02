//! Synthetic Evolution simulation core.
//!
//! Owns world state and advances it one deterministic tick at a time. It is a pure
//! library: it does no I/O, holds no `static` mutable state, and never allocates in
//! the tick. Every shell — native CLI, wasm worker — owns its own I/O and calls in
//! through the narrow surface here.
//!
//! Deliberately not here: rendering, file formats, threading policy, and anything
//! that would make two `World`s in one process interfere with each other.

pub mod agents;
pub mod arena;
pub mod brain;
pub mod chemo;
pub mod command;
pub mod crossover;
pub mod effectors;
pub mod feeding;
pub mod founder;
pub mod genome;
pub mod ids;
pub mod ledger;
pub mod math;
pub mod metabolism;
pub mod movement;
pub mod mutate;
pub mod params;
pub mod perceive;
pub mod plants;
pub mod pool;
pub mod reproduction;
pub mod rng;
pub mod spatial;
pub mod state_hash;
pub mod tick;
pub mod world;

pub use agents::{Agents, SpawnSpec};
pub use arena::{Arena, Block};
pub use brain::{Neuron, Synapse};
pub use chemo::ChemoField;
pub use effectors::{Effector, Intents};
pub use ids::{AgentId, InnovationId, NULL_ID, NeuronId, PartId};
pub use ledger::EnergyLedger;
pub use params::{ParamError, SimParams};
pub use perceive::Sensor;
pub use plants::Plants;
pub use pool::SlotPool;
pub use rng::Rng;
pub use spatial::SpatialHash;
pub use world::World;
