//! WASM shell: the `wasm-bindgen` surface for the browser worker.
//!
//! Owns the JS boundary — it holds a `World`, its render snapshot, and optional
//! history capture, steps them, and hands JS the observations to read. Nothing here
//! decides anything about the simulation; persistence belongs to the JS worker.
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

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use sim_core::checkpoint::{CHECKPOINT_FORMAT, CheckpointLimits};
use sim_core::command::Command;
use sim_core::control::BrainInheritance;
use sim_core::ids::{AgentId, BirthId};
use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use sim_core::snapshot::Snapshot;
use sim_core::spawn::{ArenaUsage, SpawnFailureCounts};
use sim_core::species::SpeciesEventCounts;
use sim_core::world::World;

// The native metrics reader also uses this module's validation helpers.
#[path = "../../shared/cohort_capture.rs"]
mod cohort_capture;
#[allow(dead_code)]
#[path = "../../shared/complexity.rs"]
mod complexity;
mod history;
#[path = "../../shared/history_event_wire.rs"]
mod history_event_wire;
// Native checks an archive's completeness against `LATER_PARAMS`; the browser checks
// the same shape in JS before calling in.
#[allow(dead_code)]
#[path = "../../shared/later_params.rs"]
mod later_params;
// Native assembles manifests with `Manifest::new`; the browser builds them in JS.
#[allow(dead_code)]
#[path = "../../shared/saved_run.rs"]
mod saved_run;

use cohort_capture::CohortCapture;

/// Crate version, so the worker can assert it matches the JS bundle it shipped with.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn js_error(context: &str, detail: impl core::fmt::Display) -> JsError {
    JsError::new(&format!("{context}: {detail}"))
}

fn parse_params(params_json: Option<&str>) -> Result<SimParams, JsError> {
    let params = match params_json {
        Some(json) => serde_json::from_str(json).map_err(|e| js_error("bad params", e))?,
        None => SimParams::default(),
    };
    params
        .validate()
        .map_err(|e| js_error("invalid params", e))?;
    Ok(params)
}

/// An archive's params as its run had them: a field the archive predates reads as that
/// run's value, not today's default, exactly as the native history reader reads it.
fn parse_archive_params(params_json: &str) -> Result<SimParams, JsError> {
    let wire: serde_json::Value =
        serde_json::from_str(params_json).map_err(|e| js_error("bad params", e))?;
    let mut params = SimParams::deserialize(&wire).map_err(|e| js_error("bad params", e))?;
    later_params::restore(&mut params, &wire);
    params
        .validate()
        .map_err(|e| js_error("invalid params", e))?;
    Ok(params)
}

/// Validates and canonicalizes params without allocating a world.
///
/// Used before replacing a running browser world so a malformed shared URL cannot
/// destroy the valid simulation already on screen.
#[wasm_bindgen]
pub fn validate_params(params_json: Option<String>) -> Result<String, JsError> {
    let params = parse_params(params_json.as_deref())?;
    serde_json::to_string(&params).map_err(|e| js_error("params", e))
}

#[derive(Serialize)]
struct Comparison {
    disjoint: usize,
    excess: usize,
    normalizer: usize,
    matching_connections: usize,
    mean_weight_difference: f64,
    /// Each weighted term of `value`, so marker turnover is visible apart from weights.
    disjoint_term: f64,
    excess_term: f64,
    weight_term: f64,
    value: f64,
    threshold: f64,
}

/// Genetic distance between two archived representatives under an archive's params,
/// exactly as the classifier computes it (spec §3.4). Observation only.
#[wasm_bindgen]
pub fn compare_representatives(
    a_json: &str,
    b_json: &str,
    params_json: &str,
) -> Result<String, JsError> {
    let params = parse_archive_params(params_json)?;
    let genome = |json: &str| -> Result<Vec<sim_core::genome::Gene>, JsError> {
        let genes: Vec<sim_core::genome::Gene> =
            serde_json::from_str(json).map_err(|e| js_error("representative", e))?;
        if genes.len() > params.storage.max_genes as usize {
            return Err(js_error("representative", "exceeds storage.max_genes"));
        }
        sim_core::genome::validate(&genes)
            .map_err(|e| js_error("representative", format!("{e:?}")))?;
        Ok(genes)
    };
    let (a, b) = (genome(a_json)?, genome(b_json)?);
    let coefficients = &params.distance;
    let d = sim_core::distance::between(&a, &b, coefficients);
    let comparison = Comparison {
        disjoint: d.disjoint,
        excess: d.excess,
        normalizer: d.normalizer,
        matching_connections: d.matching_connections,
        mean_weight_difference: d.mean_weight_difference,
        disjoint_term: f64::from(coefficients.disjoint_coefficient)
            * (d.disjoint as f64 / d.normalizer as f64),
        excess_term: f64::from(coefficients.excess_coefficient)
            * (d.excess as f64 / d.normalizer as f64),
        weight_term: f64::from(coefficients.weight_coefficient) * d.mean_weight_difference,
        value: d.value,
        threshold: params.species.threshold,
    };
    serde_json::to_string(&comparison).map_err(|e| js_error("comparison", e))
}

/// The core checkpoint format this build reads and writes.
#[wasm_bindgen]
pub fn checkpoint_format() -> u32 {
    CHECKPOINT_FORMAT
}

#[derive(Serialize)]
struct DecodedSavedRun {
    manifest: saved_run::Manifest,
    /// `[offset, length]` of each cohort checkpoint within the bundle bytes.
    checkpoints: Vec<[usize; 2]>,
    /// `[offset, length]` of each included archive, or `null` per unavailable segment.
    history: Vec<Option<[usize; 2]>>,
}

/// Validates a saved run's framing, manifest, and every cohort checkpoint, then
/// returns the manifest with section offsets. History archives are validated by
/// the caller's archive reader. The restored worlds are dropped.
#[wasm_bindgen]
pub fn decode_saved_run(
    bytes: &[u8],
    max_bytes: usize,
    max_core_bytes: u64,
) -> Result<String, JsError> {
    let run = saved_run::decode(bytes, max_bytes).map_err(|e| js_error("saved run", e))?;
    saved_run::restore_cohorts(&run, bytes.len(), max_core_bytes)
        .map_err(|e| js_error("saved run", e))?;
    let base = bytes.as_ptr() as usize;
    let span = |section: &[u8]| [section.as_ptr() as usize - base, section.len()];
    let decoded = DecodedSavedRun {
        checkpoints: run.checkpoints.iter().map(|s| span(s)).collect(),
        history: run.history.iter().map(|s| s.map(span)).collect(),
        manifest: run.manifest,
    };
    serde_json::to_string(&decoded).map_err(|e| js_error("saved run", e))
}

/// Frames a saved run from a manifest and its sections concatenated in order:
/// each cohort checkpoint, then each included history archive.
#[wasm_bindgen]
pub fn encode_saved_run(manifest_json: &str, sections: &[u8]) -> Result<Vec<u8>, JsError> {
    let manifest: saved_run::Manifest =
        serde_json::from_str(manifest_json).map_err(|e| js_error("saved-run manifest", e))?;
    let lengths: Vec<usize> = manifest
        .cohorts
        .iter()
        .map(|c| c.bytes.0 as usize)
        .chain(manifest.history.iter().filter_map(|s| match s {
            saved_run::Segment::Included { bytes, .. } => Some(bytes.0 as usize),
            saved_run::Segment::Unavailable { .. } => None,
        }))
        .collect();
    if lengths.iter().sum::<usize>() != sections.len() {
        return Err(js_error(
            "saved run",
            "sections do not match the manifest lengths",
        ));
    }
    let mut rest = sections;
    let parts: Vec<&[u8]> = lengths
        .iter()
        .map(|&len| {
            let (part, tail) = rest.split_at(len);
            rest = tail;
            part
        })
        .collect();
    saved_run::encode(&manifest, &parts).map_err(|e| js_error("saved run", e))
}

/// Builds the scalar-inheritance control (spec §7.8). Topology and sensors are inherited
/// and can evolve; neural scalars are redrawn after edits. The JS name is stable.
#[wasm_bindgen]
pub fn random_control(seed: u64, params_json: Option<String>) -> Result<Sim, JsError> {
    Sim::with_brain_inheritance(
        seed,
        params_json.as_deref(),
        BrainInheritance::RandomizedAtBirth,
    )
}

/// Builds the structural null (spec §7.8): each child takes a random survivor's
/// topology with its parent's scalars where the two share genes, then mutates like an
/// evolving child. The browser can watch it beside the evolving world; like the native
/// shell, it records no history and saves no runs, whose formats carry the scalar
/// control.
#[wasm_bindgen]
pub fn structural_null(seed: u64, params_json: Option<String>) -> Result<Sim, JsError> {
    Sim::with_brain_inheritance(
        seed,
        params_json.as_deref(),
        BrainInheritance::StructuralNull,
    )
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
    /// Allocation generation per slot; changes when a dead slot is reused.
    incarnation: Span,
    /// Slots in the plant arrays. Fixed for the life of a world, like `capacity`.
    plant_capacity: u32,
    /// `x, y, z` per plant.
    plant_position: Span,
    /// What each site holds, so a fat plant draws differently from a bare one.
    plant_energy: Span,
    /// Slots in the corpse arrays. Fixed for the life of a world.
    corpse_capacity: u32,
    /// `x, y, z` per corpse slot.
    corpse_position: Span,
    /// What each corpse holds; 0 marks a free slot.
    corpse_energy: Span,
}

/// What a renderer needs that is not per-frame: the extent it is drawing into, and how
/// to draw a plant and a corpse.
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
    corpse_capacity: u32,
    corpse_radius: f32,
    corpse_signature: [f32; 3],
    /// A newborn's tank, against which a renderer shades how much a corpse holds.
    corpse_full_energy: f32,
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
    incarnation: u32,
    /// Decimal text because JSON numbers cannot represent every u64 exactly.
    tick: String,
    energy: f64,
    age: u32,
    size: f32,
    signature: [f32; 3],
    species_id: u32,
    birth_id: BirthId,
    parent_birth_a: BirthId,
    parent_birth_b: BirthId,
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
    spawn_failures: SpawnFailureCounts,
    structural_mutations: StructuralMutationCounts,
    species_events: SpeciesEventCounts,
    history: Option<CohortCapture>,
    history_enable_closed: bool,
}

#[derive(Serialize)]
struct StorageDiagnostics {
    arena_usage: [ArenaUsage; 5],
    spawn_failures: SpawnFailureCounts,
}

#[derive(Serialize)]
struct SpeciesPopulation {
    species_id: u32,
    population: u32,
}

#[derive(Serialize)]
struct SpeciesDiagnostics {
    populations: Vec<SpeciesPopulation>,
    unclassified_population: u32,
    events: SpeciesEventCounts,
}

impl Sim {
    fn with_brain_inheritance(
        seed: u64,
        params_json: Option<&str>,
        brain_inheritance: BrainInheritance,
    ) -> Result<Self, JsError> {
        let params = parse_params(params_json)?;
        let world = World::new_with_brain_inheritance(seed, params, brain_inheritance)
            .map_err(|e| js_error("world construction", e))?;
        Ok(Self::from_world(world))
    }

    /// Shell counters are observations of this `Sim`, so a restored world counts
    /// spawn refusals, edits, and species events from its load onward.
    fn from_world(world: World) -> Self {
        let snapshot = Snapshot::for_world(&world);
        Self {
            world,
            snapshot,
            spawn_failures: SpawnFailureCounts::default(),
            structural_mutations: StructuralMutationCounts::default(),
            species_events: SpeciesEventCounts::default(),
            history: None,
            history_enable_closed: false,
        }
    }
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
        Self::with_brain_inheritance(seed, params_json.as_deref(), BrainInheritance::Evolving)
    }

    /// Restores an untrusted checkpoint, paused at its saved tick (spec §7.10).
    ///
    /// `max_core_bytes` is this host's ceiling on the saved world's core budget. The
    /// founders are already placed, so callers must not seed this world again.
    pub fn restore(checkpoint: &[u8], max_core_bytes: u64) -> Result<Sim, JsError> {
        let limits = CheckpointLimits {
            max_bytes: checkpoint.len(),
            max_core_bytes,
        };
        let world =
            World::from_checkpoint(checkpoint, limits).map_err(|e| js_error("checkpoint", e))?;
        let mut sim = Self::from_world(world);
        sim.refresh();
        Ok(sim)
    }

    /// Encodes this world at its current between-ticks boundary, without advancing it.
    /// Allocates at the shell boundary and may detach snapshot views (§7.3).
    pub fn checkpoint(&self) -> Vec<u8> {
        self.world.checkpoint()
    }

    /// The seed this world was constructed from, including after a restore.
    pub fn seed(&self) -> u64 {
        self.world.seed()
    }

    /// The browser mode identifier for this world's heredity.
    pub fn heredity(&self) -> String {
        match self.world.brain_inheritance() {
            BrainInheritance::Evolving => "evolving",
            BrainInheritance::RandomizedAtBirth => "randomized_at_birth",
            BrainInheritance::StructuralNull => "structural_null",
        }
        .to_owned()
    }

    /// Seeds generation 0, reporting how many fit the pool and shared arenas.
    ///
    /// The layout belongs to `sim-core` — it is folded into every seeded run, so a
    /// browser and a headless sweep that arranged founders differently would not be
    /// running the same experiment. This is a direct call rather than a command because
    /// seeding is a boundary condition: there is no tick to stamp it for yet.
    pub fn seed_founders(&mut self, count: u32) -> u32 {
        // A zero-count or refused seed attempt still closes initial capture (§3.4).
        self.history_enable_closed = true;
        let counts = &mut self.spawn_failures;
        let species = &mut self.species_events;
        let placed = match &mut self.history {
            Some(capture) => self.world.seed_founders_with_history_observer(
                count,
                |error| counts.record(error),
                |event| species.record(event),
                |event, representative| capture.record(event, representative),
            ),
            None => self.world.seed_founders_with_observers(
                count,
                |error| counts.record(error),
                |event| species.record(event),
            ),
        };
        self.refresh();
        placed
    }

    /// Advances `ticks` ticks and refreshes the snapshot once, at the end.
    pub fn step_many(&mut self, ticks: u32) {
        self.history_enable_closed = true;
        let counts = &mut self.spawn_failures;
        let mutations = &mut self.structural_mutations;
        let species = &mut self.species_events;
        match &mut self.history {
            Some(capture) => {
                for _ in 0..ticks {
                    self.world.step_with_history_observer(
                        |error| counts.record(error),
                        |event| mutations.record(event),
                        |event| species.record(event),
                        |event, representative| capture.record(event, representative),
                    );
                }
            }
            None => {
                for _ in 0..ticks {
                    self.world.step_with_all_observers(
                        |error| counts.record(error),
                        |event| mutations.record(event),
                        |event| species.record(event),
                    );
                }
            }
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
            incarnation: span(self.snapshot.incarnation()),
            plant_capacity: self.snapshot.plant_capacity(),
            plant_position: span(self.snapshot.plant_position()),
            plant_energy: span(self.snapshot.plant_energy()),
            corpse_capacity: self.snapshot.corpse_capacity(),
            corpse_position: span(self.snapshot.corpse_position()),
            corpse_energy: span(self.snapshot.corpse_energy()),
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
            corpse_capacity: self.snapshot.corpse_capacity(),
            corpse_radius: params.corpses.radius,
            corpse_signature: params.corpses.signature,
            corpse_full_energy: params.reproduction.start_energy,
        };
        serde_json::to_string(&hints).map_err(|e| js_error("render hints", e))
    }

    pub fn tick(&self) -> u64 {
        self.world.tick_count()
    }

    pub fn population(&self) -> u32 {
        self.world.population()
    }

    pub fn species_count(&self) -> u32 {
        self.world.species_count()
    }

    pub fn unclassified_population(&self) -> u32 {
        self.world.unclassified_population()
    }

    /// Living agents born in this world; persistent founders are deliberately excluded.
    pub fn descendants(&self) -> u32 {
        self.world.living_descendants()
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

    /// On-demand arena element usage and cumulative spawn refusals since construction.
    ///
    /// Counts allocate nothing during seeding or stepping. This JSON request can grow
    /// WASM memory, so consumers must refresh detached snapshot views (spec §7.3).
    /// The core budget excludes this shell's snapshot, transports, and paired worlds;
    /// these diagnostics are not a browser resident-memory safety guarantee.
    pub fn storage_diagnostics(&self) -> Result<String, JsError> {
        let diagnostics = StorageDiagnostics {
            arena_usage: self.world.storage_usage(),
            spawn_failures: self.spawn_failures,
        };
        serde_json::to_string(&diagnostics).map_err(|e| js_error("storage diagnostics", e))
    }

    /// Cumulative structural candidate edits since construction, separate from births
    /// and spawn refusals. Only successful positive-rate gates count as attempts.
    ///
    /// Applied candidates can still fail to spawn; these counts do not measure live
    /// complexity. Like other JSON requests, this can detach snapshot views (§7.3).
    pub fn structural_mutation_diagnostics(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.structural_mutations)
            .map_err(|e| js_error("structural mutation diagnostics", e))
    }

    /// On-demand populations in historical ID order and lifecycle counts since construction.
    ///
    /// IDs identify species only within this world (spec §3.4). Unclassified agents
    /// are separate, not a species. This allocating request can detach snapshot views.
    pub fn species_diagnostics(&self) -> Result<String, JsError> {
        let diagnostics = SpeciesDiagnostics {
            populations: self
                .world
                .species()
                .active()
                .map(|(id, population)| SpeciesPopulation {
                    species_id: id.raw(),
                    population,
                })
                .collect(),
            unclassified_population: self.world.unclassified_population(),
            events: self.species_events,
        };
        serde_json::to_string(&diagnostics).map_err(|e| js_error("species diagnostics", e))
    }

    /// On-demand exact live genome-size distributions, identical to native metrics'
    /// `complexity` at the same completed tick. Observation only, never a score.
    /// Like other JSON requests, this allocates and can detach snapshot views (§7.3).
    pub fn complexity_diagnostics(&self) -> Result<String, JsError> {
        serde_json::to_string(&complexity::sample_complexity(&self.world))
            .map_err(|e| js_error("complexity diagnostics", e))
    }

    /// Retunes the world. Errors on anything that would resize what is already
    /// allocated — see `World::set_params` for which fields those are and why.
    pub fn set_params(&mut self, params_json: &str) -> Result<(), JsError> {
        let params: SimParams =
            serde_json::from_str(params_json).map_err(|e| js_error("bad params", e))?;
        self.world
            .set_params(params)
            .map_err(|e| js_error("rejected params", e))
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
    /// kilobytes per agent against the snapshot's 61 bytes, and it is read at the rate a
    /// human clicks (spec §2.2b).
    pub fn inspect_agent(&self, index: u32, incarnation: u32) -> Result<String, JsError> {
        let id = AgentId::new(index);
        if index >= self.world.pool().capacity() || !self.world.pool().is_alive(id) {
            return Err(JsError::new(&format!("no live agent at slot {index}")));
        }
        if self.world.pool().incarnation(id) != Some(incarnation) {
            return Err(JsError::new(&format!(
                "agent at slot {index} has been replaced"
            )));
        }
        let agents = self.world.agents();
        let i = id.index();
        let inspection = Inspection {
            index,
            incarnation,
            tick: self.world.tick_count().to_string(),
            energy: agents.energy[i] as f64 + agents.energy_reserve[i],
            age: agents.age[i],
            size: agents.size[i],
            signature: agents.signature[i].to_array(),
            species_id: agents.species_id[i],
            birth_id: agents.birth_id[i],
            parent_birth_a: agents.parent_birth_a[i],
            parent_birth_b: agents.parent_birth_b[i],
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
