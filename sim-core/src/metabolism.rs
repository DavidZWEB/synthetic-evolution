//! What it costs to be alive for one tick.
//!
//! Spec §5.2's cost function, and the reason it is load-bearing rather than flavour:
//! every term here is what makes some strategy lose. Without `k_brain` and `k_sensor`,
//! "grow every organ" has no downside and genomes bloat until the sim crawls; without
//! `base`, sitting still is free and nothing ever has to forage; without `k_move`,
//! sprinting everywhere is as cheap as drifting. None of these terms is a tuning knob
//! to be relaxed when a number looks bad — a metric that improves when a cost is weakened
//! is usually a simulation that has quietly stopped selecting for anything.
//!
//! The whole charge is dissipation: it leaves the world through the ledger and is not
//! transferred to anything (spec §5.1).
//!
//! Deliberately not here: who pays it and what happens when they cannot. Charging and
//! death are the world's business.

use crate::params::MetabolismParams;

/// Energy one agent burns in a **tick**, at this body, brain, sensor load, and force.
///
/// `muscle` and `mouth` add `k · (trait² − 1)` each, zero at the reference body of 1,
/// so a Phase 2 body pays exactly what it did (spec §3.5). Below 1 they are discounts,
/// and a bill they would take below zero is zero: upkeep dissipates energy and cannot
/// create it (spec §5.1). `force` is the thrust a drive produced after muscle scaled
/// it, not the acceleration mass leaves of it.
///
/// Per tick, not per second, because that is how spec §5.5 states every constant in the
/// table ("0.05 /tick") and how it states the one relationship that matters: idling is
/// fatal within roughly 2000 ticks on a full tank, which is exactly `start_energy /
/// base`. Reading them per second instead multiplies every lifetime by 60 and leaves
/// none of §5.5's numbers meaning what they say.
///
/// It does make this the one system whose rate is per tick while `movement`'s drag is
/// per second, so changing `world.dt` rescales lifetimes without rescaling coasting.
/// That is the spec's calibration rather than a preference, and re-deriving the whole
/// table onto a per-second footing is not something to do by arithmetic with no running
/// population to check against — see the budget note in the M7 plan.
#[inline]
pub fn cost_per_tick(
    size: f32,
    muscle: f32,
    mouth: f32,
    brain_units: u32,
    sensor_load: f32,
    force: f32,
    params: &MetabolismParams,
) -> f32 {
    let bill = params.base
        + params.k_size * size * size
        + params.k_muscle * (muscle * muscle - 1.0)
        + params.k_mouth * (mouth * mouth - 1.0)
        + params.k_brain * brain_units as f32
        + params.k_sensor * sensor_load
        + params.k_move * force * force;
    // Compared rather than `f32::max`, which may return either zero for -0 and +0: a
    // byte-identical run needs the same zero on every target (spec §2.1).
    if bill > 0.0 { bill } else { 0.0 }
}

/// How long an idle agent survives on `energy`, in ticks, at this size and brain, with
/// the reference muscle and mouth.
///
/// The number spec §5.5 states its relationship against — idling must be fatal within
/// roughly 2000 ticks, or sitting still is a viable strategy and nothing evolves
/// (spec §10). Exposed rather than left to arithmetic in a comment because it is the
/// figure the metabolic budget is actually tuned against.
pub fn idle_ticks(
    energy: f32,
    size: f32,
    brain_units: u32,
    sensor_load: f32,
    params: &MetabolismParams,
) -> f32 {
    let per_tick = cost_per_tick(size, 1.0, 1.0, brain_units, sensor_load, 0.0, params);
    if per_tick <= 0.0 {
        return f32::INFINITY;
    }
    energy / per_tick
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;

    /// The default topology's brain and sensor load, from `founder`.
    const BRAIN_UNITS: u32 = 268;
    const SENSOR_LOAD: f32 = 16.0;

    #[test]
    fn every_term_costs_something() {
        // If any of these stopped contributing, the strategy it exists to penalise
        // would become free and the sim would quietly stop selecting against it.
        let p = SimParams::default().metabolism;
        let base = cost_per_tick(0.0, 1.0, 1.0, 0, 0.0, 0.0, &p);
        assert!(base > 0.0, "idling is free");
        assert!(
            cost_per_tick(3.0, 1.0, 1.0, 0, 0.0, 0.0, &p) > base,
            "size is free"
        );
        assert!(
            cost_per_tick(0.0, 1.0, 1.0, 268, 0.0, 0.0, &p) > base,
            "brain is free"
        );
        assert!(
            cost_per_tick(0.0, 1.0, 1.0, 0, 16.0, 0.0, &p) > base,
            "sensors are free"
        );
        assert!(
            cost_per_tick(0.0, 1.0, 1.0, 0, 0.0, 1.0, &p) > base,
            "moving is free"
        );
    }

    #[test]
    fn the_reference_body_pays_exactly_the_phase_2_cost() {
        // Muscle and mouth of 1 must add exactly nothing, or every Phase 2 world would
        // drift the moment these terms landed (spec §3.5).
        let p = SimParams::default().metabolism;
        for (size, units, load, force) in [(3.0, 13, 3.0, 0.0), (2.5, 268, 16.0, 0.7)] {
            let phase_2 = p.base
                + p.k_size * size * size
                + p.k_brain * units as f32
                + p.k_sensor * load
                + p.k_move * force * force;
            let now = cost_per_tick(size, 1.0, 1.0, units, load, force, &p);
            assert_eq!(now.to_bits(), phase_2.to_bits());
        }
    }

    #[test]
    fn muscle_and_mouth_add_their_squared_upkeep() {
        let p = SimParams::default().metabolism;
        let reference = cost_per_tick(3.0, 1.0, 1.0, 13, 3.0, 0.0, &p);
        let strong = cost_per_tick(3.0, 2.0, 1.0, 13, 3.0, 0.0, &p);
        let wide = cost_per_tick(3.0, 1.0, 1.5, 13, 3.0, 0.0, &p);
        let weak = cost_per_tick(3.0, 0.5, 1.0, 13, 3.0, 0.0, &p);
        assert!((strong - reference - p.k_muscle * 3.0).abs() < 1e-6);
        assert!((wide - reference - p.k_mouth * 1.25).abs() < 1e-6);
        assert!(
            (weak - reference + p.k_muscle * 0.75).abs() < 1e-6,
            "weaker muscle is cheaper to carry"
        );
    }

    #[test]
    fn discounts_never_take_the_bill_below_zero() {
        // Every other cost zero, so the weakest muscle's discount is the whole bill.
        // Paying it out would create energy (spec §5.1).
        let p = MetabolismParams {
            base: 0.0,
            k_size: 0.0,
            k_brain: 0.0,
            k_sensor: 0.0,
            k_move: 0.0,
            k_muscle: 1.0,
            k_mouth: 0.0,
        };
        let weak = cost_per_tick(3.0, 0.25, 1.0, 13, 3.0, 0.0, &p);
        assert_eq!(weak.to_bits(), 0.0f32.to_bits(), "{weak}");
        assert_eq!(cost_per_tick(3.0, 2.0, 1.0, 13, 3.0, 0.0, &p), 3.0);
        // A bill of -0 is charged as +0, the same zero on every target.
        let negative_zero = MetabolismParams {
            base: -0.0,
            k_size: -0.0,
            k_brain: -0.0,
            k_sensor: -0.0,
            k_move: -0.0,
            k_muscle: -0.0,
            k_mouth: -0.0,
        };
        let zero = cost_per_tick(3.0, 1.0, 1.0, 13, 3.0, 0.5, &negative_zero);
        assert_eq!(zero.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn doubling_size_roughly_quadruples_its_term() {
        // Spec §5.5 states the relationship, not the constant: upkeep is quadratic in
        // radius because surface area is.
        let p = SimParams::default().metabolism;
        let one = cost_per_tick(1.0, 1.0, 1.0, 0, 0.0, 0.0, &p) - p.base;
        let two = cost_per_tick(2.0, 1.0, 1.0, 0, 0.0, 0.0, &p) - p.base;
        assert!((two / one - 4.0).abs() < 1e-4, "{one} -> {two}");
    }

    #[test]
    fn sprinting_costs_more_than_twice_as_much_as_half_speed() {
        // Quadratic in force, so a sprint is never worth it for a marginal gain.
        let p = SimParams::default().metabolism;
        let half = cost_per_tick(0.0, 1.0, 1.0, 0, 0.0, 0.5, &p) - p.base;
        let full = cost_per_tick(0.0, 1.0, 1.0, 0, 0.0, 1.0, &p) - p.base;
        assert!((full / half - 4.0).abs() < 1e-4);
    }

    #[test]
    fn base_alone_is_the_two_thousand_ticks_spec_5_5_names() {
        // §5.5's headline relationship, and the check that the constants are read per
        // tick: `start_energy / base` is 100 / 0.05 = 2000 exactly. Read per second
        // instead, every lifetime in the table is out by a factor of 60.
        let params = SimParams::default();
        let bare = MetabolismParams {
            k_size: 0.0,
            k_brain: 0.0,
            k_sensor: 0.0,
            ..params.metabolism.clone()
        };
        let ticks = idle_ticks(params.reproduction.start_energy, 0.0, 0, 0.0, &bare);
        assert!(
            (ticks - 2_000.0).abs() < 1.0,
            "base alone gives {ticks:.0} ticks"
        );
    }

    #[test]
    fn the_default_budget_preserves_a_finite_foraging_window() {
        // M12 tuned the whole budget against live populations rather than making base
        // alone satisfy the ceiling. An idle founder gets time to encounter food and
        // mature, but cannot survive indefinitely without learning to forage.
        let params = SimParams::default();
        let ticks = idle_ticks(
            params.reproduction.start_energy,
            params.body.size,
            BRAIN_UNITS,
            SENSOR_LOAD,
            &params.metabolism,
        );
        assert!(
            (1_100.0..1_300.0).contains(&ticks),
            "an idle founder now lasts {ticks:.0} ticks, outside the M12-tuned window"
        );
    }

    #[test]
    fn a_zero_cost_world_never_starves() {
        // Guards the division. A params set with every term at zero is legal and would
        // otherwise divide by zero rather than reporting the truth: nothing dies.
        let mut p = SimParams::default().metabolism;
        p.base = 0.0;
        p.k_size = 0.0;
        p.k_brain = 0.0;
        p.k_sensor = 0.0;
        assert!(idle_ticks(100.0, 3.0, 268, 16.0, &p).is_infinite());
    }
}
