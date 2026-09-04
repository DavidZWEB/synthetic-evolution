//! WASM shell: the `wasm-bindgen` surface for the browser worker.
//!
//! Owns the JS boundary and nothing else — it holds a `World` and its render snapshot,
//! steps them, and hands JS the addresses to read. Nothing here decides anything about
//! the simulation; if a function does more than marshal, it belongs in `sim-core`.
//!
//! **The boundary is per-tick, never per-agent** (spec §7.3). [`Sim::step_many`] runs a
//! whole batch and refreshes the snapshot once at the end, because the renderer only
//! ever draws the last state — refreshing per tick would do a hundred times the work at
//! 100× speed and throw away ninety-nine of the results. Inspector data is pulled for
//! one selected agent at human speed, so it can afford JSON.
//!
//! **On views detaching.** WASM linear memory *is* the buffer JS reads, so a zero-copy
//! view over the snapshot stays valid only while that memory does not move. Growing it
//! detaches every existing `TypedArray`, silently. The pool, the arenas and the snapshot
//! are all sized at construction so the tick itself never grows memory — but an
//! allocator asked for anything at all may still take a new page, and `push_command` and
//! `inspect_agent` both allocate. So this shell takes spec §7.3's second option as well
//! as its first: [`Sim::snapshot_layout`] can be called at any time, and JS rebuilds its
//! views whenever `memory.buffer` is not the one it built them from. That check is a
//! pointer comparison per frame.

use serde::Serialize;
use wasm_bindgen::prelude::*;

use sim_core::command::Command;
use sim_core::ids::AgentId;
use sim_core::params::SimParams;
use sim_core::snapshot::Snapshot;
use sim_core::world::World;

/// Crate version, so the worker can assert it matches the JS bundle it shipped with.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn js_error(context: &str, detail: impl core::fmt::Display) -> JsError {
    JsError::new(&format!("{context}: {detail}"))
}

/// Where one snapshot array lives in WASM memory, right now.
#[derive(Serialize)]
struct Span {
    ptr: usize,
    len: usize,
}

/// Everything JS needs to build its views, in one call.
///
/// Field names are snake_case, like everything else crossing this boundary: `SimParams`
/// and `Command` are `deny_unknown_fields`, so a camelCase key there is a hard error,
/// and a boundary where some keys are one and some the other is how a typo becomes a
/// silent no-op instead of a loud failure.
#[derive(Serialize)]
struct Layout {
    /// Slots in every array. `population` is how many currently hold an agent.
    capacity: u32,
    population: u32,
    tick: u64,
    /// `x, y, z` per slot.
    position: Span,
    /// `x, y, z, w` per slot.
    orientation: Span,
    size: Span,
    /// `r, g, b` per slot.
    signature: Span,
    /// One byte per slot; everything else is meaningless where this is 0.
    alive: Span,
    species: Span,
    part_offset: Span,
    part_count: Span,
    /// Slots in the plant arrays. Fixed for the life of a world, like `capacity`.
    plant_capacity: u32,
    /// `x, y, z` per plant.
    plant_position: Span,
    /// What each site holds, so a fat plant draws differently from a bare one.
    plant_energy: Span,
}

/// What a renderer needs that is not per-frame: the extent it is drawing into, and how
/// to draw a plant.
///
/// Read from the world rather than agreed by convention. A client with its own copy of
/// `world.size` draws a correct picture of the wrong world the moment either moves.
#[derive(Serialize)]
struct RenderHints {
    world_size: f32,
    /// Simulated seconds advanced by one fixed tick.
    seconds_per_tick: f32,
    agent_capacity: u32,
    plant_capacity: u32,
    plant_radius: f32,
    plant_signature: [f32; 3],
    /// What a full site holds, so a renderer can show how full one is.
    plant_max_energy: f32,
}

fn span<T>(slice: &[T]) -> Span {
    Span {
        ptr: slice.as_ptr() as usize,
        len: slice.len(),
    }
}

/// What the inspector shows for one selected agent (spec §2.2b).
#[derive(Serialize)]
struct Inspection<'a> {
    index: u32,
    tick: u64,
    energy: f32,
    age: u32,
    size: f32,
    signature: [f32; 3],
    species_id: u32,
    parent_a: u32,
    /// Always `NULL_ID` in Phase 1: reproduction is asexual until Phase 6 (spec §9.1).
    parent_b: u32,
    brain_units: u32,
    sensor_load: f32,
    /// Live neuron outputs, in brain-slot order — the activations, not the genome's
    /// description of them.
    activations: Vec<f32>,
    genome: &'a [sim_core::genome::Gene],
}

/// One simulation and the buffer a frame is drawn from.
#[wasm_bindgen]
pub struct Sim {
    world: World,
    snapshot: Snapshot,
}

#[wasm_bindgen]
impl Sim {
    /// Builds a world from a seed and an optional JSON `SimParams`.
    ///
    /// Omitting the params means the shipped defaults, which is what a first-time
    /// visitor gets. Malformed or invalid params are an error rather than a silent
    /// fallback: a client that thinks it set a value and did not would be tuning
    /// something it cannot see.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64, params_json: Option<String>) -> Result<Sim, JsError> {
        let params = match params_json.as_deref() {
            Some(json) => serde_json::from_str(json).map_err(|e| js_error("bad params", e))?,
            None => SimParams::default(),
        };
        let world = World::new(seed, params).map_err(|e| js_error("invalid params", e.0))?;
        let snapshot = Snapshot::for_world(&world);
        Ok(Sim { world, snapshot })
    }

    /// Seeds generation 0, and reports how many the pool had room for.
    ///
    /// The layout belongs to `sim-core` — it is folded into every seeded run, so a
    /// browser and a headless sweep that arranged founders differently would not be
    /// running the same experiment. This is a direct call rather than a command because
    /// seeding is a boundary condition: there is no tick to stamp it for yet.
    pub fn seed_founders(&mut self, count: u32) -> u32 {
        let placed = self.world.seed_founders(count);
        self.refresh();
        placed
    }

    /// Advances `ticks` ticks and refreshes the snapshot once, at the end.
    pub fn step_many(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.world.step();
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        self.snapshot.update(&self.world);
    }

    /// Addresses and lengths of every snapshot array, as JSON.
    ///
    /// Safe to call whenever; JS should call it again any time `memory.buffer` differs
    /// from the one its views were built over, because that is exactly when they have
    /// detached.
    pub fn snapshot_layout(&self) -> Result<String, JsError> {
        let layout = Layout {
            capacity: self.snapshot.capacity(),
            population: self.snapshot.population(),
            tick: self.snapshot.tick(),
            position: span(self.snapshot.position()),
            orientation: span(self.snapshot.orientation()),
            size: span(self.snapshot.size()),
            signature: span(self.snapshot.signature()),
            alive: span(self.snapshot.alive()),
            species: span(self.snapshot.species()),
            part_offset: span(self.snapshot.part_offset()),
            part_count: span(self.snapshot.part_count()),
            plant_capacity: self.snapshot.plant_capacity(),
            plant_position: span(self.snapshot.plant_position()),
            plant_energy: span(self.snapshot.plant_energy()),
        };
        serde_json::to_string(&layout).map_err(|e| js_error("layout", e))
    }

    /// Everything a renderer needs that does not change frame to frame.
    pub fn render_hints(&self) -> Result<String, JsError> {
        let params = self.world.params();
        let hints = RenderHints {
            world_size: params.world.size,
            seconds_per_tick: params.world.dt,
            agent_capacity: self.snapshot.capacity(),
            plant_capacity: self.snapshot.plant_capacity(),
            plant_radius: params.plants.radius,
            plant_signature: params.plants.signature,
            plant_max_energy: params.plants.max_energy,
        };
        serde_json::to_string(&hints).map_err(|e| js_error("render hints", e))
    }

    pub fn tick(&self) -> u64 {
        self.world.tick_count()
    }

    pub fn population(&self) -> u32 {
        self.world.population()
    }

    /// Mean energy of the living population, sampled on demand by the UI.
    pub fn mean_energy(&self) -> f64 {
        self.world.mean_agent_energy()
    }

    /// The world's fingerprint. Exposed so a browser run can be compared against a
    /// headless one on the same seed without shipping the whole state (spec §7.8).
    pub fn state_hash(&self) -> u64 {
        self.world.state_hash()
    }

    pub fn params_json(&self) -> Result<String, JsError> {
        serde_json::to_string(self.world.params()).map_err(|e| js_error("params", e))
    }

    /// Retunes the world. Errors on anything that would resize what is already
    /// allocated — see `World::set_params` for which fields those are and why.
    pub fn set_params(&mut self, params_json: &str) -> Result<(), JsError> {
        let params: SimParams =
            serde_json::from_str(params_json).map_err(|e| js_error("bad params", e))?;
        self.world
            .set_params(params)
            .map_err(|e| js_error("rejected params", e.0))
    }

    /// Queues a request, to apply on the tick it is stamped for (spec §2.2b).
    pub fn push_command(&mut self, command_json: &str) -> Result<(), JsError> {
        let command: Command =
            serde_json::from_str(command_json).map_err(|e| js_error("bad command", e))?;
        self.world.push_command(command);
        Ok(())
    }

    pub fn pending_commands(&self) -> usize {
        self.world.pending_commands()
    }

    /// Everything the inspector shows for one agent, as JSON.
    ///
    /// Pulled for the one selected agent rather than streamed for everybody: this is
    /// kilobytes per agent against the snapshot's 57 bytes, and it is read at the rate a
    /// human clicks (spec §2.2b).
    pub fn inspect_agent(&self, index: u32) -> Result<String, JsError> {
        let id = AgentId::new(index);
        if index >= self.world.pool().capacity() || !self.world.pool().is_alive(id) {
            return Err(JsError::new(&format!("no live agent at slot {index}")));
        }
        let agents = self.world.agents();
        let i = id.index();
        let inspection = Inspection {
            index,
            tick: self.world.tick_count(),
            energy: agents.energy[i],
            age: agents.age[i],
            size: agents.size[i],
            signature: agents.signature[i].to_array(),
            species_id: agents.species_id[i],
            parent_a: agents.parent_a[i],
            parent_b: agents.parent_b[i],
            brain_units: agents.brain_units[i],
            sensor_load: agents.sensor_load[i],
            activations: self.world.brain(id).iter().map(|n| n.output).collect(),
            genome: self.world.genome(id),
        };
        serde_json::to_string(&inspection).map_err(|e| js_error("inspect", e))
    }
}
