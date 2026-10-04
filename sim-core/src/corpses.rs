//! Corpses: the energy a dead agent leaves behind for others to eat (spec §5.1).
//!
//! A fixed pool of slots, each a position and a compensated energy pair. A death moves
//! `energy_fraction` of the agent's energy into a corpse where it fell; eating takes from
//! it exactly as from a plant; decomposition drains it into dissipation until it falls
//! below `min_energy` and its slot frees. Slots are reused last-freed-first, so which
//! slot a corpse takes is a function of the order deaths and removals happened in, which
//! the tick fixes (spec §2.4).
//!
//! Deliberately not here: deciding who dies (step 10, `World::despawn`), who eats
//! (`feeding`), or returning decomposed energy to plants, which Phase 3 does not do.

use glam::Vec3;

use crate::energy;
use crate::params::{CorpseParams, SimParams};
use crate::spatial::SpatialHash;

/// Every corpse slot in the world, live or free.
#[derive(Clone, Debug)]
pub struct Corpses {
    position: Vec<Vec3>,
    energy: Vec<f32>,
    energy_reserve: Vec<f64>,
    /// One byte per slot: 1 while a corpse lies there.
    alive: Vec<u8>,
    /// Free slots, popped from the end; reserved at capacity so freeing never allocates.
    free: Vec<u32>,
    /// Corpse shares dissipated whole because every slot was taken.
    refused: u64,
    /// Grid-cell scratch, kept so a rebuild never allocates.
    cells: Vec<u32>,
    hash: SpatialHash,
}

/// A checkpoint's corpse state, borrowed for validation and restore.
pub(crate) struct SavedCorpses<'a> {
    pub position: &'a [Vec3],
    pub energy: &'a [f32],
    pub reserve: &'a [f64],
    pub alive: &'a [u8],
    pub free: &'a [u32],
    pub refused: u64,
}

impl Corpses {
    /// An empty pool of `max_corpses` slots.
    pub fn new(params: &SimParams) -> Self {
        let count = params.corpses.max_corpses as usize;
        Self {
            position: vec![Vec3::ZERO; count],
            energy: vec![0.0; count],
            energy_reserve: vec![0.0; count],
            alive: vec![0; count],
            // Reversed, so the first corpse takes slot 0.
            free: (0..count as u32).rev().collect(),
            refused: 0,
            cells: vec![0; count],
            hash: SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                count as u32,
            ),
        }
    }

    /// Moves `energy_fraction` of a dying agent's energy into a corpse at `at`, and
    /// returns how much moved. Nothing moves when the share is below `min_energy` — a
    /// starved agent dies empty — or when every slot is taken, which is counted. The
    /// caller dissipates whatever the agent still holds.
    pub(crate) fn leave(
        &mut self,
        at: Vec3,
        source: &mut f32,
        source_reserve: &mut f64,
        params: &CorpseParams,
    ) -> f64 {
        let share = energy::total(*source, *source_reserve) * params.energy_fraction as f64;
        if share <= 0.0 || share < params.min_energy as f64 {
            return 0.0;
        }
        let Some(slot) = self.free.pop() else {
            self.refused += 1;
            return 0.0;
        };
        let i = slot as usize;
        self.position[i] = at;
        self.energy[i] = 0.0;
        self.energy_reserve[i] = 0.0;
        self.alive[i] = 1;
        let moved = energy::transfer(
            source,
            source_reserve,
            &mut self.energy[i],
            &mut self.energy_reserve[i],
            share,
        );
        self.rebuild();
        moved
    }

    /// Decomposes every corpse by `decay` of its energy per second, removing those left
    /// below `min_energy`, and returns the energy dissipated. Slot order, so removals
    /// free slots in an order the tick fixes.
    pub(crate) fn decay(&mut self, params: &CorpseParams, dt: f32) -> f64 {
        let rate = (params.decay as f64 * dt as f64).clamp(0.0, 1.0);
        let floor = params.min_energy as f64;
        let mut dissipated = 0.0;
        let mut removed = false;
        for i in 0..self.alive.len() {
            if self.alive[i] == 0 {
                continue;
            }
            let held = energy::total(self.energy[i], self.energy_reserve[i]);
            let lost = if held - held * rate < floor {
                removed = true;
                self.alive[i] = 0;
                self.free.push(i as u32);
                f64::MAX
            } else {
                held * rate
            };
            dissipated +=
                energy::take_amount(&mut self.energy[i], &mut self.energy_reserve[i], lost)
                    .approximate();
        }
        if removed {
            self.rebuild();
        }
        dissipated
    }

    fn rebuild(&mut self) {
        self.hash
            .rebuild(&self.position, &self.alive, &mut self.cells);
    }

    /// Total energy held across every corpse.
    pub fn total_energy(&self) -> f64 {
        self.energy
            .iter()
            .zip(&self.energy_reserve)
            .map(|(&energy, &reserve)| energy::total(energy, reserve))
            .sum()
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

    #[inline]
    pub fn alive(&self) -> &[u8] {
        &self.alive
    }

    #[inline]
    pub(crate) fn free_slots(&self) -> &[u32] {
        &self.free
    }

    /// Corpse shares dissipated whole because the pool was full.
    #[inline]
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Live corpses.
    pub fn count(&self) -> usize {
        self.alive.len() - self.free.len()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.alive.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }

    /// Neighbour grid over live corpses, rebuilt whenever one appears or is removed.
    #[inline]
    pub fn hash(&self) -> &SpatialHash {
        &self.hash
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
        if self.alive.get(index) != Some(&1) {
            return 0.0;
        }
        energy::transfer(
            &mut self.energy[index],
            &mut self.energy_reserve[index],
            destination,
            destination_reserve,
            wanted as f64,
        )
    }

    /// Restores saved corpses over an empty pool of the same capacity.
    ///
    /// Untrusted input: lengths match the pool, every live slot holds finite,
    /// non-negative energy on the world's plane, dead slots hold nothing, and the free
    /// list names each dead slot exactly once. A refused restore leaves the pool empty.
    pub(crate) fn restore(
        &mut self,
        saved: SavedCorpses<'_>,
        size: f32,
    ) -> Result<(), &'static str> {
        let count = self.len();
        if [
            saved.position.len(),
            saved.energy.len(),
            saved.reserve.len(),
            saved.alive.len(),
        ]
        .iter()
        .any(|&len| len != count)
        {
            return Err("corpse state does not match the corpse pool");
        }
        for i in 0..count {
            let (p, e, r) = (saved.position[i], saved.energy[i], saved.reserve[i]);
            let valid = match saved.alive[i] {
                1 => {
                    p.is_finite()
                        && p.z == 0.0
                        && (0.0..=size).contains(&p.x)
                        && (0.0..=size).contains(&p.y)
                        && e.is_finite()
                        && e >= 0.0
                        && r.is_finite()
                }
                0 => e == 0.0 && r == 0.0,
                _ => false,
            };
            if !valid {
                return Err("corpse slots must be live on the plane or empty");
            }
        }
        let mut listed = vec![false; count];
        for &slot in saved.free {
            let slot = slot as usize;
            if slot >= count || saved.alive[slot] == 1 || std::mem::replace(&mut listed[slot], true)
            {
                return Err("corpse free list must name each empty slot once");
            }
        }
        if listed
            .iter()
            .zip(saved.alive)
            .any(|(&free, &alive)| !free && alive == 0)
        {
            return Err("corpse free list must name each empty slot once");
        }
        self.position.copy_from_slice(saved.position);
        self.energy.copy_from_slice(saved.energy);
        self.energy_reserve.copy_from_slice(saved.reserve);
        self.alive.copy_from_slice(saved.alive);
        self.free.clear();
        self.free.extend_from_slice(saved.free);
        self.refused = saved.refused;
        self.rebuild();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(slots: u32) -> (Corpses, SimParams) {
        let mut params = SimParams::default();
        params.corpses.max_corpses = slots;
        params.corpses.energy_fraction = 0.6;
        params.corpses.min_energy = 1.0;
        params.corpses.decay = 0.5;
        (Corpses::new(&params), params)
    }

    #[test]
    fn a_death_leaves_its_share_and_keeps_the_rest_with_the_agent() {
        let (mut corpses, params) = pool(2);
        let (mut energy, mut reserve) = (100.0f32, 0.0f64);
        let moved = corpses.leave(
            Vec3::new(5.0, 6.0, 0.0),
            &mut energy,
            &mut reserve,
            &params.corpses,
        );
        assert!((moved - 60.0).abs() < 1e-4);
        assert!((energy::total(energy, reserve) - 40.0).abs() < 1e-4);
        assert_eq!(corpses.count(), 1);
        assert_eq!(corpses.alive()[0], 1, "the first corpse takes slot 0");
        assert!((corpses.total_energy() - 60.0).abs() < 1e-4);
    }

    #[test]
    fn a_starved_agent_leaves_nothing() {
        let (mut corpses, params) = pool(2);
        let (mut energy, mut reserve) = (0.5f32, 0.0f64);
        assert_eq!(
            corpses.leave(Vec3::ZERO, &mut energy, &mut reserve, &params.corpses),
            0.0
        );
        assert_eq!(energy, 0.5, "a refused share stays with the agent");
        assert_eq!(corpses.count(), 0);
    }

    #[test]
    fn a_full_pool_refuses_and_counts() {
        let (mut corpses, params) = pool(1);
        for _ in 0..2 {
            let (mut energy, mut reserve) = (100.0f32, 0.0f64);
            corpses.leave(Vec3::ZERO, &mut energy, &mut reserve, &params.corpses);
        }
        assert_eq!(corpses.count(), 1);
        assert_eq!(corpses.refused(), 1);
    }

    #[test]
    fn decay_dissipates_exactly_what_corpses_lose_and_frees_the_spent() {
        let (mut corpses, params) = pool(2);
        let (mut energy, mut reserve) = (100.0f32, 0.0f64);
        corpses.leave(Vec3::ZERO, &mut energy, &mut reserve, &params.corpses);
        let before = corpses.total_energy();
        let mut dissipated = 0.0;
        // 0.5 per second at one-second steps halves the corpse each time until it is
        // below a joule and is removed.
        for _ in 0..10 {
            dissipated += corpses.decay(&params.corpses, 1.0);
        }
        assert!((before - corpses.total_energy() - dissipated).abs() < 1e-9);
        assert_eq!(corpses.count(), 0, "a spent corpse was not removed");
        assert_eq!(corpses.total_energy(), 0.0);
        assert!((dissipated - before).abs() < 1e-9);
    }

    #[test]
    fn freed_slots_are_reused_last_freed_first() {
        let (mut corpses, params) = pool(3);
        for _ in 0..2 {
            let (mut energy, mut reserve) = (100.0f32, 0.0f64);
            corpses.leave(Vec3::ZERO, &mut energy, &mut reserve, &params.corpses);
        }
        let mut quick = params.corpses.clone();
        quick.min_energy = 1e9;
        corpses.decay(&quick, 1.0);
        assert_eq!(corpses.count(), 0);
        let (mut energy, mut reserve) = (100.0f32, 0.0f64);
        corpses.leave(Vec3::ZERO, &mut energy, &mut reserve, &params.corpses);
        assert_eq!(
            corpses.alive()[1],
            1,
            "slot 1 was freed last, so it is reused first"
        );
    }

    #[test]
    fn the_grid_finds_a_new_corpse() {
        let (mut corpses, params) = pool(2);
        let (mut energy, mut reserve) = (100.0f32, 0.0f64);
        let at = Vec3::new(300.0, 400.0, 0.0);
        corpses.leave(at, &mut energy, &mut reserve, &params.corpses);
        let mut found = false;
        corpses
            .hash()
            .for_each_within(corpses.position(), at, 1.0, |index, _, _| {
                found |= index == 0
            });
        assert!(found);
    }

    #[test]
    fn restore_refuses_an_inconsistent_free_list() {
        let (mut corpses, params) = pool(2);
        let position = [Vec3::ZERO; 2];
        let saved = |free: &'static [u32]| SavedCorpses {
            position: &position,
            energy: &[0.0, 0.0],
            reserve: &[0.0, 0.0],
            alive: &[0, 0],
            free,
            refused: 0,
        };
        assert!(corpses.restore(saved(&[1, 0]), params.world.size).is_ok());
        assert!(
            corpses.restore(saved(&[1]), params.world.size).is_err(),
            "slot 0 is lost"
        );
        assert!(
            corpses.restore(saved(&[1, 1]), params.world.size).is_err(),
            "slot 1 twice"
        );
    }
}
