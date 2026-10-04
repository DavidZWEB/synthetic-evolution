//! The bite: who swings, whom a swing hits, what a hit takes, and recovering from both
//! (spec §4.2).
//!
//! Swings resolve in two passes, so that index order cannot decide who swings or who
//! dies: the tick fixes every swing and its target from the state at the start of step
//! 7, paying every cost, and only then lands the hits, mouthfuls in agent-index order
//! and wounds summed per victim in an order no slot chooses. Deliberately not here:
//! dying. A victim at health 0 joins the starved in step 10.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::energy::{self, Amount};
use crate::ids::AgentId;
use crate::math;
use crate::params::CombatParams;
use crate::spatial::SpatialHash;

/// Swings, hits, and kills since the world began: what the shells report about
/// predation (spec §7.9). Counts, not behaviour; nothing in the tick reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiteCounts {
    pub swings: u64,
    pub hits: u64,
    /// Agents that died at health 0, whoever landed the last hit.
    pub kills: u64,
}

/// One swing fixed by the first pass: who swung, whom it hits (`AgentId::NULL` for a
/// miss), and, once it lands, the damage it deals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Swing {
    pub biter: AgentId,
    pub target: AgentId,
    pub damage: f32,
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
    countable_cooldown(combat.cooldown_seconds, dt).unwrap_or(u32::MAX)
}

/// The whole ticks that cover `seconds` at timestep `dt`, or `None` past `u32::MAX`:
/// the count validation requires to fit, and the one a swing starts. `seconds` must be
/// finite and non-negative and `dt` positive, as validation ensures first.
pub(crate) fn countable_cooldown(seconds: f32, dt: f32) -> Option<u32> {
    ceil_ticks(1, seconds, dt)
}

/// A cooldown's remaining ticks after the timestep changes from `from_dt` to `to_dt`:
/// the same remaining time, rounded up so a retune never shortens it (spec §4.2).
///
/// `None` when that many ticks cannot be counted, which a retune must refuse.
pub(crate) fn rescale_cooldown(ticks: u32, from_dt: f32, to_dt: f32) -> Option<u32> {
    ceil_ticks(ticks, from_dt, to_dt)
}

/// `⌈count · numerator / denominator⌉` exactly, over the `f32` values as stored, or
/// `None` past `u32::MAX`.
///
/// Exact rather than in floating point because any float quotient can round down
/// onto a whole number first, and its ceiling then counts a tick short of the duration
/// (spec §4.2): 0.15 s over 0.05 s in `f32`, or a billion ticks in `f64`. Every stored
/// `f32` is a whole mantissa times a power of two, so the quotient is a ratio of
/// integers that `u128` divides exactly.
fn ceil_ticks(count: u32, numerator: f32, denominator: f32) -> Option<u32> {
    let (mantissa, exponent) = exact_parts(numerator);
    let (divisor, divisor_exponent) = exact_parts(denominator);
    debug_assert!(divisor > 0, "validated positive timestep");
    // Below 2^56: a 24-bit mantissa times a 32-bit count.
    let dividend = mantissa * u128::from(count);
    if dividend == 0 {
        return Some(0);
    }
    let shift = exponent - divisor_exponent;
    let quotient = if shift >= 0 {
        // With a divisor below 2^24, a shift of 57 already puts the quotient past 2^32,
        // and anything less keeps the shifted dividend inside u128.
        if shift >= 57 {
            return None;
        }
        (dividend << shift).div_ceil(divisor)
    } else if shift > -100 {
        dividend.div_ceil(divisor << -shift)
    } else {
        // The divisor passes 2^100 and every dividend: a positive fraction of a tick.
        1
    };
    u32::try_from(quotient).ok()
}

/// A finite, non-negative `f32` as the whole mantissa and power of two it stores.
fn exact_parts(x: f32) -> (u128, i32) {
    debug_assert!(x.is_finite() && x >= 0.0);
    let bits = x.to_bits();
    let fraction = u128::from(bits & 0x7f_ffff);
    match (bits >> 23) & 0xff {
        0 => (fraction, -149),
        biased => (fraction | 0x80_0000, biased as i32 - 150),
    }
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
    // A full circle takes every direction, whatever rounding does behind the biter.
    let full_circle = targets.arc >= core::f32::consts::PI;
    // Nearest images on the plane lie up to w/√2 apart, so a search capped at the world
    // still finds every target in reach, whatever a retune did to the size ranges.
    let search = (aim.reach + aim.radius + targets.largest).min(targets.world_size);
    let mut nearest: Option<(f64, u32)> = None;
    targets.hash.for_each_within(
        targets.positions,
        aim.position,
        search,
        |index, offset, _| {
            if index as usize == biter {
                return;
            }
            // Squared in f64 from the offset itself. An f32 square overflows at the
            // largest scales and underflows to zero at the smallest, where a victim far
            // out of reach, or two at different distances, would compare as equal.
            let (x, y, z) = (
                f64::from(offset.x),
                f64::from(offset.y),
                f64::from(offset.z),
            );
            let distance_squared = x * x + y * y + z * z;
            let limit = f64::from(aim.reach)
                + f64::from(aim.radius)
                + f64::from(targets.sizes[index as usize]);
            if distance_squared > limit * limit {
                return;
            }
            // The angle off the aim, measured directly. Comparing the dot product with
            // the distance times `cos(arc)` fails at narrow arcs, where the cosine
            // rounds to 1 and demands an alignment no rounded offset has.
            let across = offset.x * direction.y - offset.y * direction.x;
            if !full_circle && math::atan2(across.abs(), offset.dot(direction)) > targets.arc {
                return;
            }
            if nearest.is_none_or(|best| (distance_squared, index) < best) {
                nearest = Some((distance_squared, index));
            }
        },
    );
    nearest.map(|(_, index)| index as usize)
}

/// The health a hit removes: `attack_damage · g / s_victim`, clamped to `[0, 1]`,
/// where `g` is the biter's gape (mouth times its relative size) and `s_victim` the
/// victim's relative size (spec §3.5, §4.2).
pub(crate) fn damage(gape: f32, victim_scale: f32, combat: &CombatParams) -> f32 {
    (combat.attack_damage * gape / victim_scale).clamp(0.0, 1.0)
}

/// Lands every hit's damage, summed per victim in ascending order and subtracted
/// once. Subtracting hit by hit rounds differently in different orders, enough to
/// leave a victim alive at a sliver of health or dead at 0, and the order would be
/// the attackers' slots (spec §4.2). Reorders `swings`.
pub(crate) fn wound(swings: &mut [Swing], health: &mut [f32]) {
    // Damage is never negative, so its bits order as its value does; misses, aimed at
    // `AgentId::NULL`, sort last.
    swings.sort_unstable_by_key(|swing| (swing.target, swing.damage.to_bits()));
    let mut rest = &*swings;
    while let Some(first) = rest.first().filter(|swing| !swing.target.is_null()) {
        let count = rest
            .iter()
            .take_while(|swing| swing.target == first.target)
            .count();
        let total: f32 = rest[..count].iter().map(|swing| swing.damage).sum();
        let victim = &mut health[first.target.index()];
        *victim = (*victim - total).max(0.0);
        rest = &rest[count..];
    }
}

/// The two bodies in a hit, and the energy each holds.
pub(crate) struct Hit<'a> {
    pub biter_energy: &'a mut f32,
    pub biter_reserve: &'a mut f64,
    pub victim_energy: &'a mut f32,
    pub victim_reserve: &'a mut f64,
    /// Gape: mouth times the biter's relative size (spec §3.5).
    pub gape: f32,
}

/// What a landed hit's mouthful moved: the biter's share, and the rest, which left
/// the world.
pub(crate) struct Mouthful {
    pub kept: f64,
    pub dissipated: Amount,
}

/// Takes one hit's mouthful (spec §4.2). It asks `mouthful · g²` and takes what the
/// victim holds, of which the biter keeps `assimilation`; the rest leaves the world.
pub(crate) fn take_mouthful(hit: Hit<'_>, combat: &CombatParams) -> Mouthful {
    let ask = (combat.mouthful * hit.gape * hit.gape) as f64;
    let taken = ask.min(energy::total(*hit.victim_energy, *hit.victim_reserve));
    let kept = energy::transfer(
        hit.victim_energy,
        hit.victim_reserve,
        hit.biter_energy,
        hit.biter_reserve,
        taken * combat.assimilation as f64,
    );
    let dissipated = energy::take_amount(hit.victim_energy, hit.victim_reserve, taken - kept);
    Mouthful { kept, dissipated }
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
    fn the_slowest_regeneration_validation_accepts_still_heals_a_nearly_full_agent() {
        // One representable step of a health just short of full is the smallest a tick
        // may heal. Half of it rounds away and never heals, which is why validation
        // refuses it, and why a timestep retune that would shrink a step below it is
        // refused too.
        let step = f32::EPSILON / 2.0;
        let mut params = SimParams::default();
        params.world.dt = 0.25;
        params.combat.health_regen = 4.0 * step;
        assert_eq!(params.validate(), Ok(()));
        for start in [1.0 - step, 0.75] {
            let mut health = start;
            recover(&mut health, &mut 0, &params.combat, params.world.dt);
            assert_eq!(health, start + step, "{start}");
        }
        let mut slower = params.clone();
        slower.combat.health_regen = 2.0 * step;
        assert!(slower.validate().is_err());
        let mut health = 0.75;
        recover(&mut health, &mut 0, &slower.combat, slower.world.dt);
        assert_eq!(health, 0.75, "half a step rounds away");
        let mut finer = SimParams::default();
        finer.world.dt = 1e-6;
        let cell = SimParams::default().sensing.max_sense_radius();
        assert!(SimParams::default().check_retune(&finer, cell).is_err());
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
    fn a_new_timestep_keeps_each_cooldown_s_remaining_time() {
        let sixtieth = 1.0 / 60.0;
        assert_eq!(rescale_cooldown(29, sixtieth, 1.0 / 120.0), Some(58));
        assert_eq!(
            rescale_cooldown(29, sixtieth, 1.0 / 30.0),
            Some(15),
            "14.5 rounds up"
        );
        assert_eq!(rescale_cooldown(0, sixtieth, 1.0 / 120.0), Some(0));
        assert_eq!(rescale_cooldown(u32::MAX, 1.0, 1.0), Some(u32::MAX));
        assert_eq!(
            rescale_cooldown(29, sixtieth, 1e-10),
            None,
            "too many to count"
        );
    }

    #[test]
    fn a_cooldown_is_never_shorter_than_its_seconds() {
        let c = combat();
        assert_eq!(cooldown_ticks(&c, 1.0 / 60.0), 30);
        assert_eq!(cooldown_ticks(&c, 0.2), 3, "2.5 ticks round up");
        let tight = CombatParams {
            cooldown_seconds: 0.15,
            ..c.clone()
        };
        // In f32, 0.15 / 0.05 rounds to exactly 3, and 3 ticks are just short of it.
        assert_eq!(cooldown_ticks(&tight, 0.05), 4);
        let none = CombatParams {
            cooldown_seconds: 0.0,
            ..c
        };
        assert_eq!(cooldown_ticks(&none, 1.0 / 60.0), 0);
    }

    #[test]
    fn tick_counts_are_exact_where_a_float_quotient_rounds_onto_a_whole_number() {
        // f64 rounds each true quotient down onto a whole number, and its ceiling then
        // counts a tick short of the stored duration (spec §4.2).
        assert_eq!(
            countable_cooldown(119_362_880.0, 0.031_249_994),
            Some(3_819_612_844)
        );
        assert_eq!(
            rescale_cooldown(3_800_000_000, 0.023_821_32, 0.022_351_718),
            Some(4_049_846_155)
        );
        // The extremes: past any count, a sliver of one tick, and subnormals.
        assert_eq!(countable_cooldown(f32::MAX, f32::MIN_POSITIVE), None);
        assert_eq!(rescale_cooldown(u32::MAX, 2.0, 1.0), None);
        assert_eq!(countable_cooldown(f32::from_bits(1), f32::MAX), Some(1));
        assert_eq!(
            countable_cooldown(f32::from_bits(3), f32::from_bits(2)),
            Some(2)
        );
    }

    /// A normal `f32` in [2^-20, 2^20), from its unbiased exponent and fraction bits.
    fn normal(exponent: i32, fraction: u32) -> f32 {
        f32::from_bits(((exponent + 127) as u32) << 23 | fraction)
    }

    proptest::proptest! {
        #[test]
        fn tick_counts_are_the_exact_ceiling(
            count in proptest::prelude::any::<u32>(),
            numerator in (-20i32..20, 0u32..1 << 23),
            denominator in (-20i32..20, 0u32..1 << 23),
        ) {
            let n = normal(numerator.0, numerator.1);
            let d = normal(denominator.0, denominator.1);
            // Every value in range is a whole multiple of 2^-43, so scaling by 2^43 in
            // f64 gives the exact integers, by a route that never splits a float.
            let whole = |x: f32| (f64::from(x) * (1u64 << 43) as f64) as u128;
            let exact = (u128::from(count) * whole(n)).div_ceil(whole(d));
            proptest::prop_assert_eq!(ceil_ticks(count, n, d), u32::try_from(exact).ok());
        }
    }

    #[test]
    fn a_full_circle_bites_what_lies_directly_behind() {
        // Behind a biter heading 0.3 rad, the f32 dot product rounds to just below minus
        // the distance, the full circle's threshold, so testing the angle would miss.
        let at = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let behind = [at(50.0, 50.0), at(49.044_662, 49.704_48)];
        let pi = core::f32::consts::PI;
        assert_eq!(hit_from(&behind, &[1.0; 2], 0.3, pi), Some(1));
        // Short of a full circle, behind is out of the arc.
        assert_eq!(hit_from(&behind, &[1.0; 2], 0.3, pi - 0.01), None);
    }

    #[test]
    fn a_narrow_arc_bites_along_its_aim_and_a_zero_arc_only_dead_ahead() {
        // At 1e-4 rad the cosine rounds to exactly 1 in f32, so comparing the dot
        // product with the distance missed a target under a microradian off the aim.
        let at = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let along = [at(50.0, 50.0), at(52.30217, 50.712143)];
        assert_eq!(hit_from(&along, &[1.0; 2], 0.3, 1e-4), Some(1));
        // A zero arc takes only what lies exactly on the aim.
        let ahead = [at(50.0, 50.0), at(53.0, 50.0)];
        assert_eq!(hit_from(&ahead, &[1.0; 2], 0.0, 0.0), Some(1));
        let beside = [at(50.0, 50.0), at(53.0, 50.001)];
        assert_eq!(hit_from(&beside, &[1.0; 2], 0.0, 0.0), None);
    }

    #[test]
    fn reach_and_nearness_hold_where_f32_squares_underflow_to_zero() {
        // Bodies of radius 1e-25 with no reach: a victim 1e-23 away is fifty limits out,
        // yet both squares were 0 in f32 and it was hit. Two victims at different
        // distances likewise tied at 0, and the lower slot won. Placed near the origin,
        // where offsets this small are representable.
        let at = |x: f32| Vec3::new(1e-20 + x, 1e-20, 0.0);
        let tiny = [1e-25; 3];
        let quarter = core::f32::consts::FRAC_PI_4;
        assert_eq!(
            hit_reaching(0.0, &[at(0.0), at(1e-23)], &tiny, 0.0, quarter),
            None
        );
        // The farther victim takes the lower slot, so only true distances choose.
        assert_eq!(
            hit_reaching(1e-22, &[at(0.0), at(2e-23), at(1e-23)], &tiny, 0.0, quarter),
            Some(2)
        );
    }

    #[test]
    fn a_bite_finds_a_long_range_target_in_the_largest_world_validation_admits() {
        // Every squared distance at this scale stays finite, so reach and arc compare
        // against real numbers; past it, validation refuses the world.
        let positions = [Vec3::ZERO, Vec3::new(2e17, 0.0, 0.0)];
        let mut hash = SpatialHash::new(1e18, 1e17, 2);
        hash.rebuild(&positions, &[1; 2], &mut [0; 2]);
        let aim = Aim {
            position: positions[0],
            heading: 0.0,
            reach: 3e17,
            radius: 1.0,
        };
        let targets = Targets {
            positions: &positions,
            sizes: &[1.0; 2],
            hash: &hash,
            largest: 1.0,
            world_size: 1e18,
            arc: core::f32::consts::FRAC_PI_4,
        };
        assert_eq!(target(0, &aim, &targets), Some(1));
    }

    /// Agents in a 100-unit world, searched from slot 0 with a reach of 2.
    fn hit_from(positions: &[Vec3], sizes: &[f32], heading: f32, arc: f32) -> Option<usize> {
        hit_reaching(2.0, positions, sizes, heading, arc)
    }

    fn hit_reaching(
        reach: f32,
        positions: &[Vec3],
        sizes: &[f32],
        heading: f32,
        arc: f32,
    ) -> Option<usize> {
        let mut hash = SpatialHash::new(100.0, 10.0, positions.len() as u32);
        let alive = vec![1; positions.len()];
        let mut cells = vec![0; positions.len()];
        hash.rebuild(positions, &alive, &mut cells);
        let aim = Aim {
            position: positions[0],
            heading,
            reach,
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
        // A reach of half the world, the most an import may carry, still finds a
        // diagonal target 52.9 away: within 50 + 3 + 3, though past half the world.
        assert_eq!(
            hit_reaching(
                50.0,
                &[at(25.0, 50.0), at(74.0, 70.0)],
                &[3.0, 3.0],
                0.0,
                quarter
            ),
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
        assert_eq!(damage(2.0, 4.0, &c), 0.125, "0.25 · 2 / 4");
        assert_eq!(damage(2.0, 0.25, &c), 1.0, "clamped to full health");
        let (mut biter, mut biter_reserve) = (10.0f32, 0.0f64);
        let (mut victim, mut victim_reserve) = (100.0f32, 0.0f64);
        let mouthful = take_mouthful(
            Hit {
                biter_energy: &mut biter,
                biter_reserve: &mut biter_reserve,
                victim_energy: &mut victim,
                victim_reserve: &mut victim_reserve,
                gape: 2.0,
            },
            &c,
        );
        // A mouthful of 20 · 2², of which 0.75 is kept.
        assert_eq!((victim, victim_reserve), (20.0, 0.0));
        assert_eq!((biter, biter_reserve), (70.0, 0.0));
        assert_eq!(
            (mouthful.kept, mouthful.dissipated.approximate()),
            (60.0, 20.0)
        );

        // A victim holding less than the ask gives what it has.
        let mut biter = 0.0f32;
        let mut victim = 30.0f32;
        let mouthful = take_mouthful(
            Hit {
                biter_energy: &mut biter,
                biter_reserve: &mut 0.0,
                victim_energy: &mut victim,
                victim_reserve: &mut 0.0,
                gape: 2.0,
            },
            &c,
        );
        assert_eq!((victim, biter), (0.0, 22.5));
        assert_eq!(
            (mouthful.kept, mouthful.dissipated.approximate()),
            (22.5, 7.5)
        );
    }

    #[test]
    fn wounds_land_the_same_whatever_order_the_hits_came_in() {
        // Mouths 0.4, 0.8, and 2.8 deal 0.1, 0.2, and 0.7. Subtracted hit by hit, some
        // orders leave health exactly 0 and others a few billionths, which would let the
        // attackers' slots decide whether the victim dies (spec §4.2).
        let c = combat();
        let hits = [0.4, 0.8, 2.8].map(|mouth| damage(mouth, 1.0, &c));
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for order in orders {
            let mut swings = order.map(|k| Swing {
                biter: AgentId::new(k as u32 + 1),
                target: AgentId::new(0),
                damage: hits[k],
            });
            let mut health = [1.0f32, 1.0];
            wound(&mut swings, &mut health);
            assert_eq!(health[0].to_bits(), 0.0f32.to_bits(), "{order:?}");
            assert_eq!(health[1], 1.0, "{order:?}: an unhit agent was wounded");
        }
        // Misses land nothing, and each victim takes only its own hits.
        let mut swings = [
            Swing {
                biter: AgentId::new(0),
                target: AgentId::NULL,
                damage: 0.0,
            },
            Swing {
                biter: AgentId::new(2),
                target: AgentId::new(1),
                damage: 0.25,
            },
            Swing {
                biter: AgentId::new(3),
                target: AgentId::new(0),
                damage: 0.5,
            },
        ];
        let mut health = [1.0f32, 1.0];
        wound(&mut swings, &mut health);
        assert_eq!(health, [0.5, 0.75]);
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
