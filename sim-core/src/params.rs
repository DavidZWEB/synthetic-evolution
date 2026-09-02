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

/// The complete parameter set for one world.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimParams {
    pub world: WorldParams,
    pub body: BodyParams,
    pub metabolism: MetabolismParams,
    pub movement: MovementParams,
    pub sensing: SensingParams,
    pub brain: BrainParams,
    pub reproduction: ReproductionParams,
    pub mutation: MutationParams,
    pub feeding: FeedingParams,
    pub plants: PlantParams,
    pub chemo: ChemoParams,
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
    /// That ~2000 is the relationship for *total* idle cost, not for this term alone,
    /// and the defaults do not yet meet it. At the default topology and body:
    ///
    /// | term | per tick | vs base |
    /// |---|---|---|
    /// | `base` | 0.050 | 1.0× |
    /// | `k_brain` × 268 units | 0.013 | 0.27× |
    /// | `k_sensor` × 16 units | 0.160 | 3.2× |
    /// | `k_size` × size² | 0.180 | 3.6× |
    /// | total | 0.403 | — |
    ///
    /// That is ~250 idle ticks on a 100-energy tank, against the ~2000 asked for.
    /// `k_brain` now lands where §5.5 wants it; the overshoot is the other two. Both
    /// come from defaults chosen here rather than from the spec: `k_size` is quadratic
    /// in [`BodyParams::size`], which defaults to 3, and `k_sensor` is multiplied by
    /// channel count rather than sensor count. Left as a known gap rather than papered
    /// over, because none of it is measurable until M7 wires metabolism to a running
    /// population — and guessing at three constants at once is how tuning becomes
    /// unfalsifiable.
    pub base: f32,
    /// Cost coefficient on size². Doubling radius should roughly quadruple upkeep.
    pub k_size: f32,
    /// Cost per neuron and per connection — see `genome::brain_complexity`, and note
    /// it is *not* per gene: sensors are billed by `k_sensor` instead.
    ///
    /// The term that stops genomes bloating without limit. It does no selective work
    /// until Phase 2, because every Phase 1 genome is the same size, so today it only
    /// adds to `base`.
    ///
    /// **0.00005, deliberately not the 0.001 in spec §5.5.** That table's two stated
    /// relationships contradict each other. A 200-unit brain at "~20% of base" means
    /// the term should land near 0.01 when `base` is 0.05 — but 200 × 0.001 is 0.2,
    /// four times base rather than a fifth of it. Taken literally it kills an idle
    /// agent in ~300 ticks against the ~2000 the same table asks for, which is spec
    /// §10's "population → 0 early" written into the defaults. 0.00005 satisfies both
    /// relationships at once.
    pub k_brain: f32,
    /// Cost per sensor, weighted by channel count — see `genome::sensor_load`. An eye
    /// returning distance and colour costs four times a single-channel interoceptor,
    /// and metering perception is what makes evolution pay for its own compute
    /// (spec §2.2c).
    ///
    /// Weighting by channels rather than per-sensor is this codebase's reading of §5.5's
    /// "0.01 each, weighted by modality"; it makes the default sensor set 16 weighted
    /// units rather than 5, so this term now dominates the metabolic budget. See the
    /// note on [`Self::base`] before tuning it.
    pub k_sensor: f32,
    /// Cost coefficient on |force|². Sprinting should drain a full tank in ~200 ticks.
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
    /// Rays per agent in Phase 1's fixed sensor set. Perception is 60–80% of tick
    /// cost once vision is in, and ray count is metered as a metabolic cost so
    /// evolution pays for its own compute (spec §2.2c).
    pub vision_rays: u32,
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

/// Fixed CTRNN topology for Phase 1. The *representation* is already a variable-length
/// gene list; only the mutation operators that change topology are absent (§3.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrainParams {
    /// Hidden neurons per brain.
    pub hidden_neurons: u32,
    /// Always-present oscillator neurons. Cheap scaffold — evolution finds them fast
    /// and builds gaits and timing on them (spec §3.2).
    pub oscillators: u32,
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
    /// Energy an agent is born with.
    pub start_energy: f32,
    /// Energy floor below which the reproduce effector does nothing. Must sit above
    /// [`Self::start_energy`], so growth is required before breeding — `validate`
    /// enforces that relationship rather than leaving it to a comment (spec §5.5).
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
    /// Per-connection chance of a Gaussian nudge.
    pub weight_perturb_rate: f32,
    /// Standard deviation of that nudge.
    pub weight_perturb_sigma: f32,
    /// Per-connection chance of a uniform resample instead.
    pub weight_reset_rate: f32,
    /// Bound on connection weights, for reset and clamping.
    pub weight_limit: f32,
    /// Per-neuron chance of a Gaussian nudge to bias, tau, or oscillator period.
    /// Without this the oscillator period in [`BrainParams`] is fixed for all time,
    /// and spec §3.2's "evolvable period" is not true of Phase 1.
    pub neuron_perturb_rate: f32,
    pub bias_perturb_sigma: f32,
    /// Multiplicative, so tau explores across orders of magnitude rather than
    /// random-walking off the bottom of its range.
    pub tau_perturb_factor: f32,
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
    pub reach: f32,
}

impl Default for FeedingParams {
    fn default() -> Self {
        Self {
            rate: 1.0,
            gate: 0.5,
            reach: 0.0,
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

/// Ceilings on the params that size an allocation or feed integer arithmetic.
///
/// These are not tuning limits — they are the boundary that keeps a bad value from
/// becoming a panic or an out-of-memory abort. Params arrive from JS at runtime, so
/// "nobody would set that" is not a guarantee (spec §7.6).
const MAX_AGENTS: u32 = 1_000_000;
const MAX_PLANTS: u32 = 1_000_000;
/// Perception is 60–80% of tick cost, and each ray widens every brain. Far above any
/// useful value, low enough that the brain-width arithmetic cannot overflow.
const MAX_VISION_RAYS: u32 = 256;
/// Total chemo cells across all axes and channels.
const MAX_CHEMO_CELLS: u64 = 16_777_216;
/// Spatial-hash cells per axis. The grid is `world.size / max_sense_radius` across, so
/// a very short sense radius in a large world is what blows this up.
const MAX_GRID_CELLS_PER_AXIS: f32 = 1_024.0;

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
    /// `!(x > 0.0)` rather than `x <= 0.0` throughout: the negated form also rejects
    /// NaN, which is the shape a bad value arrives in from JSON.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn validate(&self) -> Result<(), ParamError> {
        if !(self.world.size > 0.0) {
            return Err(ParamError("world.size must be positive"));
        }
        if !(self.world.dt > 0.0) {
            return Err(ParamError("world.dt must be positive"));
        }
        if self.world.max_agents == 0 {
            return Err(ParamError("world.max_agents must be non-zero"));
        }
        if self.world.max_agents > MAX_AGENTS {
            return Err(ParamError("world.max_agents exceeds the pool ceiling"));
        }
        if self.plants.max_plants > MAX_PLANTS {
            return Err(ParamError("plants.max_plants exceeds the pool ceiling"));
        }
        if self.sensing.vision_rays > MAX_VISION_RAYS {
            return Err(ParamError(
                "sensing.vision_rays exceeds the per-agent ceiling",
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
        if self.world.size / self.sensing.max_sense_radius() > MAX_GRID_CELLS_PER_AXIS {
            return Err(ParamError(
                "sense radius is too small for the world; the spatial grid would explode",
            ));
        }
        if self.brain.tau_min <= 0.0 || self.brain.tau_max < self.brain.tau_min {
            return Err(ParamError("brain.tau range must be positive and ordered"));
        }
        if self.brain.oscillator_period_min <= 0.0
            || self.brain.oscillator_period_max < self.brain.oscillator_period_min
        {
            return Err(ParamError(
                "brain.oscillator_period range must be positive and ordered",
            ));
        }
        if !(self.brain.weight_init_scale > 0.0) || !self.brain.weight_init_scale.is_finite() {
            return Err(ParamError("brain.weight_init_scale must be positive"));
        }
        if !(self.reproduction.start_energy > 0.0) {
            return Err(ParamError("reproduction.start_energy must be positive"));
        }
        if self.reproduction.threshold <= self.reproduction.start_energy {
            return Err(ParamError(
                "reproduction.threshold must exceed start_energy, or breeding needs no growth",
            ));
        }
        if !(0.0..=1.0).contains(&self.reproduction.energy_split) {
            return Err(ParamError("reproduction.energy_split must be in [0, 1]"));
        }
        if self.reproduction.spawn_radius < 0.0 {
            return Err(ParamError("reproduction.spawn_radius must be non-negative"));
        }
        if !(self.feeding.rate >= 0.0) || !self.feeding.rate.is_finite() {
            return Err(ParamError("feeding.rate must be non-negative"));
        }
        if self.feeding.reach < 0.0 {
            return Err(ParamError("feeding.reach must be non-negative"));
        }
        if self
            .plants
            .signature
            .iter()
            .any(|c| !(0.0..=1.0).contains(c))
        {
            return Err(ParamError("plants.signature must be in [0, 1] per channel"));
        }
        if self.chemo.cells[0] == 0 || self.chemo.cells[1] == 0 || self.chemo.cells[2] != 1 {
            return Err(ParamError(
                "chemo.cells must be non-empty in x and y, and depth 1 in V1",
            ));
        }
        if self.chemo.decay.is_empty() {
            return Err(ParamError("chemo needs at least one channel"));
        }
        let cells = self.chemo.cells.iter().map(|&c| c as u64).product::<u64>()
            * self.chemo.channels() as u64;
        if cells > MAX_CHEMO_CELLS {
            return Err(ParamError(
                "chemo grid times channels exceeds the cell ceiling",
            ));
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
        Ok(())
    }
}

impl Default for WorldParams {
    fn default() -> Self {
        Self {
            size: 1_000.0,
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
            k_size: 0.02,
            k_brain: 0.00005,
            k_sensor: 0.01,
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
            chemo_radius: 40.0,
        }
    }
}

impl Default for BrainParams {
    fn default() -> Self {
        Self {
            hidden_neurons: 6,
            oscillators: 2,
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
            threshold: 150.0,
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
            weight_perturb_rate: 0.8,
            weight_perturb_sigma: 0.15,
            weight_reset_rate: 0.05,
            weight_limit: 4.0,
            neuron_perturb_rate: 0.2,
            bias_perturb_sigma: 0.1,
            tau_perturb_factor: 0.1,
        }
    }
}

impl Default for PlantParams {
    fn default() -> Self {
        Self {
            energy_input_rate: 600.0,
            max_plants: 4_000,
            max_energy: 60.0,
            radius: 2.0,
            scent_rate: 0.02,
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
    use super::*;

    #[test]
    fn defaults_validate() {
        SimParams::default()
            .validate()
            .expect("shipped defaults must be coherent");
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
