//! Eating: moving energy from a plant or a corpse into the agent touching it.
//!
//! Step 7 of the tick, and the first **transfer** in the simulation — energy changes
//! hands without entering or leaving the world. That distinction is the whole reason
//! the ledger records flows rather than events: a transfer cancels, so one written
//! wrongly shows up as drift without any test having to predict the mistake (spec §5.1).
//!
//! An agent eats the *nearest* food in reach rather than everything at once. It is the
//! more physical reading of "absorb food/corpse in contact radius" (spec §4.2), and it
//! means a crowded patch is not worth more per tick than a single plant — so a
//! population cannot feed faster simply by standing where food overlaps. Plants and
//! corpses compete on distance to their edge; a tie goes to the plant.
//!
//! Deliberately not here: whether the agent asked. The gate on the ingest drive belongs
//! with the caller that reads the intent buffer, and biting is its own step.

use glam::Vec3;

use crate::corpses::Corpses;
use crate::plants::Plants;
use crate::spatial::SpatialHash;

/// The food one agent can reach this tick. Each reach is the agent's body plus that
/// food's radius plus the feeding reach, so comparing distance minus reach compares
/// distance to the food's edge.
pub struct Larder<'a> {
    pub plants: &'a mut Plants,
    pub plant_reach: f32,
    pub corpses: &'a mut Corpses,
    pub corpse_reach: f32,
}

/// The nearest entry within `reach` that still holds food, and its squared distance.
fn nearest_within(
    hash: &SpatialHash,
    positions: &[Vec3],
    at: Vec3,
    reach: f32,
    has_food: impl Fn(usize) -> bool,
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    hash.for_each_within(positions, at, reach, |index, _, d2| {
        let index = index as usize;
        // Empty food is still there, but choosing it would block a fuller one beside it.
        if !has_food(index) {
            return;
        }
        if best.is_none_or(|(_, best_d2)| d2 < best_d2) {
            best = Some((index, d2));
        }
    });
    best
}

/// The nearest plant within `reach` of `at`, or `None` if there is nothing to eat.
///
/// Ties go to whichever the grid visits first. The plant grid is rebuilt from positions
/// in plant-index order whenever a plant reseeds, so that is a function of where the
/// plants are rather than an artifact of when the query happened to run.
pub fn nearest(at: Vec3, reach: f32, plants: &Plants) -> Option<usize> {
    nearest_within(plants.hash(), plants.position(), at, reach, |index| {
        plants.energy_at(index) > 0.0
    })
    .map(|(index, _)| index)
}

/// Moves up to `wanted` from the nearest food in reach, plant or corpse, into
/// `agent_energy`.
///
/// Both stores are `f32`, so independently debiting and crediting a nominal amount can
/// round to different endpoint deltas. The agent's reserve keeps any debit too small
/// for the destination to represent and applies it on a later transfer, preserving the
/// closed economy without permanently stalling at a float boundary (spec §5.1).
pub fn ingest(
    at: Vec3,
    wanted: f32,
    agent_energy: &mut f32,
    agent_energy_reserve: &mut f64,
    larder: &mut Larder<'_>,
) -> f64 {
    if wanted <= 0.0 {
        return 0.0;
    }
    // Found first, taken second: the search borrows the food to read and the take
    // borrows it to write, and they cannot overlap.
    let plants = &*larder.plants;
    let plant = nearest_within(
        plants.hash(),
        plants.position(),
        at,
        larder.plant_reach,
        |i| plants.energy_at(i) > 0.0,
    )
    .map(|(index, d2)| (index, d2.sqrt() - larder.plant_reach));
    let corpses = &*larder.corpses;
    let corpse = corpses
        .hash()
        .and_then(|hash| {
            nearest_within(hash, corpses.position(), at, larder.corpse_reach, |i| {
                corpses.energy_at(i) > 0.0
            })
        })
        .map(|(index, d2)| (index, d2.sqrt() - larder.corpse_reach));
    match (plant, corpse) {
        (Some((index, plant_edge)), Some((_, corpse_edge))) if plant_edge <= corpse_edge => larder
            .plants
            .transfer_to(index, agent_energy, agent_energy_reserve, wanted),
        (_, Some((index, _))) => {
            larder
                .corpses
                .transfer_to(index, agent_energy, agent_energy_reserve, wanted)
        }
        (Some((index, _)), None) => {
            larder
                .plants
                .transfer_to(index, agent_energy, agent_energy_reserve, wanted)
        }
        (None, None) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use crate::rng::Rng;

    fn no_corpses() -> Corpses {
        let mut params = SimParams::default();
        params.corpses.max_corpses = 0;
        Corpses::new(&params)
    }

    /// A world of plants at exactly these positions, each holding `energy`.
    fn plants_at(positions: &[Vec3], energy: f32) -> Plants {
        let mut params = SimParams::default();
        params.plants.max_plants = positions.len() as u32;
        let mut plants = Plants::new(&params, &mut Rng::from_seed(1));
        plants.place_for_test(positions);
        for _ in 0..1_000_000 {
            if plants.total_energy() >= energy as f64 * positions.len() as f64 {
                break;
            }
            plants.grow(&params.plants, params.world.dt);
        }
        plants
    }

    fn ingest_once(at: Vec3, reach: f32, wanted: f32, plants: &mut Plants) -> f64 {
        let mut agent_energy = 0.0;
        let mut agent_reserve = 0.0;
        let before = plants.total_energy();
        let mut corpses = no_corpses();
        ingest(
            at,
            wanted,
            &mut agent_energy,
            &mut agent_reserve,
            &mut Larder {
                plants,
                plant_reach: reach,
                corpses: &mut corpses,
                corpse_reach: reach,
            },
        );
        let removed = before - plants.total_energy();
        assert_eq!(removed, agent_energy as f64 + agent_reserve);
        removed
    }

    #[test]
    fn an_agent_eats_the_plant_it_is_touching() {
        let mut plants = plants_at(&[Vec3::new(500.0, 500.0, 0.0)], 60.0);
        let before = plants.total_energy();
        let taken = ingest_once(Vec3::new(501.0, 500.0, 0.0), 5.0, 2.0, &mut plants);
        assert!((taken - 2.0).abs() < 1e-4, "took {taken}");
        assert!((plants.total_energy() - (before - taken)).abs() < 1e-3);
    }

    #[test]
    fn nothing_in_reach_means_nothing_taken() {
        let mut plants = plants_at(&[Vec3::new(500.0, 500.0, 0.0)], 60.0);
        let before = plants.total_energy();
        assert_eq!(
            ingest_once(Vec3::new(700.0, 500.0, 0.0), 5.0, 2.0, &mut plants),
            0.0
        );
        assert_eq!(plants.total_energy(), before);
    }

    #[test]
    fn the_nearest_plant_is_the_one_eaten() {
        // Grid order is not distance order. Eating whichever the walk reached first
        // would make what an agent feeds on depend on the hash's layout.
        let mut plants = plants_at(
            &[Vec3::new(530.0, 500.0, 0.0), Vec3::new(505.0, 500.0, 0.0)],
            60.0,
        );
        let before: Vec<f32> = plants.energy().to_vec();
        ingest_once(Vec3::new(500.0, 500.0, 0.0), 40.0, 3.0, &mut plants);
        assert_eq!(plants.energy()[0], before[0], "ate the far plant");
        assert!(plants.energy()[1] < before[1], "did not eat the near plant");
    }

    #[test]
    fn an_empty_plant_is_skipped_for_one_that_has_something() {
        // An emptied site stays in the world and stays visible, so without this an
        // agent could sit on a bare patch drawing nothing while food sat beside it.
        let mut plants = plants_at(
            &[Vec3::new(502.0, 500.0, 0.0), Vec3::new(520.0, 500.0, 0.0)],
            60.0,
        );
        plants.take(0, 1e9);
        let taken = ingest_once(Vec3::new(500.0, 500.0, 0.0), 40.0, 3.0, &mut plants);
        assert!(
            (taken - 3.0).abs() < 1e-4,
            "took {taken} from the fuller plant"
        );
        assert!(plants.energy()[1] < 60.0);
    }

    #[test]
    fn a_plant_cannot_give_more_than_it_holds() {
        // The transfer is bounded by what exists, or two agents sharing a plant in one
        // tick would each draw a full portion and conjure the difference (spec §5.1).
        let mut plants = plants_at(&[Vec3::new(500.0, 500.0, 0.0)], 60.0);
        let held = plants.total_energy();
        let first = ingest_once(Vec3::new(500.0, 500.0, 0.0), 5.0, 1e9, &mut plants);
        let second = ingest_once(Vec3::new(500.0, 500.0, 0.0), 5.0, 1e9, &mut plants);
        assert!((first - held).abs() < 1e-3, "took {first} of {held}");
        assert_eq!(second, 0.0, "an emptied plant kept giving");
        assert!(plants.total_energy() < 1e-3);
    }

    #[test]
    fn eating_never_creates_energy() {
        let mut plants = plants_at(
            &[
                Vec3::new(500.0, 500.0, 0.0),
                Vec3::new(504.0, 500.0, 0.0),
                Vec3::new(508.0, 500.0, 0.0),
            ],
            60.0,
        );
        let before = plants.total_energy();
        let mut taken = 0.0;
        for i in 0..500 {
            let at = Vec3::new(500.0 + (i % 9) as f32, 500.0, 0.0);
            taken += ingest_once(at, 6.0, 0.7, &mut plants);
        }
        assert!(
            (plants.total_energy() + taken - before).abs() < 1e-2,
            "{before} -> {} held plus {taken} taken",
            plants.total_energy()
        );
    }

    #[test]
    fn a_negative_appetite_takes_nothing() {
        // The drive comes from a neuron and the rate from params; neither is bounded
        // below by anything but this. A negative take would push energy *into* a plant
        // from nowhere.
        let mut plants = plants_at(&[Vec3::new(500.0, 500.0, 0.0)], 60.0);
        let before = plants.total_energy();
        assert_eq!(
            ingest_once(Vec3::new(500.0, 500.0, 0.0), 5.0, -10.0, &mut plants),
            0.0
        );
        assert_eq!(plants.total_energy(), before);
    }

    /// A pool holding one corpse of `energy` at `at`.
    fn corpse_at(at: Vec3, energy: f32) -> Corpses {
        let mut params = SimParams::default();
        params.corpses.max_corpses = 1;
        params.corpses.energy_fraction = 1.0;
        let mut corpses = Corpses::new(&params);
        let (mut source, mut reserve) = (energy, 0.0);
        corpses.leave(at, &mut source, &mut reserve, &params.corpses);
        corpses.settle();
        corpses
    }

    fn eat(at: Vec3, plants: &mut Plants, corpses: &mut Corpses) -> f64 {
        let (mut energy, mut reserve) = (0.0, 0.0);
        ingest(
            at,
            2.0,
            &mut energy,
            &mut reserve,
            &mut Larder {
                plants,
                plant_reach: 5.0,
                corpses,
                corpse_reach: 5.0,
            },
        )
    }

    #[test]
    fn a_nearer_corpse_is_eaten_before_a_plant() {
        let mut plants = plants_at(&[Vec3::new(504.0, 500.0, 0.0)], 60.0);
        let mut corpses = corpse_at(Vec3::new(501.0, 500.0, 0.0), 50.0);
        let plant_before = plants.total_energy();
        let taken = eat(Vec3::new(500.0, 500.0, 0.0), &mut plants, &mut corpses);
        assert!(taken > 0.0);
        assert_eq!(plants.total_energy(), plant_before, "ate the farther plant");
        assert!((corpses.total_energy() - (50.0 - taken)).abs() < 1e-4);
    }

    #[test]
    fn a_tie_goes_to_the_plant() {
        let at = Vec3::new(500.0, 500.0, 0.0);
        let mut plants = plants_at(&[Vec3::new(503.0, 500.0, 0.0)], 60.0);
        let mut corpses = corpse_at(Vec3::new(497.0, 500.0, 0.0), 50.0);
        eat(at, &mut plants, &mut corpses);
        assert_eq!(
            corpses.total_energy(),
            50.0,
            "a tied corpse was eaten first"
        );
    }

    #[test]
    fn a_corpse_alone_feeds_and_conserves() {
        let mut plants = plants_at(&[Vec3::new(900.0, 900.0, 0.0)], 60.0);
        let mut corpses = corpse_at(Vec3::new(500.0, 501.0, 0.0), 50.0);
        let before = corpses.total_energy() + plants.total_energy();
        let taken = eat(Vec3::new(500.0, 500.0, 0.0), &mut plants, &mut corpses);
        assert!(taken > 0.0, "a corpse in reach fed nothing");
        assert!((corpses.total_energy() + plants.total_energy() + taken - before).abs() < 1e-6);
    }
}
