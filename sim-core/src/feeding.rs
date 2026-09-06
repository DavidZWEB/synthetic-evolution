//! Eating: moving energy from a plant into the agent touching it.
//!
//! Step 7 of the tick, and the first **transfer** in the simulation — energy changes
//! hands without entering or leaving the world. That distinction is the whole reason
//! the ledger records flows rather than events: a transfer cancels, so one written
//! wrongly shows up as drift without any test having to predict the mistake (spec §5.1).
//!
//! An agent eats the *nearest* plant in reach rather than every plant at once. It is the
//! more physical reading of "absorb food in contact radius" (spec §4.2), and it means a
//! crowded patch is not worth more per tick than a single plant — so a population cannot
//! feed faster simply by standing where plants overlap.
//!
//! Deliberately not here: whether the agent asked. The gate on the ingest drive belongs
//! with the caller that reads the intent buffer, and biting other agents is Phase 3.

use glam::Vec3;

use crate::plants::Plants;

/// The nearest plant within `reach` of `at`, or `None` if there is nothing to eat.
///
/// Ties go to whichever the grid visits first. The plant grid is built once and never
/// rebuilt, so that is stable for the life of the world rather than an artifact of when
/// the query happened to run.
pub fn nearest(at: Vec3, reach: f32, plants: &Plants) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    plants
        .hash()
        .for_each_within(plants.position(), at, reach, |index, _, d2| {
            let index = index as usize;
            // An empty plant is still a plant, but there is nothing to take from it and
            // choosing it would block a fuller one right beside it.
            if plants.energy_at(index) <= 0.0 {
                return;
            }
            if best.is_none_or(|(_, best_d2)| d2 < best_d2) {
                best = Some((index, d2));
            }
        });
    best.map(|(index, _)| index)
}

/// Moves up to `wanted` from the nearest plant in reach into `agent_energy`.
///
/// Both stores are `f32`, so independently debiting and crediting a nominal amount can
/// round to different endpoint deltas. The agent's reserve keeps any debit too small
/// for the destination to represent and applies it on a later transfer, preserving the
/// closed economy without permanently stalling at a float boundary (spec §5.1).
pub fn ingest(
    at: Vec3,
    reach: f32,
    wanted: f32,
    agent_energy: &mut f32,
    agent_energy_reserve: &mut f64,
    plants: &mut Plants,
) -> f64 {
    if wanted <= 0.0 {
        return 0.0;
    }
    // Found first, taken second: the search borrows the plants to read and the take
    // borrows them to write, and they cannot overlap.
    let Some(index) = nearest(at, reach, plants) else {
        return 0.0;
    };
    plants.transfer_to(index, agent_energy, agent_energy_reserve, wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use crate::rng::Rng;

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
        ingest(
            at,
            reach,
            wanted,
            &mut agent_energy,
            &mut agent_reserve,
            plants,
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
}
