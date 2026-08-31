//! What it costs to be alive for one tick.
//!
//! Spec §5.2's cost function, and the reason it is load-bearing rather than flavour:
//! every term here is what makes some strategy lose. Without `k_brain` and `k_sensor`,
//! "grow every organ" has no downside and genomes bloat until the sim crawls; without
//! `base`, sitting still is free and nothing ever has to forage; without `k_move`,
//! sprinting everywhere is as cheap as drifting. CLAUDE.md lists these under *do not
//! simplify* for exactly that reason — a metric that improves when a cost is weakened
//! is usually a simulation that has quietly stopped selecting for anything.
//!
//! The whole charge is dissipation: it leaves the world through the ledger and is not
//! transferred to anything (spec §5.1).
//!
//! Deliberately not here: who pays it and what happens when they cannot. Charging and
//! death are the world's business.

use crate::params::MetabolismParams;

/// Energy one agent burns in a **tick**, at this size, brain, sensor load, and thrust.
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
    brain_units: u32,
    sensor_load: f32,
    thrust: f32,
    params: &MetabolismParams,
) -> f32 {
    params.base
        + params.k_size * size * size
        + params.k_brain * brain_units as f32
        + params.k_sensor * sensor_load
        + params.k_move * thrust * thrust
}

/// How long an idle agent survives on `energy`, in ticks, at this body and brain.
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
    let per_tick = cost_per_tick(size, brain_units, sensor_load, 0.0, params);
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
        let base = cost_per_tick(0.0, 0, 0.0, 0.0, &p);
        assert!(base > 0.0, "idling is free");
        assert!(cost_per_tick(3.0, 0, 0.0, 0.0, &p) > base, "size is free");
        assert!(
            cost_per_tick(0.0, 268, 0.0, 0.0, &p) > base,
            "brain is free"
        );
        assert!(
            cost_per_tick(0.0, 0, 16.0, 0.0, &p) > base,
            "sensors are free"
        );
        assert!(cost_per_tick(0.0, 0, 0.0, 1.0, &p) > base, "moving is free");
    }

    #[test]
    fn doubling_size_roughly_quadruples_its_term() {
        // Spec §5.5 states the relationship, not the constant: upkeep is quadratic in
        // radius because surface area is.
        let p = SimParams::default().metabolism;
        let one = cost_per_tick(1.0, 0, 0.0, 0.0, &p) - p.base;
        let two = cost_per_tick(2.0, 0, 0.0, 0.0, &p) - p.base;
        assert!((two / one - 4.0).abs() < 1e-4, "{one} -> {two}");
    }

    #[test]
    fn sprinting_costs_more_than_twice_as_much_as_half_speed() {
        // Quadratic in force, so a sprint is never worth it for a marginal gain.
        let p = SimParams::default().metabolism;
        let half = cost_per_tick(0.0, 0, 0.0, 0.5, &p) - p.base;
        let full = cost_per_tick(0.0, 0, 0.0, 1.0, &p) - p.base;
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
    fn the_default_budget_is_still_the_known_overshoot() {
        // A tripwire, not an endorsement. At the default body and topology an idle
        // agent lasts about 250 ticks against the ~2000 §5.5 asks for, because
        // `k_size` is quadratic in a `body.size` that defaults to 3 and `k_sensor` is
        // charged per channel rather than per sensor — both defaults chosen in this
        // repo rather than taken from the spec.
        //
        // It is pinned rather than fixed because the plan is explicit that these move
        // against a running population and several seeds, not by arithmetic, and
        // reproduction does not exist yet. When the tuning pass runs, this test should
        // fail and be updated deliberately, with the new reasoning on the params.
        let params = SimParams::default();
        let ticks = idle_ticks(
            params.reproduction.start_energy,
            params.body.size,
            BRAIN_UNITS,
            SENSOR_LOAD,
            &params.metabolism,
        );
        assert!(
            (200.0..320.0).contains(&ticks),
            "an idle agent now lasts {ticks:.0} ticks, not the ~250 recorded in the \
             M7 budget note. If this is the tuning pass, update the note and the \
             reasoning on MetabolismParams in the same commit."
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
