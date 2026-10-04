//! The bite: who swings, whom a swing hits, what a hit takes, and recovering from both
//! (spec §4.2).
//!
//! Swings resolve in two passes, so that index order cannot decide who swings or who
//! dies: the tick fixes every swing and its target from the state at the start of step
//! 7, paying every cost, and only then applies hits in agent-index order. Deliberately
//! not here: dying. A victim at health 0 joins the starved in step 10.

use glam::Vec3;

use crate::energy::{self, Amount};
use crate::ids::AgentId;
use crate::math;
use crate::params::CombatParams;
use crate::spatial::SpatialHash;

/// One swing fixed by the first pass: who swung, and whom it hits, or `AgentId::NULL`
/// for a miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Swing {
    pub biter: AgentId,
    pub target: AgentId,
}

/// Whether an agent swings this tick: its drive is above the gate, its cooldown has
/// run out, and it holds the swing's cost (spec §4.2).
pub(crate) fn swings(
    drive: f32,
    cooldown: u32,
    energy: f32,
    reserve: f64,
    combat: &CombatParams,
) -> bool {
    drive > combat.gate && cooldown == 0 && energy::at_least(energy, reserve, combat.attack_cost)
}

/// Whole ticks before a swinging agent can swing again: never fewer than
/// `cooldown_seconds`, which validation keeps countable.
pub(crate) fn cooldown_ticks(combat: &CombatParams, dt: f32) -> u32 {
    libm::ceilf(combat.cooldown_seconds / dt) as u32
}

/// Where a swing comes from and points.
pub(crate) struct Aim {
    pub position: Vec3,
    /// Radians: the biter's heading turned by its bite's azimuth.
    pub heading: f32,
    /// How far past both bodies the bite reaches.
    pub reach: f32,
    pub radius: f32,
}

/// The agents a swing can find, as they stand after step 5's movement.
pub(crate) struct Targets<'a> {
    pub positions: &'a [Vec3],
    pub sizes: &'a [f32],
    /// Built from `positions`, so it finds agents where they are now.
    pub hash: &'a SpatialHash,
    /// The largest radius a living body can have, so one search covers every victim.
    pub largest: f32,
    pub world_size: f32,
    pub arc: f32,
}

/// The agent a swing hits: the nearest other living agent whose centre lies within the
/// bite's reach past both bodies and within `arc` of its direction (spec §4.2), the
/// lower slot on a tie, so hash order cannot choose. `None` on a miss.
pub(crate) fn target(biter: usize, aim: &Aim, targets: &Targets<'_>) -> Option<usize> {
    let direction = Vec3::new(math::cos(aim.heading), math::sin(aim.heading), 0.0);
    let cos_arc = math::cos(targets.arc);
    // A retune may widen the size ranges past an old gene's reach; past half the world
    // the torus has no nearer image to search anyway.
    let search = (aim.reach + aim.radius + targets.largest).min(targets.world_size * 0.5);
    let mut nearest: Option<(f32, u32)> = None;
    targets.hash.for_each_within(
        targets.positions,
        aim.position,
        search,
        |index, offset, distance_squared| {
            if index as usize == biter {
                return;
            }
            let limit = aim.reach + aim.radius + targets.sizes[index as usize];
            if distance_squared > limit * limit
                || offset.dot(direction) < math::sqrt(distance_squared) * cos_arc
            {
                return;
            }
            if nearest.is_none_or(|best| (distance_squared, index) < best) {
                nearest = Some((distance_squared, index));
            }
        },
    );
    nearest.map(|(_, index)| index as usize)
}

/// The two bodies in a hit, and the energy each holds.
pub(crate) struct Hit<'a> {
    pub biter_energy: &'a mut f32,
    pub biter_reserve: &'a mut f64,
    pub victim_energy: &'a mut f32,
    pub victim_reserve: &'a mut f64,
    pub victim_health: &'a mut f32,
    /// Gape: mouth times the biter's relative size (spec §3.5).
    pub gape: f32,
    /// The victim's size relative to the reference body.
    pub victim_scale: f32,
}

/// Applies one hit and returns what it dissipated (spec §4.2). Damage is
/// `attack_damage · g / s_victim`, clamped to `[0, 1]`, and accumulates. The mouthful
/// asks `mouthful · g²` and takes what the victim holds, of which the biter keeps
/// `assimilation`; the rest leaves the world.
pub(crate) fn hit(hit: Hit<'_>, combat: &CombatParams) -> Amount {
    let damage = (combat.attack_damage * hit.gape / hit.victim_scale).clamp(0.0, 1.0);
    *hit.victim_health = (*hit.victim_health - damage).max(0.0);

    let ask = (combat.mouthful * hit.gape * hit.gape) as f64;
    let taken = ask.min(energy::total(*hit.victim_energy, *hit.victim_reserve));
    let kept = energy::transfer(
        hit.victim_energy,
        hit.victim_reserve,
        hit.biter_energy,
        hit.biter_reserve,
        taken * combat.assimilation as f64,
    );
    energy::take_amount(hit.victim_energy, hit.victim_reserve, taken - kept)
}

/// Step 9's recovery for one living agent: health regenerates by `health_regen` per
/// second up to full, but only above 0, so a lethal hit cannot heal before step 10
/// resolves it; and the cooldown counts down a tick (spec §2.4, §4.2).
pub(crate) fn recover(health: &mut f32, cooldown: &mut u32, combat: &CombatParams, dt: f32) {
    if *health > 0.0 {
        *health = (*health + combat.health_regen * dt).min(1.0);
    }
    *cooldown = cooldown.saturating_sub(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;

    fn combat() -> CombatParams {
        SimParams::default().combat
    }

    #[test]
    fn a_swing_needs_the_drive_the_cooldown_and_the_cost() {
        let c = combat();
        assert!(swings(0.51, 0, 8.0, 0.0, &c), "the cost exactly in hand");
        assert!(!swings(0.5, 0, 100.0, 0.0, &c), "resting at the gate");
        assert!(!swings(0.9, 1, 100.0, 0.0, &c), "still cooling down");
        assert!(!swings(0.9, 0, 7.5, 0.25, &c), "short of the cost");
        assert!(swings(0.9, 0, 7.5, 0.5, &c), "the reserve counts");
    }

    #[test]
    fn a_cooldown_is_never_shorter_than_its_seconds() {
        let c = combat();
        assert_eq!(cooldown_ticks(&c, 1.0 / 60.0), 30);
        assert_eq!(cooldown_ticks(&c, 0.2), 3, "2.5 ticks round up");
        let none = CombatParams {
            cooldown_seconds: 0.0,
            ..c
        };
        assert_eq!(cooldown_ticks(&none, 1.0 / 60.0), 0);
    }

    /// Agents of radius 1 in a 100-unit world, searched from slot 0.
    fn hit_from(positions: &[Vec3], sizes: &[f32], heading: f32, arc: f32) -> Option<usize> {
        let mut hash = SpatialHash::new(100.0, 10.0, positions.len() as u32);
        let alive = vec![1; positions.len()];
        let mut cells = vec![0; positions.len()];
        hash.rebuild(positions, &alive, &mut cells);
        let aim = Aim {
            position: positions[0],
            heading,
            reach: 2.0,
            radius: sizes[0],
        };
        let targets = Targets {
            positions,
            sizes,
            hash: &hash,
            largest: 3.0,
            world_size: 100.0,
            arc,
        };
        target(0, &aim, &targets)
    }

    #[test]
    fn a_swing_hits_the_nearest_agent_ahead_within_reach_of_both_bodies() {
        let quarter = core::f32::consts::FRAC_PI_4;
        let at = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let ones = [1.0; 5];
        // Slot 3 is nearest but outside the arc, slot 1 is in reach but further than
        // slot 2, and slot 4 is behind.
        let crowd = [
            at(50.0, 50.0),
            at(53.0, 50.0),
            at(52.0, 51.5),
            at(51.0, 48.5),
            at(45.0, 50.0),
        ];
        assert_eq!(hit_from(&crowd, &ones, 0.0, quarter), Some(2));
        // Facing the other way, nothing within the arc is in reach.
        assert_eq!(
            hit_from(&crowd, &ones, core::f32::consts::PI, quarter),
            None
        );
        // Reach is past both bodies: 2 + 1 + 1 misses at 4.5, and a victim of radius 3
        // is in reach at 5.5.
        assert_eq!(
            hit_from(&[at(50.0, 50.0), at(54.5, 50.0)], &ones, 0.0, quarter),
            None
        );
        assert_eq!(
            hit_from(&[at(50.0, 50.0), at(55.5, 50.0)], &[1.0, 3.0], 0.0, quarter),
            Some(1)
        );
        // Mirror images tie on distance, and the lower slot wins whatever the hash order.
        let mirror = [at(50.0, 50.0), at(52.0, 48.5), at(52.0, 51.5)];
        assert_eq!(hit_from(&mirror, &ones, 0.0, quarter), Some(1));
        // A biter never bites itself, even alone.
        assert_eq!(hit_from(&[at(50.0, 50.0)], &ones, 0.0, quarter), None);
        // Across the seam, the minimum image is in reach.
        assert_eq!(
            hit_from(&[at(99.0, 50.0), at(1.5, 50.0)], &ones, 0.0, quarter),
            Some(1)
        );
    }

    #[test]
    fn a_hit_wounds_by_gape_over_size_and_moves_exactly_the_mouthful() {
        // Every value is exact in f32, so the transfers are too; the shipped 0.8 is not.
        let c = CombatParams {
            assimilation: 0.75,
            ..combat()
        };
        let (mut biter, mut biter_reserve) = (10.0f32, 0.0f64);
        let (mut victim, mut victim_reserve) = (100.0f32, 0.0f64);
        let mut health = 1.0f32;
        let dissipated = hit(
            Hit {
                biter_energy: &mut biter,
                biter_reserve: &mut biter_reserve,
                victim_energy: &mut victim,
                victim_reserve: &mut victim_reserve,
                victim_health: &mut health,
                gape: 2.0,
                victim_scale: 4.0,
            },
            &c,
        );
        // Damage 0.25 · 2 / 4; a mouthful of 20 · 2², of which 0.75 is kept.
        assert_eq!(health, 0.875);
        assert_eq!((victim, victim_reserve), (20.0, 0.0));
        assert_eq!((biter, biter_reserve), (70.0, 0.0));
        assert_eq!(dissipated.approximate(), 20.0);

        // A victim holding less than the ask gives what it has, and health floors at 0.
        let mut biter = 0.0f32;
        let mut victim = 30.0f32;
        let mut health = 0.25f32;
        let dissipated = hit(
            Hit {
                biter_energy: &mut biter,
                biter_reserve: &mut 0.0,
                victim_energy: &mut victim,
                victim_reserve: &mut 0.0,
                victim_health: &mut health,
                gape: 2.0,
                victim_scale: 0.5,
            },
            &c,
        );
        assert_eq!((health, victim, biter), (0.0, 0.0, 22.5));
        assert_eq!(dissipated.approximate(), 7.5);
    }

    #[test]
    fn health_regenerates_only_above_zero_and_cooldowns_count_down() {
        let c = combat();
        let mut cooldown = 2;
        let mut wounded = 0.5;
        recover(&mut wounded, &mut cooldown, &c, 0.5);
        assert_eq!((wounded, cooldown), (0.51, 1));
        // Lethal damage cannot heal before step 10 resolves it.
        let mut killed = 0.0;
        recover(&mut killed, &mut cooldown, &c, 0.5);
        assert_eq!((killed, cooldown), (0.0, 0));
        let mut nearly = 0.999;
        recover(&mut nearly, &mut cooldown, &c, 0.5);
        assert_eq!((nearly, cooldown), (1.0, 0), "capped at full, and no wrap");
    }
}
