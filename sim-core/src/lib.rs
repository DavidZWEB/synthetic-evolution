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
pub mod ids;
pub mod math;
pub mod params;
pub mod pool;
pub mod rng;
pub mod spatial;
pub mod world;

pub use agents::{Agents, SpawnSpec};
pub use arena::{Arena, Block};
pub use ids::{AgentId, InnovationId, NULL_ID, NeuronId, PartId};
pub use params::{ParamError, SimParams};
pub use pool::SlotPool;
pub use rng::Rng;
pub use spatial::SpatialHash;
pub use world::World;
