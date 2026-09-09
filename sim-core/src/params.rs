//! Every tunable constant in the simulation, in one serde-serializable struct.
//!
//! Nothing here is a compile-time constant. Tuning happens in the browser and in
//! headless sweeps, not in the compiler, so a number someone might want to twiddle
//! belongs in this file even when it currently has one plausible value (spec §7.6).
//! Starting values and — more importantly — the *relationship* each one has to the
//! others come from spec §5.5.
//!
//! Deliberately not here: anything derived rather than chosen. Spatial-hash cell size
//! is a method on [`SensingParams`], not a field, because a field could disagree with
//! the sensor ranges it must cover.
//!
//! Deliberately absent: constants for phases that do not exist yet. Predation
//! (`attack_cost`, `corpse_energy_fraction`) and speciation thresholds arrive with the
//! systems that read them.

use serde::{Deserialize, Serialize};

use crate::genome;
use crate::storage::StorageLayout;

/// The complete parameter set for one world.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimParams {
    pub world: WorldParams,
    pub storage: StorageParams,
    pub body: BodyParams,
    pub metabolism: MetabolismParams,
    pub movement: MovementParams,
    pub sensing: SensingParams,
    pub brain: BrainParams,
    pub reproduction: ReproductionParams,
    pub mutation: MutationParams,
    pub distance: DistanceParams,
    pub species: SpeciesParams,
    pub feeding: FeedingParams,
    pub plants: PlantParams,
    pub chemo: ChemoParams,
}

/// Typed-gene distance coefficients (spec §3.4), not a species or fitness policy.
///
/// Frozen with species configuration for the life of a world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DistanceParams {
    pub disjoint_coefficient: f32,
    pub excess_coefficient: f32,
    pub weight_coefficient: f32,
}

impl Default for DistanceParams {
    fn default() -> Self {
        Self {
            disjoint_coefficient: 1.0,
            excess_coefficient: 1.0,
            weight_coefficient: 0.4,
        }
    }
}

impl DistanceParams {
    /// Also usable at boundaries that compare genomes without constructing a world.
    pub fn validate(&self) -> Result<(), ParamError> {
        if [
            self.disjoint_coefficient,
            self.excess_coefficient,
            self.weight_coefficient,
        ]
        .iter()
        .any(|&coefficient| !coefficient.is_finite() || coefficient < 0.0)
        {
            return Err(ParamError(
                "distance coefficients must be finite and non-negative",
            ));
        }
        Ok(())
    }
}

/// Construction-time species classification policy (spec §3.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeciesParams {
    /// Maximum active immutable representatives. Zero makes every admission unclassified.
    pub capacity: u32,
    /// Provisional measurement scale, not a calibrated biological species boundary.
    ///
    /// Departs from §5.5's provisional 3.0, which exceeds the maximum structural-only
    /// distance of 2 at the starting coefficients. Approved at 0.5 for M4 integration;
    /// deletion-default calibration and ecological interpretation remain M8 work.
    pub threshold: f64,
}

impl Default for SpeciesParams {
    fn default() -> Self {
        Self {
            capacity: 256,
            threshold: 0.5,
        }
    }
}

/// Construction-time storage policy (spec §2.2a).
///
/// Per-slot allowances multiply `world.max_agents` to size shared arenas. They are
/// pooled across organisms, not per-organism strides, and never depend on founder
/// composition. Individual genomes may exceed an allowance while staying within
/// their separate `max_*` limits and the available aggregate storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageParams {
    pub genes_per_slot: u32,
    pub neurons_per_slot: u32,
    pub synapses_per_slot: u32,
    pub sensors_per_slot: u32,
    pub effectors_per_slot: u32,
    /// Maximum genes in one organism, also sizing the reusable genome scratch.
    pub max_genes: u32,
    /// Maximum neurons in one organism, also sizing the reusable fan-in scratch.
    pub max_neurons: u32,
    /// Maximum connection genes, including disabled connections.
    pub max_connections: u32,
    pub max_sensors: u32,
    pub max_vision_rays: u32,
    pub max_effectors: u32,
    /// Portable upper bound on cumulative requested heap bytes for one core construction.
    ///
    /// Includes eager buffers, metadata, templates and constructor temporaries.
    /// Excludes shell snapshots/transports, a second control world, and allocator/OS
    /// overhead; passing validation does not guarantee host RAM is available.
    pub max_memory_bytes: u64,
}

impl Default for StorageParams {
    fn default() -> Self {
        Self {
            genes_per_slot: 284,
            neurons_per_slot: 28,
            synapses_per_slot: 240,
            sensors_per_slot: 5,
            effectors_per_slot: 4,
            max_genes: 1_024,
            max_neurons: 128,
            max_connections: 1_024,
            max_sensors: 32,
            max_vision_rays: 32,
            max_effectors: 4,
            max_memory_bytes: 100_663_296,
        }
    }
}

/// Extent, capacity, and the fixed timestep.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldParams {
    /// Side length of the square world, in world units. Wraps at the edges.
    pub size: f32,
    /// Agent pool capacity. Pre-allocated at startup and never grown — growing WASM
    /// memory detaches every JS view over the snapshot (spec §7.3).
    pub max_agents: u32,
    /// Seconds of sim time per tick. 60 ticks/sec of sim time (spec §2.1).
    pub dt: f32,
    /// How far generation 0 spreads from the centre, as a fraction of [`Self::size`].
    ///
    /// A real tunable rather than a layout detail: it sets the initial density, and
    /// density decides whether founders start close enough for kin selection to have
    /// anything to act on and near enough to plants to eat. 0.4 fills most of the world
    /// without piling everything on the seam.
    pub founder_spread: f32,
}

/// Fixed body traits. Genetic from Phase 2; one value for everyone in Phase 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyParams {
    /// Collision and rendering radius, in world units.
    pub size: f32,
}

/// Per-tick energy costs. Spec §5.2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetabolismParams {
    /// Flat upkeep. Idling must be fatal within ~2000 ticks on a full tank, or
    /// sitting still is a viable strategy and nothing evolves (spec §10).
    ///
    /// That ~2000 is an upper bound on *total* idle lifetime, not a target for this
    /// term alone. After the Phase 1 acceptance tuning, the default topology and body
    /// pay:
    ///
    /// | term | per tick | vs base |
    /// |---|---|---|
    /// | `base` | 0.050 | 1.0× |
    /// | `k_brain` × 268 units | 0.013 | 0.27× |
    /// | `k_sensor` × 16 units | 0.010 | 0.20× |
    /// | `k_size` × size² | 0.011 | 0.23× |
    /// | total | 0.085 | — |
    ///
    /// A full idle tank therefore lasts ~1,181 ticks: long enough to encounter food and
    /// reach maturity, still well inside the ~2,000-tick ceiling. The previous 0.403
    /// total killed every tested seed before selection accumulated; see the M12 record
    /// in `docs/phase-1-implementation-plan.md`.
    pub base: f32,
    /// Cost coefficient on size². Doubling radius should roughly quadruple upkeep.
    ///
    /// **0.00125, deliberately below the original design guess of 0.02.** The body
    /// radius is 3 rather than 1, so 0.02 made this term 3.6× base by itself. The
    /// tuned value prices the default body at 0.01125/tick, enough to keep size costly
    /// once it becomes evolvable without making the fixed Phase 1 body fatal.
    pub k_size: f32,
    /// Cost per neuron and per connection — see `genome::brain_complexity`, and note
    /// it is *not* per gene: sensors are billed by `k_sensor` instead.
    ///
    /// The term that stops genomes bloating without limit. It does no selective work
    /// until Phase 2, because every Phase 1 genome is the same size, so today it only
    /// adds to `base`.
    ///
    /// **0.00005, deliberately not the original 0.001 design guess.** A 200-unit brain
    /// at "~20% of base" means the term should land near 0.01 when `base` is 0.05 —
    /// but 200 × 0.001 is 0.2, four times base rather than a fifth of it. Taken
    /// literally it kills an idle agent in ~300 ticks against the ~2000 ceiling.
    /// 0.00005 satisfies both relationships at once.
    pub k_brain: f32,
    /// Cost per sensor, weighted by channel count — see `genome::sensor_load`. An eye
    /// returning distance and colour costs four times a single-channel interoceptor,
    /// and metering perception is what makes evolution pay for its own compute
    /// (spec §2.2c).
    ///
    /// Weighting by channels rather than per-sensor is this codebase's reading of
    /// §5.5's "weighted by modality". **0.000625 makes the 16-channel founding suite
    /// cost 0.01/tick in total.** Eyes remain four times as expensive as a
    /// single-channel interoceptor, while perception no longer costs 3.2× base before
    /// an agent moves.
    pub k_sensor: f32,
    /// Cost coefficient on |force|². Sprinting should drain a full tank in ~200 ticks.
    ///
    /// With the M12 budget, full thrust costs 0.585/tick against 0.085 at idle: idling
    /// buys about 6.9× the lifetime, up from 2.2× before the other metabolic terms were
    /// tuned down. The accepted populations still had to forage before reproducing, but
    /// this ratio is the one to watch with `stable_but_idle` when Phase 2 makes brain
    /// structure evolvable (spec §10).
    pub k_move: f32,
}

/// Locomotion limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MovementParams {
    /// Force at full thrust output.
    pub max_thrust: f32,
    /// Radians per second at full turn output.
    pub max_turn_rate: f32,
    /// Velocity retained per second. Below 1 the world is viscous, which keeps
    /// ballistic drifting from beating steering.
    pub drag: f32,
    /// Speed ceiling, so a runaway brain output cannot tunnel through the hash.
    pub max_speed: f32,
}

/// Sensor ranges and resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SensingParams {
    /// How far a vision ray reaches.
    pub vision_range: f32,
    /// Full cone width of one ray, radians.
    pub vision_fov: f32,
    /// Founder vision-ray count; evolved ray limits live in `StorageParams`.
    /// Perception remains a metered metabolic cost (spec §2.2c).
    pub vision_rays: u32,
    /// Founder food-chemoreceptor count, not an evolved-organ capacity.
    pub chemo_sensors: u32,
    /// Founder energy-interoceptor count, not an evolved-organ capacity.
    pub energy_sensors: u32,
    /// Radius over which the chemo sensor samples concentration and gradient.
    pub chemo_radius: f32,
}

impl SensingParams {
    /// Spatial-hash cell size: the largest radius any sensor can query.
    ///
    /// Derived rather than configured, because a cell smaller than the longest sensor
    /// silently truncates neighbour queries — the fast path stops matching the
    /// brute-force reference and every agent goes half-blind (spec §2.3).
    pub fn max_sense_radius(&self) -> f32 {
        self.vision_range.max(self.chemo_radius)
    }
}

/// Founder CTRNN topology and neural scalar initialization.
/// Descendant topology evolves independently of these starting counts (spec §3.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrainParams {
    /// Hidden neurons in the founding template.
    pub hidden_neurons: u32,
    /// Always-present oscillator neurons. Cheap scaffold — evolution finds them fast
    /// and builds gaits and timing on them (spec §3.2).
    pub oscillators: u32,
    /// Founder incoming connections per hidden/output target. None means dense;
    /// a finite count selects distinct sources once per world, shared by all founders.
    pub connections_per_target: Option<u32>,
    /// Inclusive range for a neuron's time constant. Small tau reacts, large tau
    /// integrates; the spread is what gives the brain a memory of any length.
    pub tau_min: f32,
    pub tau_max: f32,
    /// Inclusive range for an oscillator's period, in ticks.
    pub oscillator_period_min: f32,
    pub oscillator_period_max: f32,
    /// Half-width of the range a founder's connection weights are drawn from, **before
    /// division by the square root of the target neuron's fan-in** (`founder`).
    ///
    /// Deliberately not [`MutationParams::weight_limit`]. That is a *bound* on where
    /// evolution may take a weight over a lineage; this is the scale one should start
    /// at, and the two differ by the fan-in factor. Drawing from the full ±4 bound with
    /// the default topology's 24 inputs per neuron sums to order ±20 before the sigmoid
    /// sees it, so every founder saturates on tick one, cannot respond to its sensors,
    /// and weight mutation cannot walk it back out in any useful time. It looks like a
    /// tuning failure and is an initialisation bug — `brain`'s
    /// `without_fan_in_scaling_a_founder_saturates` is the guard.
    ///
    /// At ±2 over √24 a founder's summed input lands near ±1, which is the band where
    /// the sigmoid still has a gradient. Not from spec §5.5, which does not name a
    /// weight-initialisation scale.
    pub weight_init_scale: f32,
}

/// Asexual budding with spatial viscosity. Spec §5.4.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReproductionParams {
    /// Founders' initial reserve and the reference one-tank energy used by sensors.
    ///
    /// Offspring receive a fraction of their parent's actual energy rather than a free
    /// grant. At the tuned default threshold and 50/50 split, a marginal birth leaves
    /// both parent and child with this 100-energy reserve.
    pub start_energy: f32,
    /// Energy floor below which the reproduce effector does nothing. Must sit above
    /// [`Self::start_energy`], so growth is required before breeding — `validate`
    /// enforces that relationship rather than leaving it to a comment (spec §5.5).
    ///
    /// **200 rather than the original 150:** splitting 150 made two 75-energy agents,
    /// so reproduction itself pushed both below the configured starting reserve. At
    /// 200, frequent reproduction still has its intended cost—half the parent's stored
    /// energy—but does not create two already-marginal lives.
    pub threshold: f32,
    /// Effector output above which the brain is asking to reproduce. Brain-gated, not
    /// automatic at a threshold — life-history strategy is meant to be evolvable
    /// (spec §4.2).
    pub gate: f32,
    /// Fraction of the parent's energy the offspring receives.
    pub energy_split: f32,
    /// How far from the parent an offspring may spawn.
    ///
    /// Load-bearing and small on purpose: offspring near parents means neighbours are
    /// relatives, which is what makes kin selection operate and communication
    /// evolvable. Random placement quietly makes cooperation impossible (spec §5.4).
    pub spawn_radius: f32,
    /// Ticks after birth before an agent may reproduce.
    pub maturity_ticks: u32,
}

/// Mutation operators. Phase 1 perturbs and resets scalars; topology operators arrive
/// in Phase 2 (spec §3.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MutationParams {
    pub structural: StructuralMutationParams,
    pub organs: OrganMutationParams,
    /// Per-connection chance of a Gaussian nudge.
    ///
    /// **0.025 rather than the original 0.8.** With 240 founding connections, 0.8
    /// changed roughly 192 weights at every birth and erased useful behavior faster
    /// than selection could retain it. This rate changes about six.
    pub weight_perturb_rate: f32,
    /// Standard deviation of that nudge.
    pub weight_perturb_sigma: f32,
    /// Per-connection chance of a uniform resample instead. The default 0.0015625
    /// produces about 0.375 resets per 240-connection birth; the original 0.05
    /// produced about twelve.
    pub weight_reset_rate: f32,
    /// Bound on connection weights, for reset and clamping.
    pub weight_limit: f32,
    /// Per-neuron chance of a Gaussian nudge to bias, tau, or oscillator period.
    /// Without this the oscillator period in [`BrainParams`] is fixed for all time,
    /// and spec §3.2's "evolvable period" is not true of Phase 1.
    ///
    /// The 0.00625 default changes about 0.175 of the 28 founding neurons per birth,
    /// retaining useful dynamics while still giving long runs variation to select.
    pub neuron_perturb_rate: f32,
    pub bias_perturb_sigma: f32,
    /// Multiplicative, so tau explores across orders of magnitude rather than
    /// random-walking off the bottom of its range.
    pub tau_perturb_factor: f32,
}

/// Per-offspring structural edits, disabled by default while preserving the accepted
/// scalar-only baseline. Physical removal remains opt-in pending distance calibration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StructuralMutationParams {
    /// Physical deletion loses the edge's innovation marker. Re-adding the same
    /// endpoints gets a fresh ID, so distance can increase without changed wiring.
    /// Keep the shipped default at zero until D4 calibrates distance coefficients
    /// and the species threshold against deletion/recreation (spec §3.4).
    /// Explicit calibration runs may opt in before changing the default.
    pub remove_connection_rate: f32,
    /// Removing/rebuilding a neuron also replaces its incident-edge innovation
    /// history. Keep the shipped default at zero until D4's coefficients and
    /// threshold are calibrated against this marker turnover (spec §3.4);
    /// nonzero rates remain available for explicit calibration runs.
    pub remove_neuron_rate: f32,
    pub toggle_connection_rate: f32,
    pub add_connection_rate: f32,
    pub add_neuron_rate: f32,
    pub split_neuron_bias: f32,
    /// Initial incoming weight when splitting an edge; outgoing weight is inherited.
    pub split_input_weight: f32,
}

impl Default for StructuralMutationParams {
    fn default() -> Self {
        Self {
            remove_connection_rate: 0.0,
            remove_neuron_rate: 0.0,
            toggle_connection_rate: 0.0,
            add_connection_rate: 0.0,
            add_neuron_rate: 0.0,
            split_neuron_bias: 0.0,
            split_input_weight: 1.0,
        }
    }
}

impl StructuralMutationParams {
    pub fn is_disabled(&self) -> bool {
        self.remove_connection_rate == 0.0
            && self.remove_neuron_rate == 0.0
            && self.toggle_connection_rate == 0.0
            && self.add_connection_rate == 0.0
            && self.add_neuron_rate == 0.0
    }
}

/// Sensor edits, disabled by default to preserve existing runs (spec section 3.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OrganMutationParams {
    pub remove_sensor_rate: f32,
    pub add_sensor_rate: f32,
    pub vision_weight: f32,
    pub chemo_weight: f32,
    pub energy_weight: f32,
    /// Initial bias of fresh sensor-target neurons; tau uses the configured brain range.
    pub neuron_bias: f32,
}

impl Default for OrganMutationParams {
    fn default() -> Self {
        Self {
            remove_sensor_rate: 0.0,
            add_sensor_rate: 0.0,
            vision_weight: 1.0,
            chemo_weight: 1.0,
            energy_weight: 1.0,
            neuron_bias: 0.0,
        }
    }
}

impl OrganMutationParams {
    pub fn is_disabled(&self) -> bool {
        self.remove_sensor_rate == 0.0 && self.add_sensor_rate == 0.0
    }
}

/// Eating. What an agent can draw from a plant it is touching, and when it asks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeedingParams {
    /// Energy per tick an agent draws from the plant it is touching, at full drive.
    ///
    /// Per tick, matching [`MetabolismParams`], so the two can be compared directly:
    /// this has to beat upkeep by enough that foraging pays, or eating is a way to
    /// starve more slowly rather than a way to live (spec §5.1).
    pub rate: f32,
    /// Effector output above which the brain is asking to eat.
    pub gate: f32,
    /// How far past its own body an agent can reach, added to its size and the plant's
    /// radius. Zero means it has to be in contact.
    ///
    /// **4 world units after M12 tuning.** The resulting nine-unit capture radius is
    /// still local relative to vision and chemo range, but gives a steering agent more
    /// than one tick to feed as it crosses a fixed plant. Zero-reach populations went
    /// extinct across the acceptance seeds even after the metabolic budget was viable.
    pub reach: f32,
}

impl Default for FeedingParams {
    fn default() -> Self {
        Self {
            rate: 1.0,
            gate: 0.5,
            reach: 4.0,
        }
    }
}

/// The autotroph base. Non-brained entities that hold the energy entering the world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlantParams {
    /// Total energy entering the world per second of sim time.
    ///
    /// The one knob for a population crash. Do not add free energy anywhere else to
    /// fix one — a leak makes selection stop being real (spec §5.1).
    ///
    /// **12,000 after M12 tuning.** At 60 ticks/s this offers at most 200 energy/tick,
    /// close to the minimum upkeep of the web profile's 2,000 founders. Plant caps mean
    /// unused supply never enters, while the observed carrying regime remains far below
    /// the agent ceiling and the random-brain control collapses to a few agents.
    pub energy_input_rate: f32,
    /// Plant pool capacity.
    pub max_plants: u32,
    /// Energy at which a plant stops growing.
    pub max_energy: f32,
    /// Collision and ingest radius.
    pub radius: f32,
    /// Concentration deposited into chemo channel 0 per unit of stored energy, per
    /// second. This is what gives the chemo sensor a food gradient to climb in
    /// Phase 1, before any agent can emit anything.
    pub scent_rate: f32,
    /// Fraction of [`Self::max_energy`] each plant holds when the world is created.
    ///
    /// **Defaults full; empty was a bug rather than a choice.** Under the pre-M12
    /// budget, filling a bare larder took ~24,000 ticks against a founder lifetime of
    /// ~250, so generation 0 starved in a world with no food in it yet. That reads as
    /// a foraging failure and invites weakening a metabolic cost (spec §10). Measured
    /// in `docs/phase-1-implementation-plan.md` under M7.
    ///
    /// Filled uniformly and without drawing from `rng`: a random fill would shift every
    /// genome scalar drawn after it.
    pub initial_fill: f32,
    /// The colour a `vision_ray` reports when it hits a plant.
    ///
    /// **Plants are visible as well as smellable — decided here, at M7.** The
    /// alternative was not neutral. `k_sensor` charges by channel, so the default
    /// sensor set is 12 units of eye against 4 of everything else; there is no
    /// predation until Phase 3 to make seeing another agent worth anything, and Phase 1
    /// has no remove-sensor operator, so selection could not have shed the useless eyes
    /// for the whole of the phase §8's criterion is judged in. Every agent would have
    /// paid for three organs that see nothing, and it would have surfaced during the
    /// metabolic tuning pass looking like a `k_sensor` problem rather than a missing
    /// query — the kind of symptom that gets "fixed" by weakening a metabolic cost,
    /// which is how a simulation quietly stops selecting for anything (spec §10).
    ///
    /// Visible gives two independent routes to food, so an M12 run shows which one
    /// evolution finds first, and makes the signature channel meaningful immediately:
    /// green means food is learnable on day one, on the same machinery spec §4.2 wants
    /// for aposematism in Phase 4.
    ///
    /// An emptied plant stays visible — the site persists and regrows, so blinking it
    /// out would be stranger than leaving it. Telling a fat plant from a bare one needs
    /// the nose, which is a selective pressure worth having rather than a defect.
    pub signature: [f32; 3],
}

/// Pheromone field. A 3D grid of depth 1 in V1 (spec §9.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChemoParams {
    /// Grid resolution `[x, y, z]`. `z` is 1 in V1 and the loops are still 3D.
    pub cells: [u32; 3],
    /// Retained fraction per tick, per channel. Per-channel on purpose: a trail and an
    /// alarm want different half-lives (spec §5.5).
    pub decay: Vec<f32>,
    /// Fraction of the way each cell moves toward the average of its neighbours per
    /// tick. Too high and every gradient flattens to zero.
    ///
    /// **0.5 is the useful ceiling, not the 1.0 validation allows.** This is an explicit
    /// scheme, so it carries the stability limit every explicit scheme does: the
    /// checkerboard mode is scaled by `1 - 2·diffuse` each tick, which damps fastest at
    /// 0.5, damps more slowly above it, and at exactly 1.0 flips sign forever without
    /// shrinking. A point deposit is full of that mode, so at 1.0 the field keeps a
    /// permanent grid artefact that reads as noise a nose will chase. Validation stops
    /// at [0, 1] because that is where the arithmetic stays bounded; going above 0.5 is
    /// a tuning mistake rather than an unsafe value.
    pub diffuse: f32,
}

impl ChemoParams {
    pub fn channels(&self) -> usize {
        self.decay.len()
    }
}

/// A parameter set that cannot produce a coherent world.
///
/// Validation happens once, at the boundary where params arrive from JS or a CLI. Past
/// that point the tick treats these invariants as established and uses
/// `debug_assert!` rather than threading `Result` through the hot loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamError(pub &'static str);

impl core::fmt::Display for ParamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid SimParams: {}", self.0)
    }
}

impl core::error::Error for ParamError {}

impl SimParams {
    /// Checked portable upper bound on cumulative core-construction heap requests.
    ///
    /// Includes constructor temporaries and the owned parameter channel buffer, not
    /// just retained world buffers. Invalid configurations, including those exceeding
    /// `storage.max_memory_bytes`, return an error before any world allocation.
    /// Temporary pointer-sized indices are charged at 8 bytes on native and WASM.
    /// Shell allocations and allocator/OS overhead are outside this estimate.
    pub fn estimated_construction_bytes(&self) -> Result<u64, ParamError> {
        self.validate_values()?;
        Ok(StorageLayout::new(self)?.construction_bytes)
    }

    /// Whether `next` may replace these params on a world already running, given the
    /// spatial grid's `grid_cell` extent.
    ///
    /// **Editable — nearly everything.** Every field of `body`, `metabolism`,
    /// `movement`, `mutation`, `feeding`, `reproduction`, `chemo.decay`,
    /// `chemo.diffuse`, `world.dt`, `world.founder_spread`, `plants` other than the two
    /// below, `sensing` ranges within the limit below, and the `brain` fields that are
    /// not topology (`tau_min`, `tau_max`, the oscillator periods, `weight_init_scale`).
    /// That is the whole point of spec §7.6: tuning happens in the browser against a
    /// running population, not in the compiler.
    ///
    /// **Frozen, because they size something already allocated:**
    ///
    /// | Field | What it sized |
    /// |---|---|
    /// | `world.max_agents` | the slot pool, every SoA array, all six arenas, the snapshot |
    /// | `storage` | aggregate arenas, per-genome scratch and construction budget |
    /// | `species`, `distance` | representative storage and stable classification meaning |
    /// | `world.size` | the spatial grid's extent and the chemo field's |
    /// | `plants.max_plants` | the plant arrays and their neighbour grid |
    /// | `chemo.cells` | the field's cell arrays |
    /// | `chemo.decay.len()` | the number of allocated field channels |
    /// | founder sensor counts, `brain.hidden_neurons`, `brain.oscillators`, `brain.connections_per_target` | the founding template and its fan-in scales |
    ///
    /// **Frozen, because it would silently do nothing:** `plants.initial_fill`, which is
    /// read once when the larder is stocked. Refusing is the honest answer for all of
    /// these — accepting a value that changes nothing makes the inspector disagree with
    /// the sim, and honouring one would move memory that JS holds views over (spec §7.3).
    ///
    /// **Sensing ranges may shrink but not grow.** `vision_range` and `chemo_radius`
    /// below the grid's cell size are fine: the cells are then larger than they need to
    /// be, which costs a little time and stays correct. Above it, a neighbour query
    /// would walk one ring of cells and miss agents beyond it — silently, and it would
    /// read as a sensor bug rather than a params one (spec §2.3).
    ///
    /// Written here rather than on `World` because none of it is about a world: it is a
    /// property of two `SimParams` and one number, which is what makes it testable
    /// without building a simulation to ask.
    ///
    /// The current params belong to an already validated world. Retuning preserves
    /// its allocated grids, so a smaller sensing radius must not be charged for a
    /// hypothetical finer grid in a new world.
    pub fn check_retune(&self, next: &SimParams, grid_cell: f32) -> Result<(), ParamError> {
        next.validate_values()?;

        for (changed, message) in [
            (
                next.species != self.species || next.distance != self.distance,
                "species and distance are fixed for the life of a world",
            ),
            (
                next.world.max_agents != self.world.max_agents,
                "world.max_agents is fixed for the life of a world",
            ),
            (
                next.storage != self.storage,
                "storage is fixed for the life of a world",
            ),
            (
                next.world.size != self.world.size,
                "world.size is fixed for the life of a world",
            ),
            (
                next.plants.max_plants != self.plants.max_plants,
                "plants.max_plants is fixed for the life of a world",
            ),
            (
                next.chemo.cells != self.chemo.cells,
                "chemo.cells is fixed for the life of a world",
            ),
            (
                next.chemo.channels() != self.chemo.channels(),
                "chemo channel count is fixed for the life of a world",
            ),
            (
                next.sensing.vision_rays != self.sensing.vision_rays
                    || next.sensing.chemo_sensors != self.sensing.chemo_sensors
                    || next.sensing.energy_sensors != self.sensing.energy_sensors
                    || next.brain.hidden_neurons != self.brain.hidden_neurons
                    || next.brain.oscillators != self.brain.oscillators
                    || next.brain.connections_per_target != self.brain.connections_per_target,
                "the founding topology is fixed for the life of a world",
            ),
            (
                next.plants.initial_fill != self.plants.initial_fill,
                "plants.initial_fill is read once, when the larder is stocked",
            ),
        ] {
            if changed {
                return Err(ParamError(message));
            }
        }

        if next.sensing.max_sense_radius() > grid_cell {
            return Err(ParamError(
                "sensing radius would outgrow the spatial grid built for this world",
            ));
        }
        Ok(())
    }

    /// Validates scalar values and the storage needed to construct a new world.
    pub fn validate(&self) -> Result<(), ParamError> {
        self.validate_values()?;
        StorageLayout::new(self)?;
        Ok(())
    }

    /// `!(x > 0.0)` rather than `x <= 0.0` throughout: the negated form also rejects
    /// NaN, which is the shape a bad value arrives in from JSON.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn validate_values(&self) -> Result<(), ParamError> {
        self.distance.validate()?;
        if !self.species.threshold.is_finite() || self.species.threshold <= 0.0 {
            return Err(ParamError("species.threshold must be finite and positive"));
        }
        if !(self.world.size > 0.0) || !self.world.size.is_finite() {
            return Err(ParamError("world.size must be finite and positive"));
        }
        if !(self.world.dt > 0.0) || !self.world.dt.is_finite() {
            return Err(ParamError("world.dt must be finite and positive"));
        }
        if !(self.body.size >= 0.0) || !self.body.size.is_finite() {
            return Err(ParamError("body.size must be finite and non-negative"));
        }
        if self.world.max_agents == 0 {
            return Err(ParamError("world.max_agents must be non-zero"));
        }
        if !(self.plants.energy_input_rate >= 0.0) || !self.plants.energy_input_rate.is_finite() {
            return Err(ParamError(
                "plants.energy_input_rate must be finite and non-negative",
            ));
        }
        if !(self.plants.max_energy >= 0.0) || !self.plants.max_energy.is_finite() {
            return Err(ParamError(
                "plants.max_energy must be finite and non-negative",
            ));
        }
        if !(self.plants.max_energy * self.plants.max_plants as f32).is_finite() {
            return Err(ParamError(
                "plants.max_energy times max_plants exceeds the finite energy ledger",
            ));
        }
        if [self.plants.radius, self.plants.scent_rate]
            .iter()
            .any(|&value| !(value >= 0.0) || !value.is_finite())
        {
            return Err(ParamError(
                "plant radius and scent rate must be finite and non-negative",
            ));
        }
        if [
            self.sensing.vision_range,
            self.sensing.vision_fov,
            self.sensing.chemo_radius,
        ]
        .iter()
        .any(|&value| !(value >= 0.0) || !value.is_finite())
        {
            return Err(ParamError(
                "sensing ranges and field of view must be finite and non-negative",
            ));
        }
        if !(self.sensing.max_sense_radius() > 0.0) {
            return Err(ParamError("sensing radii must be positive"));
        }
        if self.sensing.max_sense_radius() * 2.0 > self.world.size {
            return Err(ParamError(
                "sense radius exceeds half the world; the hash cannot wrap",
            ));
        }
        if !genome::valid_tau(self.brain.tau_min)
            || !genome::valid_tau(self.brain.tau_max)
            || self.brain.tau_max < self.brain.tau_min
        {
            return Err(ParamError(
                "brain.tau range must be ordered and have finite positive reciprocals",
            ));
        }
        if !genome::valid_oscillator_period(self.brain.oscillator_period_min)
            || !genome::valid_oscillator_period(self.brain.oscillator_period_max)
            || self.brain.oscillator_period_max < self.brain.oscillator_period_min
        {
            return Err(ParamError(
                "brain.oscillator_period range must be ordered and produce finite positive phase increments",
            ));
        }
        if !(self.brain.weight_init_scale > 0.0) || !self.brain.weight_init_scale.is_finite() {
            return Err(ParamError("brain.weight_init_scale must be positive"));
        }
        if !(self.reproduction.start_energy > 0.0) || !self.reproduction.start_energy.is_finite() {
            return Err(ParamError(
                "reproduction.start_energy must be finite and positive",
            ));
        }
        if !self.reproduction.threshold.is_finite()
            || self.reproduction.threshold <= self.reproduction.start_energy
        {
            return Err(ParamError(
                "reproduction.threshold must be finite and exceed start_energy, or breeding needs no growth",
            ));
        }
        if !self.reproduction.gate.is_finite() || !self.feeding.gate.is_finite() {
            return Err(ParamError("reproduction and feeding gates must be finite"));
        }
        if !(0.0..=1.0).contains(&self.reproduction.energy_split) {
            return Err(ParamError("reproduction.energy_split must be in [0, 1]"));
        }
        if !(self.reproduction.spawn_radius >= 0.0) || !self.reproduction.spawn_radius.is_finite() {
            return Err(ParamError(
                "reproduction.spawn_radius must be finite and non-negative",
            ));
        }
        if !(self.feeding.rate >= 0.0) || !self.feeding.rate.is_finite() {
            return Err(ParamError("feeding.rate must be non-negative"));
        }
        if !(self.feeding.reach >= 0.0) || !self.feeding.reach.is_finite() {
            return Err(ParamError("feeding.reach must be finite and non-negative"));
        }
        if [
            self.mutation.weight_perturb_rate,
            self.mutation.weight_reset_rate,
            self.mutation.neuron_perturb_rate,
        ]
        .iter()
        .any(|value| !(0.0..=1.0).contains(value))
        {
            return Err(ParamError("mutation probabilities must be in [0, 1]"));
        }
        if [
            self.mutation.weight_limit,
            self.mutation.weight_perturb_sigma,
            self.mutation.bias_perturb_sigma,
            self.mutation.tau_perturb_factor,
        ]
        .iter()
        .any(|&value| !(value >= 0.0) || !value.is_finite())
        {
            return Err(ParamError(
                "mutation bounds and perturbation scales must be finite and non-negative",
            ));
        }
        let structural = &self.mutation.structural;
        for (rate, message) in [
            (
                structural.remove_connection_rate,
                "mutation.structural.remove_connection_rate must be in [0, 1]",
            ),
            (
                structural.remove_neuron_rate,
                "mutation.structural.remove_neuron_rate must be in [0, 1]",
            ),
            (
                structural.toggle_connection_rate,
                "mutation.structural.toggle_connection_rate must be in [0, 1]",
            ),
            (
                structural.add_connection_rate,
                "mutation.structural.add_connection_rate must be in [0, 1]",
            ),
            (
                structural.add_neuron_rate,
                "mutation.structural.add_neuron_rate must be in [0, 1]",
            ),
        ] {
            if !(0.0..=1.0).contains(&rate) {
                return Err(ParamError(message));
            }
        }
        if !structural.split_neuron_bias.is_finite() {
            return Err(ParamError(
                "mutation.structural.split_neuron_bias must be finite",
            ));
        }
        if !structural.split_input_weight.is_finite()
            || (structural.add_neuron_rate > 0.0
                && structural.split_input_weight.abs() > self.mutation.weight_limit)
        {
            return Err(ParamError(
                "mutation.structural.split_input_weight must be finite and within weight_limit when splitting is enabled",
            ));
        }
        let organs = &self.mutation.organs;
        for (rate, message) in [
            (
                organs.remove_sensor_rate,
                "mutation.organs.remove_sensor_rate must be in [0, 1]",
            ),
            (
                organs.add_sensor_rate,
                "mutation.organs.add_sensor_rate must be in [0, 1]",
            ),
        ] {
            if !(0.0..=1.0).contains(&rate) {
                return Err(ParamError(message));
            }
        }
        let weights = [
            organs.vision_weight,
            organs.chemo_weight,
            organs.energy_weight,
        ];
        if weights
            .iter()
            .any(|&weight| !weight.is_finite() || weight < 0.0)
        {
            return Err(ParamError(
                "mutation.organs modality weights must be finite and non-negative",
            ));
        }
        if organs.add_sensor_rate > 0.0 && weights.iter().all(|&weight| weight == 0.0) {
            return Err(ParamError(
                "mutation.organs needs a positive modality weight when addition is enabled",
            ));
        }
        if !organs.neuron_bias.is_finite() {
            return Err(ParamError("mutation.organs.neuron_bias must be finite"));
        }
        // Structural edits can leave a target with fan-in one. The scalar control
        // must still be able to redraw its full two-sided interval (spec section 3.3).
        if (!structural.is_disabled() || !organs.is_disabled())
            && !(2.0 * self.brain.weight_init_scale).is_finite()
        {
            return Err(ParamError(
                "brain.weight_init_scale must have a finite two-sided range when structural mutation is enabled",
            ));
        }
        if self
            .plants
            .signature
            .iter()
            .any(|c| !(0.0..=1.0).contains(c))
        {
            return Err(ParamError("plants.signature must be in [0, 1] per channel"));
        }
        if !(0.0..=0.5).contains(&self.world.founder_spread) {
            return Err(ParamError(
                "world.founder_spread must be in [0, 0.5]; beyond half the world a disc \
                 drawn from the centre wraps onto itself",
            ));
        }
        if !(0.0..=1.0).contains(&self.plants.initial_fill) {
            return Err(ParamError("plants.initial_fill must be in [0, 1]"));
        }
        if self.chemo.cells[0] == 0 || self.chemo.cells[1] == 0 || self.chemo.cells[2] != 1 {
            return Err(ParamError(
                "chemo.cells must be non-empty in x and y, and depth 1 in V1",
            ));
        }
        if self.chemo.decay.is_empty() {
            return Err(ParamError("chemo needs at least one channel"));
        }
        if self.chemo.decay.iter().any(|d| !(0.0..=1.0).contains(d)) {
            return Err(ParamError("chemo.decay must be in [0, 1] per channel"));
        }
        if !(0.0..=1.0).contains(&self.chemo.diffuse) {
            return Err(ParamError("chemo.diffuse must be in [0, 1]"));
        }
        if !(0.0..=1.0).contains(&self.movement.drag) {
            return Err(ParamError("movement.drag must be in [0, 1]"));
        }
        if [self.movement.max_thrust, self.movement.max_turn_rate]
            .iter()
            .any(|&value| !(value >= 0.0) || !value.is_finite())
        {
            return Err(ParamError(
                "movement thrust and turn limits must be finite and non-negative",
            ));
        }
        if !(self.movement.max_speed >= 0.0) || !self.movement.max_speed.is_finite() {
            return Err(ParamError(
                "movement.max_speed must be finite and non-negative",
            ));
        }
        if [
            self.metabolism.base,
            self.metabolism.k_size,
            self.metabolism.k_brain,
            self.metabolism.k_sensor,
            self.metabolism.k_move,
        ]
        .iter()
        .any(|&cost| !(cost >= 0.0) || !cost.is_finite())
        {
            return Err(ParamError(
                "metabolism costs must be finite and non-negative",
            ));
        }
        Ok(())
    }
}

impl Default for WorldParams {
    fn default() -> Self {
        Self {
            size: 1_000.0,
            founder_spread: 0.4,
            max_agents: 5_000,
            dt: 1.0 / 60.0,
        }
    }
}

impl Default for BodyParams {
    fn default() -> Self {
        Self { size: 3.0 }
    }
}

impl Default for MetabolismParams {
    fn default() -> Self {
        Self {
            base: 0.05,
            k_size: 0.00125,
            k_brain: 0.00005,
            k_sensor: 0.000625,
            k_move: 0.5,
        }
    }
}

impl Default for MovementParams {
    fn default() -> Self {
        Self {
            max_thrust: 1.0,
            max_turn_rate: 3.0,
            drag: 0.9,
            max_speed: 40.0,
        }
    }
}

impl Default for SensingParams {
    fn default() -> Self {
        Self {
            vision_range: 60.0,
            vision_fov: 0.5,
            vision_rays: 3,
            chemo_sensors: 1,
            energy_sensors: 1,
            chemo_radius: 40.0,
        }
    }
}

impl Default for BrainParams {
    fn default() -> Self {
        Self {
            hidden_neurons: 6,
            oscillators: 2,
            connections_per_target: None,
            tau_min: 0.05,
            tau_max: 2.0,
            oscillator_period_min: 10.0,
            oscillator_period_max: 240.0,
            weight_init_scale: 2.0,
        }
    }
}

impl Default for ReproductionParams {
    fn default() -> Self {
        Self {
            start_energy: 100.0,
            threshold: 200.0,
            gate: 0.5,
            energy_split: 0.5,
            spawn_radius: 8.0,
            maturity_ticks: 300,
        }
    }
}

impl Default for MutationParams {
    fn default() -> Self {
        Self {
            structural: StructuralMutationParams::default(),
            organs: OrganMutationParams::default(),
            weight_perturb_rate: 0.025,
            weight_perturb_sigma: 0.15,
            weight_reset_rate: 0.0015625,
            weight_limit: 4.0,
            neuron_perturb_rate: 0.00625,
            bias_perturb_sigma: 0.1,
            tau_perturb_factor: 0.1,
        }
    }
}

impl Default for PlantParams {
    fn default() -> Self {
        Self {
            energy_input_rate: 12_000.0,
            max_plants: 4_000,
            max_energy: 60.0,
            radius: 2.0,
            scent_rate: 0.02,
            initial_fill: 1.0,
            signature: [0.2, 0.8, 0.25],
        }
    }
}

impl Default for ChemoParams {
    fn default() -> Self {
        // Depth 1: the field is a 3D grid whose Z extent V1 pins to one cell, so the
        // triple-nested loops are already written (spec §9.1).
        Self {
            cells: [128, 128, 1],
            decay: vec![0.98],
            diffuse: 0.1,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn distance_coefficients_validate_at_construction_and_retune_boundaries() {
        type Change = fn(&mut DistanceParams, f32);
        let fields: [Change; 3] = [
            |p, v| p.disjoint_coefficient = v,
            |p, v| p.excess_coefficient = v,
            |p, v| p.weight_coefficient = v,
        ];
        let current = SimParams::default();
        for change in fields {
            for value in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut next = current.clone();
                change(&mut next.distance, value);
                assert!(next.distance.validate().is_err());
                assert!(next.validate().is_err());
                assert!(current.check_retune(&next, 62.5).is_err());
            }
            for value in [0.0, f32::MAX] {
                let mut next = current.clone();
                change(&mut next.distance, value);
                next.validate().unwrap();
                assert!(current.check_retune(&next, 62.5).is_err());
            }
        }
    }

    #[test]
    fn species_defaults_validate_and_classification_configuration_is_frozen() {
        let current = SimParams::default();
        assert_eq!(current.species.capacity, 256);
        assert_eq!(current.species.threshold, 0.5);
        assert_eq!(
            serde_json::from_str::<SimParams>("{}").unwrap().species,
            current.species
        );
        for threshold in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut next = current.clone();
            next.species.threshold = threshold;
            assert!(next.validate().is_err());
            assert!(current.check_retune(&next, 62.5).is_err());
        }
        for capacity in [0, 1, 257] {
            let mut next = current.clone();
            next.species.capacity = capacity;
            next.validate().unwrap();
            assert!(current.check_retune(&next, 62.5).is_err());
        }
        let mut next = current.clone();
        next.species.threshold = 0.75;
        next.validate().unwrap();
        assert!(current.check_retune(&next, 62.5).is_err());
    }

    #[test]
    fn distance_coefficients_default_and_round_trip_in_partial_params() {
        let legacy: SimParams = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.distance, DistanceParams::default());
        let params: SimParams =
            serde_json::from_str(r#"{"distance":{"weight_coefficient":0.75}}"#).unwrap();
        assert_eq!(params.distance.disjoint_coefficient, 1.0);
        assert_eq!(params.distance.excess_coefficient, 1.0);
        assert_eq!(params.distance.weight_coefficient, 0.75);
        assert_eq!(
            serde_json::from_str::<SimParams>(&serde_json::to_string(&params).unwrap()).unwrap(),
            params
        );
        assert!(serde_json::from_str::<SimParams>(r#"{"distance":{"threshold":1}}"#).is_err());
    }

    /// Retuning policy, exercised without building a world — which is why it lives on
    /// `SimParams` rather than on `World`.
    mod retune {
        use super::*;

        #[test]
        fn structural_defaults_are_disabled_and_old_zero_weight_limits_stay_valid() {
            let mut params = SimParams::default();
            assert!(params.mutation.structural.is_disabled());
            params.mutation.weight_limit = 0.0;
            params.validate().unwrap();
            params.mutation.structural.add_neuron_rate = 1.0;
            assert!(params.validate().is_err());
            params.mutation.structural.split_input_weight = 0.0;
            params.validate().unwrap();
        }

        #[test]
        fn structural_rates_and_initializers_are_validated() {
            type Change = fn(&mut StructuralMutationParams, f32);
            let rates: [Change; 5] = [
                |p, v| p.remove_connection_rate = v,
                |p, v| p.remove_neuron_rate = v,
                |p, v| p.toggle_connection_rate = v,
                |p, v| p.add_connection_rate = v,
                |p, v| p.add_neuron_rate = v,
            ];
            for change in rates {
                for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
                    let mut params = SimParams::default();
                    change(&mut params.mutation.structural, value);
                    assert!(params.validate().is_err());
                }
            }
            let mut params = SimParams::default();
            params.mutation.structural.split_neuron_bias = f32::NAN;
            assert!(params.validate().is_err());
            params.mutation.structural.split_neuron_bias = 0.0;
            params.mutation.structural.split_input_weight = f32::INFINITY;
            assert!(params.validate().is_err());
        }

        #[test]
        fn structural_rates_can_be_retuned_without_resizing_storage() {
            let current = SimParams::default();
            let mut next = current.clone();
            next.mutation.structural.add_connection_rate = 0.05;
            next.mutation.structural.add_neuron_rate = 0.02;
            next.mutation.structural.toggle_connection_rate = 0.02;
            current.check_retune(&next, 62.5).unwrap();
            assert_eq!(current.storage, next.storage);
        }

        #[test]
        fn structural_controls_require_representable_single_input_weight_draws() {
            let mut params = SimParams::default();
            params.brain.weight_init_scale = f32::MAX * 0.75;
            params.validate().unwrap();
            params.mutation.structural.add_neuron_rate = 0.02;
            assert!(params.validate().is_err());
        }

        const GRID_CELL: f32 = 50.0;

        /// A named edit to apply to a copy of the params, for the tables below.
        type Case = (&'static str, fn(&mut SimParams));

        fn pair() -> (SimParams, SimParams) {
            let mut base = SimParams::default();
            // Both radii comfortably inside the cell, so a test about some other field
            // cannot be passing on the sensing check instead.
            base.sensing.vision_range = 20.0;
            base.sensing.chemo_radius = 20.0;
            (base.clone(), base)
        }

        #[test]
        fn ordinary_tuning_goes_through() {
            let (current, mut next) = pair();
            next.metabolism.base *= 3.0;
            next.movement.drag = 0.4;
            next.reproduction.threshold = 200.0;
            next.mutation.weight_perturb_sigma = 0.9;
            next.feeding.reach = 6.0;
            next.chemo.decay = vec![0.5; next.chemo.decay.len()];
            next.world.dt = 1.0 / 30.0;
            next.world.founder_spread = 0.2;
            next.body.size = 1.0;
            assert!(current.check_retune(&next, GRID_CELL).is_ok());
        }

        #[test]
        fn anything_that_sized_an_allocation_is_frozen() {
            let cases: [Case; 5] = [
                ("world.max_agents", |p| p.world.max_agents += 1),
                ("world.size", |p| p.world.size += 1.0),
                ("plants.max_plants", |p| p.plants.max_plants += 1),
                ("chemo.cells", |p| p.chemo.cells[0] += 1),
                ("chemo channels", |p| p.chemo.decay.push(0.5)),
            ];
            for (name, mutate) in cases {
                let (current, mut next) = pair();
                mutate(&mut next);
                assert!(
                    current.check_retune(&next, GRID_CELL).is_err(),
                    "{name} was accepted"
                );
            }
        }

        #[test]
        fn the_entire_storage_policy_is_frozen() {
            let cases: [Case; 12] = [
                ("genes_per_slot", |p| p.storage.genes_per_slot += 1),
                ("neurons_per_slot", |p| p.storage.neurons_per_slot += 1),
                ("synapses_per_slot", |p| p.storage.synapses_per_slot += 1),
                ("sensors_per_slot", |p| p.storage.sensors_per_slot += 1),
                ("effectors_per_slot", |p| p.storage.effectors_per_slot += 1),
                ("max_genes", |p| p.storage.max_genes += 1),
                ("max_neurons", |p| p.storage.max_neurons += 1),
                ("max_connections", |p| p.storage.max_connections += 1),
                ("max_sensors", |p| p.storage.max_sensors += 1),
                ("max_vision_rays", |p| p.storage.max_vision_rays += 1),
                ("max_effectors", |p| p.storage.max_effectors += 1),
                ("max_memory_bytes", |p| p.storage.max_memory_bytes += 1),
            ];
            for (name, change) in cases {
                let (current, mut next) = pair();
                change(&mut next);
                next.validate()
                    .expect("retuning, not validation, must reject the change");
                assert_eq!(
                    current.check_retune(&next, GRID_CELL).unwrap_err(),
                    ParamError("storage is fixed for the life of a world"),
                    "{name} was not frozen",
                );
            }
        }

        #[test]
        fn removing_a_chemo_channel_is_also_rejected() {
            let (mut current, _) = pair();
            current.chemo.decay.push(0.5);
            let mut next = current.clone();
            next.chemo.decay.pop();
            assert!(current.check_retune(&next, GRID_CELL).is_err());
        }

        #[test]
        fn the_founding_topology_is_frozen() {
            // These counts size the founding template and fan-in scales. Changing them
            // without rebuilding the plan would silently keep the previous topology.
            let cases: [Case; 3] = [
                ("sensing.vision_rays", |p| p.sensing.vision_rays += 1),
                ("brain.hidden_neurons", |p| p.brain.hidden_neurons += 1),
                ("brain.oscillators", |p| p.brain.oscillators += 1),
            ];
            for (name, mutate) in cases {
                let (current, mut next) = pair();
                mutate(&mut next);
                assert!(
                    current.check_retune(&next, GRID_CELL).is_err(),
                    "{name} was accepted"
                );
            }
        }

        #[test]
        fn a_construction_only_field_is_refused_rather_than_ignored() {
            let (current, mut next) = pair();
            next.plants.initial_fill = 0.25;
            assert!(current.check_retune(&next, GRID_CELL).is_err());
        }

        #[test]
        fn sensing_may_shrink_but_not_outgrow_the_grid() {
            let (current, mut smaller) = pair();
            smaller.sensing.vision_range = GRID_CELL * 0.5;
            assert!(current.check_retune(&smaller, GRID_CELL).is_ok());

            let (current, mut exact) = pair();
            exact.sensing.vision_range = GRID_CELL;
            assert!(
                current.check_retune(&exact, GRID_CELL).is_ok(),
                "a radius of exactly one cell still fits one ring"
            );

            let (current, mut bigger) = pair();
            bigger.sensing.chemo_radius = GRID_CELL * 1.01;
            assert!(current.check_retune(&bigger, GRID_CELL).is_err());
        }

        #[test]
        fn an_invalid_value_is_still_invalid() {
            let (current, mut next) = pair();
            next.world.founder_spread = 0.9;
            assert!(current.check_retune(&next, GRID_CELL).is_err());
        }
    }

    use super::*;

    #[test]
    fn defaults_validate() {
        SimParams::default()
            .validate()
            .expect("shipped defaults must be coherent");
    }

    #[test]
    fn default_birth_split_funds_two_reference_tanks() {
        let reproduction = SimParams::default().reproduction;
        let child = reproduction.threshold * reproduction.energy_split;
        let parent = reproduction.threshold * (1.0 - reproduction.energy_split);
        assert_eq!(child, parent, "the default birth does not split evenly");
        assert_eq!(child, reproduction.start_energy);
    }

    #[test]
    fn round_trips_through_postcard() {
        let params = SimParams::default();
        let bytes = postcard::to_allocvec(&params).expect("serializes");
        let back: SimParams = postcard::from_bytes(&bytes).expect("deserializes");
        assert_eq!(params, back);
    }

    #[test]
    fn absent_fields_fall_back_to_defaults() {
        // The browser sets a handful of params and leaves the rest alone, so partial
        // JSON has to mean "the default for everything else" rather than an error.
        let json = r#"{"metabolism":{"base":0.2}}"#;
        let parsed: SimParams = serde_json::from_str(json).expect("partial params parse");
        assert_eq!(parsed.metabolism.base, 0.2);
        assert_eq!(
            parsed.metabolism.k_brain,
            MetabolismParams::default().k_brain
        );
        assert_eq!(parsed.world, WorldParams::default());
    }

    #[test]
    fn misspelled_fields_are_an_error_not_a_silent_default() {
        let json = r#"{"metabolism":{"bass":0.2}}"#;
        assert!(serde_json::from_str::<SimParams>(json).is_err());
    }

    #[test]
    fn partial_storage_configuration_defaults_but_unknown_fields_fail() {
        let parsed: SimParams = serde_json::from_str(
            r#"{"storage":{"genes_per_slot":300,"max_memory_bytes":120000000}}"#,
        )
        .unwrap();
        assert_eq!(parsed.storage.genes_per_slot, 300);
        assert_eq!(parsed.storage.max_memory_bytes, 120_000_000);
        assert_eq!(
            parsed.storage.neurons_per_slot,
            StorageParams::default().neurons_per_slot
        );
        assert_eq!(
            parsed.storage.max_connections,
            StorageParams::default().max_connections
        );
        assert!(
            serde_json::from_str::<SimParams>(r#"{"storage":{"gene_per_slot":300}}"#,).is_err()
        );
    }

    #[test]
    fn validation_rejects_incoherent_worlds() {
        type BreakIt = fn(&mut SimParams);
        let cases: Vec<(&str, BreakIt)> = vec![
            ("zero dt", |p| p.world.dt = 0.0),
            ("no agents", |p| p.world.max_agents = 0),
            ("sense radius wider than the world", |p| {
                p.sensing.vision_range = 10_000.0
            }),
            ("inverted tau range", |p| p.brain.tau_max = 0.001),
            ("zero weight init scale", |p| {
                p.brain.weight_init_scale = 0.0
            }),
            ("energy split above 1", |p| {
                p.reproduction.energy_split = 1.5
            }),
            ("chemo depth above 1", |p| p.chemo.cells[2] = 2),
            ("absurd agent pool", |p| p.world.max_agents = 3_000_000_000),
            ("absurd ray count", |p| {
                p.sensing.vision_rays = 2_000_000_000
            }),
            ("plant stock overflows the ledger", |p| {
                p.plants.max_energy = f32::MAX
            }),
            ("absurd chemo grid", |p| p.chemo.cells = [65_535, 65_535, 1]),
            ("sense radius too small for the world", |p| {
                p.sensing.vision_range = 0.01;
                p.sensing.chemo_radius = 0.01;
            }),
            ("breeding needs no growth", |p| {
                p.reproduction.threshold = 50.0
            }),
            ("no chemo channels", |p| p.chemo.decay.clear()),
            // A colour is copied straight into a neuron's input, so an out-of-range one
            // pins every neuron an eye feeds — the unbounded-channel class M6 closed.
            ("a negative feeding rate", |p| p.feeding.rate = -1.0),
            ("a plant colour outside [0, 1]", |p| {
                p.plants.signature = [1.0e6, -50.0, f32::MAX]
            }),
            ("decay above 1", |p| p.chemo.decay = vec![1.4]),
            ("negative max speed", |p| p.movement.max_speed = -1.0),
            ("negative metabolic cost", |p| p.metabolism.base = -1.0),
            ("infinite world size", |p| p.world.size = f32::INFINITY),
            ("infinite timestep", |p| p.world.dt = f32::INFINITY),
            ("negative body size", |p| p.body.size = -1.0),
            ("non-finite body size", |p| p.body.size = f32::NAN),
            ("negative thrust limit", |p| p.movement.max_thrust = -1.0),
            ("non-finite turn limit", |p| {
                p.movement.max_turn_rate = f32::NAN
            }),
            ("negative vision range", |p| p.sensing.vision_range = -1.0),
            ("non-finite chemo radius", |p| {
                p.sensing.chemo_radius = f32::NAN
            }),
            ("negative vision field of view", |p| {
                p.sensing.vision_fov = -1.0
            }),
            ("non-finite tau minimum", |p| p.brain.tau_min = f32::NAN),
            ("infinite tau maximum", |p| p.brain.tau_max = f32::INFINITY),
            ("non-finite oscillator minimum", |p| {
                p.brain.oscillator_period_min = f32::NAN
            }),
            ("infinite oscillator maximum", |p| {
                p.brain.oscillator_period_max = f32::INFINITY
            }),
            ("oscillator phase increment overflows", |p| {
                p.brain.oscillator_period_min = 1e-38
            }),
            ("tau reciprocal overflows", |p| p.brain.tau_min = 1e-40),
            ("neuron count overflows", |p| {
                p.brain.hidden_neurons = u32::MAX
            }),
            ("oscillator count overflows", |p| {
                p.brain.oscillators = u32::MAX
            }),
            ("arena byte count overflows WASM32", |p| {
                p.brain.hidden_neurons = 200
            }),
            ("non-finite starting energy", |p| {
                p.reproduction.start_energy = f32::INFINITY
            }),
            ("non-finite breeding threshold", |p| {
                p.reproduction.threshold = f32::NAN
            }),
            ("non-finite reproduction gate", |p| {
                p.reproduction.gate = f32::NAN
            }),
            ("non-finite spawn radius", |p| {
                p.reproduction.spawn_radius = f32::NAN
            }),
            ("non-finite feeding gate", |p| p.feeding.gate = f32::NAN),
            ("non-finite feeding reach", |p| p.feeding.reach = f32::NAN),
            ("negative plant radius", |p| p.plants.radius = -1.0),
            ("negative scent rate", |p| p.plants.scent_rate = -1.0),
            ("negative weight bound", |p| p.mutation.weight_limit = -1.0),
            ("negative mutation sigma", |p| {
                p.mutation.weight_perturb_sigma = -1.0
            }),
            ("non-finite bias sigma", |p| {
                p.mutation.bias_perturb_sigma = f32::NAN
            }),
            ("negative tau perturbation", |p| {
                p.mutation.tau_perturb_factor = -1.0
            }),
            ("weight perturbation probability above one", |p| {
                p.mutation.weight_perturb_rate = 1.1
            }),
            ("negative weight reset probability", |p| {
                p.mutation.weight_reset_rate = -0.1
            }),
            ("non-finite neuron perturbation probability", |p| {
                p.mutation.neuron_perturb_rate = f32::NAN
            }),
            ("chemo sizing overflows u64", |p| {
                p.chemo.cells = [u32::MAX, u32::MAX, 1];
                p.chemo.decay.push(0.5);
            }),
        ];
        for (name, break_it) in cases {
            let mut params = SimParams::default();
            break_it(&mut params);
            assert!(
                params.validate().is_err(),
                "should have been rejected: {name}"
            );
        }
    }

    #[test]
    fn cell_size_covers_every_sensor() {
        let mut params = SimParams::default();
        params.sensing.chemo_radius = 500.0;
        assert_eq!(params.sensing.max_sense_radius(), 500.0);
        params.sensing.vision_range = 900.0;
        assert_eq!(params.sensing.max_sense_radius(), 900.0);
    }
}
