//! Plants: the autotrophs that hold the energy entering the world.
//!
//! Not agents. They have no brain, no genome, and no decisions — they sit where they
//! were seeded, absorb the world's fixed energy input, and scent the chemo field in
//! proportion to what they hold. Spec §5.1 is explicit that they are the substrate
//! rather than organisms, and keeping them a separate pool is what stops them competing
//! for agent slots or accidentally acquiring a brain.
//!
//! **All energy enters the world here, at a fixed rate, and nowhere else.** That is the
//! invariant the whole economy rests on: selection is only real while energy is scarce,
//! and a second source anywhere — a birth bonus, a free meal, a rounding error that
//! rounds up — quietly makes every strategy viable (spec §5.1). [`grow`]
//! returns what it actually added so the ledger records the truth rather than the
//! nominal rate.
//!
//! Plants are **a population, not scenery** (spec §5.1). Regrowth depends on what a
//! plant still holds (`grazing_lag`): grass regrows from the leaf area and reserves
//! left to it, so stripping a site has a cost that outlasts the meal. A plant that
//! stays starved dies and reseeds elsewhere, usually near another plant, so patches
//! drift as grazers wear them down. Sites follow a fertility map (`crate::fertility`,
//! spec §5.3), so plants cluster on good ground. With every M9 field at zero this is
//! Phase 1's plants exactly: fixed, uniform sites that refill at a constant rate.
//!
//! The plant count never changes — a dead plant's slot re-establishes in the same
//! tick — so there is no pool and no free list, and the neighbour grid is rebuilt only
//! on a tick when some plant moved.
//!
//! Deliberately not here: being eaten. The plant-to-agent transfer is part of the energy
//! ledger and lands with metabolism and death.

use glam::Vec3;

use crate::chemo::ChemoField;
use crate::energy;
use crate::fertility::Fertility;
use crate::math;
use crate::params::{PlantParams, SimParams};
use crate::rng::Rng;
use crate::spatial::{SpatialHash, wrap_scalar};

/// A checkpoint's plant state, borrowed for validation and restore.
pub(crate) struct SavedPlants<'a> {
    pub position: &'a [Vec3],
    pub energy: &'a [f32],
    pub reserve: &'a [f64],
    pub starved: &'a [u32],
    pub reseeded: u64,
}

/// Every plant in the world: where it is and what it holds.
///
/// Struct-of-arrays for the same reason agents are, though the arrays are much shorter.
/// Every slot is always live — a dead plant's slot reseeds in the same tick — so there
/// is no pool and no free list here.
#[derive(Clone, Debug)]
pub struct Plants {
    position: Vec<Vec3>,
    energy: Vec<f32>,
    energy_reserve: Vec<f64>,
    /// Consecutive ticks each plant has spent below `death_stock`. Stays zero while
    /// turnover is off.
    starved: Vec<u32>,
    /// Plants that have died and reseeded since the world was built.
    reseeded: u64,
    /// One byte per plant, all ones. The spatial hash takes a liveness mask and plants
    /// are always live; keeping the array rather than special-casing the hash is what
    /// lets the same well-tested grid serve both populations.
    alive: Vec<u8>,
    /// Grid-cell scratch for rebuilding after a reseed, kept so a tick never allocates.
    cells: Vec<u32>,
    /// Where a reseeded plant may establish; drawn once with the world.
    fertility: Fertility,
    hash: SpatialHash,
}

impl Plants {
    /// Scatters `max_plants` plants across the world, each stocked to `initial_fill`.
    ///
    /// Sites follow the fertility map (spec §5.3): uniform when `patchiness` is zero,
    /// clustered on fertile ground otherwise.
    pub fn new(params: &SimParams, rng: &mut Rng) -> Self {
        let count = params.plants.max_plants as usize;
        let size = params.world.size;
        let fertility = Fertility::new(params, rng);
        let position: Vec<Vec3> = (0..count)
            .map(|_| fertility.site(rng, |rng| fertility.anywhere(rng)))
            .collect();

        let mut hash = SpatialHash::new(size, params.sensing.max_sense_radius(), count as u32);
        let alive = vec![1u8; count];
        let mut cells = vec![0u32; count];
        hash.rebuild(&position, &alive, &mut cells);

        // Uniform, and drawing nothing from `rng` — see `PlantParams::initial_fill`.
        let stock = params.plants.initial_fill * params.plants.max_energy;

        Self {
            position,
            energy: vec![stock; count],
            energy_reserve: vec![0.0; count],
            starved: vec![0; count],
            reseeded: 0,
            alive,
            cells,
            fertility,
            hash,
        }
    }

    /// Absorbs one tick of the world's energy input, and reports how much actually
    /// landed.
    ///
    /// The nominal rate is offered evenly to every plant, and each one is capped at
    /// `max_energy` and, with a non-zero `grazing_lag`, takes less the less it holds.
    /// So a world absorbs **less** than its input rate both at carrying capacity and
    /// when overgrazed — the surplus is not stored anywhere, it simply never enters.
    /// That is the honest behaviour for a saturated or stripped ecosystem, and it is
    /// exactly why the ledger records the returned figure rather than
    /// `energy_input_rate * dt`: conservation has to be measured, not inferred
    /// (spec §5.1).
    pub fn grow(&mut self, params: &PlantParams, dt: f32) -> f64 {
        if self.energy.is_empty() {
            return 0.0;
        }
        let share = (params.energy_input_rate * dt / self.energy.len() as f32) as f64;
        let ceiling = params.max_energy as f64;
        let lag = params.grazing_lag as f64;
        let mut absorbed = 0.0f64;
        for (energy, reserve) in self.energy.iter_mut().zip(self.energy_reserve.iter_mut()) {
            // Skipped at zero rather than multiplied by one, so Phase 1's plants stay
            // bit-identical and an empty ceiling never divides by zero.
            let offered = if lag > 0.0 && ceiling > 0.0 {
                let held = (energy::total(*energy, *reserve) / ceiling).clamp(0.0, 1.0);
                share * (1.0 - lag * (1.0 - held))
            } else {
                share
            };
            absorbed += energy::add_capped(energy, reserve, offered, ceiling);
        }
        absorbed
    }

    /// Deposits scent into chemo channel 0, in proportion to what each plant holds.
    ///
    /// This is what gives the chemo sensor something to climb before any agent can emit
    /// anything, and it is why a bigger plant is easier to find than a nearly-eaten one.
    /// It costs the plant nothing: scent is a signal, not a transfer, and charging for
    /// it would be a second energy sink outside the ledger.
    pub fn scent(&self, field: &mut ChemoField, params: &PlantParams, dt: f32) {
        for (i, &position) in self.position.iter().enumerate() {
            let held = energy::total(self.energy[i], self.energy_reserve[i]) as f32;
            field.deposit(0, position, held * params.scent_rate * dt);
        }
    }

    /// Kills plants that have starved too long and reseeds each at once, returning how
    /// many died this tick (spec §5.1).
    ///
    /// A plant below `death_stock` of capacity for `death_seconds` dies. Its slot
    /// re-establishes at a new site: with probability `local_dispersal` within
    /// `dispersal_radius` of a uniformly chosen plant, otherwise anywhere, and either
    /// way subject to the fertility map. Its remaining stock moves with the slot, so no
    /// energy is created or destroyed and the count never changes. Resolved in
    /// plant-index order and drawing from `rng` only when a plant dies; with
    /// `death_stock` at zero nothing is read, counted, or drawn.
    pub fn turn_over(&mut self, params: &SimParams, rng: &mut Rng) -> u32 {
        let plants = &params.plants;
        if plants.death_stock <= 0.0 {
            return 0;
        }
        let threshold = plants.death_stock as f64 * plants.max_energy as f64;
        let lifetime = plants.death_seconds as f64;
        let dt = params.world.dt as f64;
        let mut died = 0u32;
        for i in 0..self.position.len() {
            if energy::total(self.energy[i], self.energy_reserve[i]) >= threshold {
                self.starved[i] = 0;
                continue;
            }
            self.starved[i] = self.starved[i].saturating_add(1);
            if self.starved[i] as f64 * dt < lifetime {
                continue;
            }
            self.position[i] = self.seedling_site(plants, rng);
            self.starved[i] = 0;
            died += 1;
        }
        if died > 0 {
            self.reseeded += u64::from(died);
            self.hash
                .rebuild(&self.position, &self.alive, &mut self.cells);
        }
        died
    }

    /// Where a dead plant's slot re-establishes: near a parent or anywhere, as the
    /// fertility map allows.
    fn seedling_site(&self, plants: &PlantParams, rng: &mut Rng) -> Vec3 {
        let size = self.fertility.world_size();
        let count = self.position.len() as u32;
        self.fertility.site(rng, |rng| {
            if !rng.chance(plants.local_dispersal) {
                return self.fertility.anywhere(rng);
            }
            let parent = self.position[rng.below(count) as usize];
            // Uniform over the disc: the square root keeps seeds from crowding the
            // parent, and polar form needs no rejection loop.
            let distance = plants.dispersal_radius * math::sqrt(rng.unit());
            let angle = core::f32::consts::TAU * rng.unit();
            Vec3::new(
                wrap_scalar(parent.x + distance * math::cos(angle), size),
                wrap_scalar(parent.y + distance * math::sin(angle), size),
                0.0,
            )
        })
    }

    /// Total energy held across every plant. The stock half of the conservation check.
    ///
    /// Values stay `f32` in world state, but the fixed-order sum is `f64`: ledger
    /// opening and later measurements must use the same aggregation or summation
    /// rounding alone looks like an energy leak (spec §5.1).
    pub fn total_energy(&self) -> f64 {
        self.energy
            .iter()
            .zip(self.energy_reserve.iter())
            .map(|(&energy, &reserve)| energy::total(energy, reserve))
            .sum()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.position.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.position.is_empty()
    }

    #[inline]
    pub fn position(&self) -> &[Vec3] {
        &self.position
    }

    #[inline]
    pub fn energy(&self) -> &[f32] {
        &self.energy
    }

    #[inline]
    pub(crate) fn energy_reserve(&self) -> &[f64] {
        &self.energy_reserve
    }

    /// Restores saved plants over a world rebuilt from the run's seed.
    ///
    /// Untrusted input: per plant, a finite site on the plane inside the world, one
    /// finite, non-negative tank, one finite reserve, and a starvation count. A refused
    /// restore leaves the plants unchanged.
    pub(crate) fn restore_state(&mut self, saved: SavedPlants<'_>) -> Result<(), &'static str> {
        let count = self.len();
        if saved.position.len() != count
            || saved.energy.len() != count
            || saved.reserve.len() != count
            || saved.starved.len() != count
        {
            return Err("plant state does not match the plant count");
        }
        // Inclusive: wrapping a reseed just below zero can round up to exactly the
        // world size, which the grid folds back to zero.
        let size = self.fertility.world_size();
        if !saved.position.iter().all(|p| {
            p.is_finite()
                && p.z == 0.0
                && (0.0..=size).contains(&p.x)
                && (0.0..=size).contains(&p.y)
        }) {
            return Err("plant sites must lie on the world's plane");
        }
        if !saved.energy.iter().all(|&e| e.is_finite() && e >= 0.0)
            || !saved.reserve.iter().all(|r| r.is_finite())
        {
            return Err("plant stock must be finite and non-negative");
        }
        self.position.copy_from_slice(saved.position);
        self.energy.copy_from_slice(saved.energy);
        self.energy_reserve.copy_from_slice(saved.reserve);
        self.starved.copy_from_slice(saved.starved);
        self.reseeded = saved.reseeded;
        self.hash
            .rebuild(&self.position, &self.alive, &mut self.cells);
        Ok(())
    }

    /// Consecutive ticks each plant has spent starving.
    #[inline]
    pub(crate) fn starved(&self) -> &[u32] {
        &self.starved
    }

    /// Plants that have died and reseeded since the world was built.
    #[inline]
    pub fn reseeded(&self) -> u64 {
        self.reseeded
    }

    #[inline]
    pub(crate) fn energy_at(&self, index: usize) -> f64 {
        energy::total(self.energy[index], self.energy_reserve[index])
    }

    pub(crate) fn transfer_to(
        &mut self,
        index: usize,
        destination: &mut f32,
        destination_reserve: &mut f64,
        wanted: f32,
    ) -> f64 {
        let (Some(source), Some(source_reserve)) = (
            self.energy.get_mut(index),
            self.energy_reserve.get_mut(index),
        ) else {
            return 0.0;
        };
        energy::transfer(
            source,
            source_reserve,
            destination,
            destination_reserve,
            wanted as f64,
        )
    }

    #[inline]
    pub fn alive(&self) -> &[u8] {
        &self.alive
    }

    /// Neighbour grid over the plants, rebuilt whenever a plant reseeds.
    #[inline]
    pub fn hash(&self) -> &SpatialHash {
        &self.hash
    }

    #[cfg(test)]
    /// Removes up to `wanted` energy from one plant for plant-local tests.
    pub fn take(&mut self, index: usize, wanted: f32) -> f64 {
        let (Some(value), Some(reserve)) = (
            self.energy.get_mut(index),
            self.energy_reserve.get_mut(index),
        ) else {
            return 0.0;
        };
        energy::take(value, reserve, wanted as f64)
    }
}

#[cfg(test)]
impl Plants {
    /// Moves plants to exact positions and rebuilds the grid, for tests that need to
    /// know where the scenery is. At runtime only turnover moves a plant.
    pub fn place_for_test(&mut self, positions: &[Vec3]) {
        assert_eq!(positions.len(), self.position.len(), "wrong plant count");
        self.position.copy_from_slice(positions);
        self.hash
            .rebuild(&self.position, &self.alive, &mut self.cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::ChemoParams;

    fn world() -> (Plants, SimParams) {
        filled(1.0)
    }

    /// A world stocked to `fill` of `max_energy`, with Phase 1's plants: uniform,
    /// regrowing at the full share, never dying. Tests of the M9 ecology switch on what
    /// they measure. Growth is only measurable from empty.
    fn filled(fill: f32) -> (Plants, SimParams) {
        let mut params = SimParams::default();
        params.plants.max_plants = 200;
        params.plants.initial_fill = fill;
        params.plants.grazing_lag = 0.0;
        params.plants.patchiness = 0.0;
        params.plants.death_stock = 0.0;
        let mut rng = Rng::from_seed(4);
        (Plants::new(&params, &mut rng), params)
    }

    #[test]
    fn plants_are_scattered_inside_the_world() {
        let (plants, params) = world();
        assert_eq!(plants.len(), 200);
        for &p in plants.position() {
            assert!((0.0..params.world.size).contains(&p.x), "{p:?}");
            assert!((0.0..params.world.size).contains(&p.y), "{p:?}");
            assert_eq!(p.z, 0.0, "V1 simulates on a plane");
        }
    }

    #[test]
    fn patchy_plants_crowd_together() {
        // Spec §5.3: the same number of plants, packed onto fertile ground, sit closer
        // to their nearest neighbour than a uniform scatter does.
        let nearest = |plants: &Plants, size: f32| -> f32 {
            let positions = plants.position();
            positions
                .iter()
                .enumerate()
                .map(|(i, &p)| {
                    positions
                        .iter()
                        .enumerate()
                        .filter(|&(j, _)| j != i)
                        .map(|(_, &q)| crate::spatial::min_image(q - p, size).length())
                        .fold(f32::INFINITY, f32::min)
                })
                .sum::<f32>()
                / positions.len() as f32
        };
        let (uniform, params) = world();
        let mut patchy_params = params.clone();
        patchy_params.plants.patchiness = 4.0;
        let patchy = Plants::new(&patchy_params, &mut Rng::from_seed(4));
        let size = params.world.size;
        let (spread, packed) = (nearest(&uniform, size), nearest(&patchy, size));
        assert!(packed < spread * 0.8, "patchy {packed} vs uniform {spread}");
    }

    #[test]
    fn seeding_is_deterministic() {
        let params = SimParams::default();
        let a = Plants::new(&params, &mut Rng::from_seed(9));
        let b = Plants::new(&params, &mut Rng::from_seed(9));
        assert_eq!(a.position(), b.position());
    }

    #[test]
    fn growth_absorbs_the_input_rate_while_there_is_room() {
        let (mut plants, params) = filled(0.0);
        let dt = params.world.dt;
        let absorbed = plants.grow(&params.plants, dt);
        let expected = params.plants.energy_input_rate * dt;
        assert!(
            (absorbed - expected as f64).abs() < expected as f64 * 1e-3,
            "absorbed {absorbed}, input was {expected}"
        );
        assert!((plants.total_energy() - absorbed).abs() < 1e-2);
    }

    #[test]
    fn grazing_lag_slows_regrowth_in_proportion_to_what_was_eaten() {
        // Spec §5.1: a plant at stock fraction x takes 1 - lag * (1 - x) of its share.
        let (mut plants, mut params) = filled(0.0);
        params.plants.grazing_lag = 0.8;
        let ceiling = params.plants.max_energy;
        plants.energy[1] = ceiling * 0.5;
        plants.energy[2] = ceiling * 0.9;
        let before: Vec<f32> = plants.energy().to_vec();
        let dt = params.world.dt;
        plants.grow(&params.plants, dt);
        let share = (params.plants.energy_input_rate * dt / plants.len() as f32) as f64;
        for (index, fraction) in [(0, 0.0), (1, 0.5), (2, 0.9)] {
            let grew = plants.energy_at(index) - before[index] as f64;
            let expected = share * (1.0 - 0.8 * (1.0 - fraction));
            assert!(
                (grew - expected).abs() < expected * 1e-4,
                "plant at {fraction}: grew {grew}, expected {expected}"
            );
        }
    }

    #[test]
    fn an_emptied_plant_still_regrows_under_heavy_grazing_lag() {
        let (mut plants, mut params) = filled(0.0);
        params.plants.grazing_lag = 0.99;
        for _ in 0..1_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        assert!(plants.energy()[0] > 0.0, "an emptied plant never regrew");
    }

    #[test]
    fn grazing_lag_conserves_what_it_absorbs() {
        let (mut plants, mut params) = filled(0.3);
        params.plants.grazing_lag = 0.5;
        let before = plants.total_energy();
        let mut absorbed = 0.0;
        for _ in 0..500 {
            absorbed += plants.grow(&params.plants, params.world.dt);
        }
        assert!((plants.total_energy() - before - absorbed).abs() < 1e-6);
    }

    #[test]
    fn a_stripped_world_absorbs_less_of_its_input_under_grazing_lag() {
        let (mut lagged, mut params) = filled(0.0);
        params.plants.grazing_lag = 0.5;
        let (mut even, plain) = filled(0.0);
        let dt = params.world.dt;
        let slow = lagged.grow(&params.plants, dt);
        let fast = even.grow(&plain.plants, dt);
        assert!((slow - fast * 0.5).abs() < fast * 1e-4, "{slow} vs {fast}");
    }

    /// A world whose plants die after `seconds` below `stock`, with local dispersal.
    fn mortal(stock: f32, seconds: f32) -> (Plants, SimParams) {
        let (plants, mut params) = filled(1.0);
        params.plants.death_stock = stock;
        params.plants.death_seconds = seconds;
        params.plants.local_dispersal = 1.0;
        params.plants.dispersal_radius = 10.0;
        (plants, params)
    }

    #[test]
    fn turnover_off_reads_counts_and_draws_nothing() {
        let (mut plants, params) = filled(0.0);
        let mut rng = Rng::from_seed(2);
        let before = rng.state_fingerprint();
        let sites = plants.position().to_vec();
        for _ in 0..10_000 {
            assert_eq!(plants.turn_over(&params, &mut rng), 0);
        }
        assert_eq!(rng.state_fingerprint(), before, "turnover drew while off");
        assert_eq!(plants.position(), &sites[..]);
        assert!(plants.starved().iter().all(|&ticks| ticks == 0));
    }

    #[test]
    fn a_plant_dies_after_starving_for_death_seconds_and_keeps_its_stock() {
        // 0.5 s at 60 ticks per second is 30 ticks of starving.
        let (mut plants, params) = mortal(0.1, 0.5);
        let mut rng = Rng::from_seed(2);
        plants.take(5, 1e9);
        plants.energy[5] = 1.0;
        let site = plants.position()[5];
        let before = plants.total_energy();
        for _ in 0..29 {
            assert_eq!(plants.turn_over(&params, &mut rng), 0);
        }
        assert_eq!(plants.position()[5], site, "died early");
        assert_eq!(plants.turn_over(&params, &mut rng), 1);
        assert_ne!(plants.position()[5], site, "did not reseed elsewhere");
        assert_eq!(plants.starved()[5], 0, "a seedling starts its own count");
        assert_eq!(
            plants.energy()[5],
            1.0,
            "the stock did not move with the slot"
        );
        assert_eq!(
            plants.total_energy(),
            before,
            "turnover changed the energy held"
        );
        assert_eq!(plants.reseeded(), 1);
    }

    #[test]
    fn recovering_past_death_stock_resets_the_count() {
        let (mut plants, params) = mortal(0.1, 0.5);
        let mut rng = Rng::from_seed(2);
        plants.take(5, 1e9);
        for _ in 0..20 {
            plants.turn_over(&params, &mut rng);
        }
        plants.energy[5] = 30.0;
        plants.turn_over(&params, &mut rng);
        assert_eq!(plants.starved()[5], 0);
    }

    #[test]
    fn local_seed_lands_within_the_dispersal_radius_of_a_plant() {
        let (mut plants, params) = mortal(0.1, 0.0);
        let size = params.world.size;
        let mut rng = Rng::from_seed(9);
        for round in 0..50 {
            let index = round % plants.len();
            let parents = plants.position().to_vec();
            plants.take(index, 1e9);
            assert_eq!(plants.turn_over(&params, &mut rng), 1);
            let landed = plants.position()[index];
            let nearest = parents
                .iter()
                .map(|&p| crate::spatial::min_image(landed - p, size).length())
                .fold(f32::INFINITY, f32::min);
            assert!(nearest <= 10.0 + 1e-3, "landed {nearest} from every parent");
            plants.energy[index] = 60.0;
        }
    }

    #[test]
    fn the_grid_follows_a_reseeded_plant() {
        let (mut plants, params) = mortal(0.1, 0.0);
        let mut rng = Rng::from_seed(4);
        plants.take(0, 1e9);
        plants.energy[0] = 2.0;
        plants.turn_over(&params, &mut rng);
        let site = plants.position()[0];
        let mut found = false;
        plants
            .hash()
            .for_each_within(plants.position(), site, 0.5, |index, _, _| {
                found |= index == 0;
            });
        assert!(found, "the grid still placed the plant at its old site");
    }

    #[test]
    fn restore_accepts_a_site_wrapped_onto_the_far_edge() {
        // wrap_scalar(-1e-6, 1000.0) rounds to exactly 1000.0 in f32.
        let edge = crate::spatial::wrap_scalar(-1e-6, 1000.0);
        assert_eq!(
            edge, 1000.0,
            "the rounding case this guards no longer occurs"
        );
        let (mut plants, _) = world();
        let mut position = plants.position().to_vec();
        position[0] = Vec3::new(edge, 3.0, 0.0);
        let (energy, reserve) = (plants.energy().to_vec(), plants.energy_reserve().to_vec());
        let starved = plants.starved().to_vec();
        fn saved<'a>(
            position: &'a [Vec3],
            energy: &'a [f32],
            reserve: &'a [f64],
            starved: &'a [u32],
        ) -> SavedPlants<'a> {
            SavedPlants {
                position,
                energy,
                reserve,
                starved,
                reseeded: 0,
            }
        }
        plants
            .restore_state(saved(&position, &energy, &reserve, &starved))
            .unwrap();
        position[0].x = 1000.5;
        assert!(
            plants
                .restore_state(saved(&position, &energy, &reserve, &starved))
                .is_err()
        );
    }

    #[test]
    fn turnover_is_deterministic() {
        let run = || {
            let (mut plants, params) = mortal(0.5, 0.1);
            let mut rng = Rng::from_seed(13);
            for index in 0..plants.len() {
                if index % 3 == 0 {
                    plants.take(index, 1e9);
                }
            }
            for _ in 0..20 {
                plants.turn_over(&params, &mut rng);
            }
            plants.position().to_vec()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_larder_is_stocked_before_the_first_tick() {
        let (full, params) = world();
        let ceiling = params.plants.max_energy as f64 * full.len() as f64;
        assert!((full.total_energy() - ceiling).abs() < 1e-1);

        let (half, _) = filled(0.5);
        assert!((half.total_energy() - ceiling * 0.5).abs() < 1e-1);

        let (bare, _) = filled(0.0);
        assert_eq!(bare.total_energy(), 0.0);
    }

    #[test]
    fn a_full_world_absorbs_less_than_its_input_rate() {
        // The honest behaviour of a saturated ecosystem, and the reason the ledger
        // records what `grow` returns rather than the nominal rate: at carrying capacity
        // the surplus never enters the world at all (spec §5.1).
        let (mut plants, params) = world();
        for _ in 0..100_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        let ceiling = params.plants.max_energy as f64 * plants.len() as f64;
        assert!(
            (plants.total_energy() - ceiling).abs() < 1e-1,
            "did not reach carrying capacity: {} of {ceiling}",
            plants.total_energy()
        );
        let absorbed = plants.grow(&params.plants, params.world.dt);
        assert_eq!(absorbed, 0.0, "a full world kept absorbing energy");
    }

    #[test]
    fn no_plant_grows_past_its_ceiling() {
        let (mut plants, params) = world();
        for _ in 0..100_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        for &e in plants.energy() {
            assert!(e <= params.plants.max_energy + 1e-4, "overgrew to {e}");
        }
    }

    #[test]
    fn scent_tracks_what_a_plant_holds() {
        // A bigger plant should be easier to find than a nearly-eaten one, or there is
        // no gradient worth evolving toward.
        let (mut plants, params) = world();
        let mut field = ChemoField::new(&ChemoParams::default(), params.world.size);
        plants.energy[0] = 50.0;
        plants.energy[1] = 1.0;
        plants.scent(&mut field, &params.plants, params.world.dt);
        let fat = field.sample(0, plants.position()[0]);
        let thin = field.sample(0, plants.position()[1]);
        assert!(fat > thin, "{fat} vs {thin}");
        assert!(thin > 0.0, "a live plant left no scent at all");
    }

    #[test]
    fn scent_costs_the_plant_nothing() {
        // Scent is a signal, not a transfer. Charging for it would be a second energy
        // sink outside the ledger, and conservation would fail for a reason nobody would
        // think to look for (spec §5.1).
        let (mut plants, params) = world();
        let mut field = ChemoField::new(&ChemoParams::default(), params.world.size);
        plants.grow(&params.plants, params.world.dt);
        let before = plants.total_energy();
        for _ in 0..100 {
            plants.scent(&mut field, &params.plants, params.world.dt);
        }
        assert_eq!(plants.total_energy(), before);
    }

    #[test]
    fn taking_never_creates_or_destroys_energy() {
        let (mut plants, params) = world();
        for _ in 0..1_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        let before = plants.total_energy();
        let taken = plants.take(3, 1.0) + plants.take(3, 1e9) + plants.take(3, 1.0);
        assert!((plants.total_energy() + taken - before).abs() < 1e-2);
        assert_eq!(
            plants.energy()[3],
            0.0,
            "an emptied plant should hold nothing"
        );
        assert_eq!(plants.take(3, 1.0), 0.0, "an empty plant kept giving");
    }

    #[test]
    fn taking_from_a_plant_that_does_not_exist_is_a_no_op() {
        let (mut plants, _) = world();
        assert_eq!(plants.take(usize::MAX, 5.0), 0.0);
        assert_eq!(plants.take(999_999, 5.0), 0.0);
    }

    #[test]
    fn an_emptied_plant_regrows_in_place() {
        // Spec §5.1's "get eaten, and reseed", read as regrowth: the site persists, so
        // the food map is fixed and the neighbour grid never needs rebuilding.
        let (mut plants, params) = world();
        for _ in 0..1_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        let where_it_was = plants.position()[7];
        plants.take(7, 1e9);
        assert_eq!(plants.energy()[7], 0.0);
        for _ in 0..1_000 {
            plants.grow(&params.plants, params.world.dt);
        }
        assert!(plants.energy()[7] > 0.0, "did not regrow");
        assert_eq!(plants.position()[7], where_it_was, "a plant moved");
    }

    #[test]
    fn the_neighbour_grid_finds_plants() {
        let (plants, params) = world();
        let target = plants.position()[11];
        let mut found = Vec::new();
        plants.hash().for_each_within(
            plants.position(),
            target,
            params.sensing.max_sense_radius(),
            |index, _, _| found.push(index),
        );
        assert!(found.contains(&11), "a plant could not find itself");
    }
}
