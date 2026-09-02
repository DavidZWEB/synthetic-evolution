//! Budding: when an agent may have offspring, and where the offspring goes.
//!
//! Asexual for all of V1 — the sexual pathway unlocks in Phase 6, and `crossover` is
//! already written and tested against that day (spec §3.4). What is here is the two
//! decisions the world needs on every birth, kept out of `world` so both are testable
//! without one.
//!
//! **Reproduction is gated by the brain, not by an energy threshold.** The threshold is
//! a floor, not a trigger: an agent that can afford to breed still has to decide to,
//! which is what turns life-history strategy into something evolvable rather than a
//! constant. Spec §4.2 calls this out as one of the two highest-value cheap features in
//! the design, and it is why the reproduce effector exists at all.
//!
//! **Offspring spawn next to their parent.** This looks like an arbitrary detail and is
//! load-bearing: it is what makes an agent's neighbours its relatives, which is the only
//! reason kin selection can operate and the only reason cooperative behaviour or
//! communication can ever pay (spec §5.4). Scattering births uniformly would leave a
//! world that runs identically and can never evolve either.
//!
//! Deliberately not here: the split of the parent's energy, which is arithmetic the
//! world does while it holds both agents, and mutation, which is `mutate`'s.

use glam::Vec3;

use crate::math;
use crate::params::ReproductionParams;
use crate::rng::Rng;
use crate::spatial::wrap_scalar;

/// Whether an agent is both asking to reproduce and able to.
///
/// Three conditions, and each rules out a different degenerate strategy: the gate means
/// the brain has to ask, the threshold means growth has to happen first — it sits above
/// `start_energy`, so an agent cannot breed on the tank it was born with — and maturity
/// stops a lineage collapsing into a chain of instant births that never has to survive
/// anything.
#[inline]
pub fn ready(energy: f32, age: u32, drive: f32, params: &ReproductionParams) -> bool {
    drive > params.gate && energy >= params.threshold && age >= params.maturity_ticks
}

/// Where an offspring appears: uniformly inside `spawn_radius` of its parent, wrapped
/// onto the torus.
///
/// Uniform over the *disc*, not over radius — `sqrt` on the draw, without which births
/// would cluster on top of the parent and the spread would be much tighter than
/// `spawn_radius` suggests.
///
/// Two draws, always, whatever the radius. A birth that drew a different number of
/// values depending on its parameters would make the random stream depend on world
/// state, and two runs of the same seed would diverge (spec §7.4).
pub fn offspring_position(
    parent: Vec3,
    params: &ReproductionParams,
    world_size: f32,
    rng: &mut Rng,
) -> Vec3 {
    let radius = params.spawn_radius * math::sqrt(rng.unit());
    let angle = rng.range(-core::f32::consts::PI, core::f32::consts::PI);
    Vec3::new(
        wrap_scalar(parent.x + radius * math::cos(angle), world_size),
        wrap_scalar(parent.y + radius * math::sin(angle), world_size),
        0.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use crate::spatial::min_image;

    fn params() -> ReproductionParams {
        SimParams::default().reproduction
    }

    #[test]
    fn the_brain_has_to_ask() {
        // An energy threshold alone would make reproduction automatic, and life-history
        // strategy would stop being something a lineage can evolve (spec §4.2).
        let p = params();
        let rich = p.threshold + 50.0;
        assert!(ready(rich, p.maturity_ticks, p.gate + 0.1, &p));
        assert!(
            !ready(rich, p.maturity_ticks, p.gate - 0.1, &p),
            "bred without asking"
        );
    }

    #[test]
    fn growth_has_to_happen_first() {
        // The threshold sits above `start_energy`, so an agent cannot breed on the tank
        // it was born with — otherwise a lineage could divide forever without ever
        // having to find food.
        let p = params();
        assert!(
            p.threshold > p.start_energy,
            "threshold must require growth"
        );
        assert!(!ready(p.start_energy, p.maturity_ticks, 1.0, &p));
        assert!(ready(p.threshold, p.maturity_ticks, 1.0, &p));
    }

    #[test]
    fn a_newborn_has_to_survive_something_first() {
        let p = params();
        let rich = p.threshold + 50.0;
        assert!(!ready(rich, 0, 1.0, &p), "bred on the tick it was born");
        assert!(!ready(rich, p.maturity_ticks - 1, 1.0, &p));
        assert!(ready(rich, p.maturity_ticks, 1.0, &p));
    }

    #[test]
    fn offspring_land_within_the_spawn_radius() {
        // Spatial viscosity. Without it neighbours are strangers, kin selection cannot
        // operate, and communication has nothing to evolve for (spec §5.4).
        let p = params();
        let size = SimParams::default().world.size;
        let parent = Vec3::new(500.0, 500.0, 0.0);
        let mut rng = Rng::from_seed(3);
        for _ in 0..2_000 {
            let child = offspring_position(parent, &p, size, &mut rng);
            let d = min_image(child - parent, size).length();
            assert!(d <= p.spawn_radius + 1e-3, "born {d} away");
            assert_eq!(child.z, 0.0, "V1 simulates on a plane");
            assert!((0.0..size).contains(&child.x) && (0.0..size).contains(&child.y));
        }
    }

    #[test]
    fn births_spread_over_the_disc_rather_than_piling_on_the_parent() {
        // Drawing the radius uniformly instead of its square root would put half of all
        // births inside a quarter of the area, and the effective spread would be far
        // tighter than `spawn_radius` claims.
        let p = params();
        let size = SimParams::default().world.size;
        let parent = Vec3::new(500.0, 500.0, 0.0);
        let mut rng = Rng::from_seed(5);
        let half = p.spawn_radius / core::f32::consts::SQRT_2;
        let inner = (0..4_000)
            .filter(|_| {
                let child = offspring_position(parent, &p, size, &mut rng);
                min_image(child - parent, size).length() < half
            })
            .count();
        // Half the disc's area lies inside radius/√2, so about half the births should.
        assert!(
            (1_800..2_200).contains(&inner),
            "{inner} of 4000 births fell in the inner half of the area"
        );
    }

    #[test]
    fn birth_placement_crosses_the_wrap_seam() {
        let p = params();
        let size = SimParams::default().world.size;
        let parent = Vec3::new(size - 1.0, 500.0, 0.0);
        let mut rng = Rng::from_seed(7);
        let mut wrapped = 0;
        for _ in 0..500 {
            let child = offspring_position(parent, &p, size, &mut rng);
            assert!(
                (0.0..size).contains(&child.x),
                "born outside the world: {child:?}"
            );
            if child.x < p.spawn_radius {
                wrapped += 1;
            }
        }
        assert!(wrapped > 0, "no birth ever crossed the seam");
    }

    #[test]
    fn placement_is_deterministic_and_costs_a_fixed_number_of_draws() {
        // A birth that drew a different count depending on its parameters would make
        // the stream depend on world state, and the same seed would stop replaying.
        let p = params();
        let size = SimParams::default().world.size;
        let run = |radius: f32| {
            let mut rng = Rng::from_seed(11);
            let mut wide = p.clone();
            wide.spawn_radius = radius;
            let first = offspring_position(Vec3::new(500.0, 500.0, 0.0), &wide, size, &mut rng);
            // Whatever the radius, the same number of draws has been consumed.
            (first, rng.next_u64())
        };
        assert_eq!(run(8.0).1, run(80.0).1, "draw count varied with the radius");
        assert_eq!(run(8.0).0, run(8.0).0);
    }
}
