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
//! Plants are **fixed sites that regrow in place**, not wandering seeds. Spec §5.1's
//! "get eaten, and reseed" is read here as regrowth, which is the weaker of the two
//! meanings and the one Phase 1 needs; it also means positions never change, so the
//! neighbour grid is built once rather than every tick. If a population ever evolves to
//! camp on a plant rather than forage between them, relocating a depleted site is the
//! small change that breaks it — see the note in the M7 plan.
//!
//! Deliberately not here: being eaten. The plant-to-agent transfer is part of the energy
//! ledger and lands with metabolism and death.

use glam::Vec3;

use crate::chemo::ChemoField;
use crate::energy;
use crate::params::{PlantParams, SimParams};
use crate::rng::Rng;
use crate::spatial::SpatialHash;

/// Every plant in the world: where it is and what it holds.
///
/// Struct-of-arrays for the same reason agents are, though the arrays are much shorter.
/// Every slot is always live — a plant is never destroyed, only emptied — so there is no
/// pool and no free list here.
#[derive(Clone, Debug)]
pub struct Plants {
    position: Vec<Vec3>,
    energy: Vec<f32>,
    energy_reserve: Vec<f64>,
    /// One byte per plant, all ones. The spatial hash takes a liveness mask and plants
    /// are always live; keeping the array rather than special-casing the hash is what
    /// lets the same well-tested grid serve both populations.
    alive: Vec<u8>,
    hash: SpatialHash,
}

impl Plants {
    /// Scatters `max_plants` plants across the world, each starting empty.
    ///
    /// Positions are drawn once and never change, so the neighbour grid is built here
    /// and not rebuilt again. Spatially uniform for now; spec §5.3's nutrient
    /// heterogeneity — the thing that actually creates niches — is a later phase.
    pub fn new(params: &SimParams, rng: &mut Rng) -> Self {
        let count = params.plants.max_plants as usize;
        let size = params.world.size;
        let position: Vec<Vec3> = (0..count)
            .map(|_| Vec3::new(rng.range(0.0, size), rng.range(0.0, size), 0.0))
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
            alive,
            hash,
        }
    }

    /// Absorbs one tick of the world's energy input, and reports how much actually
    /// landed.
    ///
    /// The nominal rate is shared evenly across every plant and each one is capped at
    /// `max_energy`, so a world at carrying capacity absorbs **less** than its input
    /// rate — the surplus is not stored anywhere, it simply never enters. That is the
    /// honest behaviour for a saturated ecosystem, and it is exactly why the ledger
    /// records the returned figure rather than `energy_input_rate * dt`: conservation
    /// has to be measured, not inferred (spec §5.1).
    pub fn grow(&mut self, params: &PlantParams, dt: f32) -> f64 {
        if self.energy.is_empty() {
            return 0.0;
        }
        let share = params.energy_input_rate * dt / self.energy.len() as f32;
        let mut absorbed = 0.0f64;
        for (energy, reserve) in self.energy.iter_mut().zip(self.energy_reserve.iter_mut()) {
            absorbed += energy::add_capped(energy, reserve, share as f64, params.max_energy as f64);
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

    /// Restores saved stocks onto sites regenerated from the run's seed.
    ///
    /// Untrusted input: one finite, non-negative tank and one finite reserve per site.
    /// A refused restore leaves the stock unchanged.
    pub(crate) fn restore_stock(
        &mut self,
        energy: &[f32],
        reserve: &[f64],
    ) -> Result<(), &'static str> {
        if energy.len() != self.len() || reserve.len() != self.len() {
            return Err("plant stock does not match the plant count");
        }
        if !energy.iter().all(|&e| e.is_finite() && e >= 0.0)
            || !reserve.iter().all(|r| r.is_finite())
        {
            return Err("plant stock must be finite and non-negative");
        }
        self.energy.copy_from_slice(energy);
        self.energy_reserve.copy_from_slice(reserve);
        Ok(())
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

    /// Neighbour grid over the plants, built once at construction.
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
    /// know where the scenery is. Not a runtime operation: plant sites are fixed once
    /// seeded, which is what lets the grid be built once.
    pub fn place_for_test(&mut self, positions: &[Vec3]) {
        assert_eq!(positions.len(), self.position.len(), "wrong plant count");
        self.position.copy_from_slice(positions);
        let mut cells = vec![0u32; self.position.len()];
        self.hash.rebuild(&self.position, &self.alive, &mut cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::ChemoParams;

    fn world() -> (Plants, SimParams) {
        filled(1.0)
    }

    /// A world stocked to `fill` of `max_energy`. Growth is only measurable from empty.
    fn filled(fill: f32) -> (Plants, SimParams) {
        let mut params = SimParams::default();
        params.plants.max_plants = 200;
        params.plants.initial_fill = fill;
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
