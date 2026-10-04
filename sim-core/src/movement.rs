//! Movement: turning thrust and turn intents into velocity, orientation, and position.
//!
//! Step 5 of the tick, and the first system that changes the world rather than asking
//! to. It reads the intent buffer and nothing else about what an agent wanted, so a
//! brain cannot move anything except through step 4 (spec §2.4).
//!
//! The world is a **torus**, so position wraps in x and y. That has to agree with
//! `spatial`, which measures every distance as a minimum image — an agent that walked
//! off the edge and kept going would be found by nothing and would see nothing.
//!
//! Z is pinned to the plane throughout. Positions are still three components and
//! orientation is still a quaternion, both hedges that make volumetric 3D an unlock
//! rather than a rewrite (spec §2.3, §9.1).
//!
//! Deliberately not here: what movement costs. `k_move · |force|²` is charged at step 9
//! with the rest of metabolism (M7, spec §5.2).

use glam::{Quat, Vec3};

use crate::math;
use crate::params::{MovementParams, WorldParams};
use crate::spatial::wrap_scalar;

/// Advances one agent by a tick of its own intent.
///
/// Turn first, then thrust: an agent pushes along the heading it just chose rather than
/// the one it held last tick. Either order is deterministic, but this one makes a turn
/// take effect immediately, which is what "steer toward the food" has to mean if a
/// single tick of sensing is to be worth anything.
pub fn integrate(
    position: &mut Vec3,
    velocity: &mut Vec3,
    orientation: &mut Quat,
    thrust: f32,
    turn: f32,
    movement: &MovementParams,
    world: &WorldParams,
) {
    let dt = world.dt;

    *orientation = math::rotate_yaw(*orientation, turn * dt);

    // `thrust` arrives as an acceleration: the caller has already divided the force
    // by the body's mass (spec §3.5).
    *velocity += math::forward(*orientation) * (thrust * dt);

    // `drag` is the fraction of velocity kept per *second*, so a tick keeps
    // `drag^dt` — otherwise the same parameter would mean different things at
    // different timesteps, and changing `dt` would silently change behaviour.
    *velocity *= math::pow(movement.drag, dt);

    // A speed ceiling so a runaway output cannot carry an agent further than a hash
    // cell in one tick, which would let it tunnel past every neighbour it should have
    // met (spec §2.3).
    let speed = velocity.length();
    if speed > movement.max_speed {
        *velocity *= movement.max_speed / speed;
    }

    *position += *velocity * dt;

    // Wrap, then pin Z. The plane is where V1 simulates; the third component exists so
    // it does not have to be added back later (spec §9.1).
    *position = Vec3::new(
        wrap_scalar(position.x, world.size),
        wrap_scalar(position.y, world.size),
        0.0,
    );
    velocity.z = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;

    struct Body {
        position: Vec3,
        velocity: Vec3,
        orientation: Quat,
    }

    impl Body {
        fn at(position: Vec3, yaw: f32) -> Self {
            Self {
                position,
                velocity: Vec3::ZERO,
                orientation: math::yaw_quat(yaw),
            }
        }

        fn step(&mut self, thrust: f32, turn: f32, params: &SimParams) {
            integrate(
                &mut self.position,
                &mut self.velocity,
                &mut self.orientation,
                thrust,
                turn,
                &params.movement,
                &params.world,
            );
        }

        fn yaw(&self) -> f32 {
            math::yaw_of(self.orientation)
        }
    }

    #[test]
    fn thrust_moves_an_agent_along_its_heading() {
        let params = SimParams::default();
        let mut east = Body::at(Vec3::new(500.0, 500.0, 0.0), 0.0);
        let mut north = Body::at(Vec3::new(500.0, 500.0, 0.0), core::f32::consts::FRAC_PI_2);
        for _ in 0..60 {
            east.step(params.movement.max_thrust, 0.0, &params);
            north.step(params.movement.max_thrust, 0.0, &params);
        }
        assert!(east.position.x > 500.0, "did not travel east");
        assert!(north.position.y > 500.0, "did not travel north");
        // The error is what gets the `abs`, not the coordinate. Written the other way
        // round this reads as "y is below 500.001" and passes for an agent a hundred
        // units off course, which is the whole class of bug it exists to catch.
        assert!(
            (east.position.y - 500.0).abs() < 1e-3,
            "drifted sideways to y = {}",
            east.position.y
        );
        assert!(
            (north.position.x - 500.0).abs() < 1e-3,
            "drifted sideways to x = {}",
            north.position.x
        );
        assert!(
            (east.position.x - 500.0 - (north.position.y - 500.0)).abs() < 1e-3,
            "heading changed the distance travelled, not just the direction"
        );
    }

    #[test]
    fn no_thrust_coasts_to_a_stop() {
        let params = SimParams::default();
        let mut body = Body::at(Vec3::new(500.0, 500.0, 0.0), 0.0);
        for _ in 0..60 {
            body.step(params.movement.max_thrust, 0.0, &params);
        }
        let moving = body.velocity.length();
        assert!(moving > 0.5, "never got going: {moving}");
        // `drag` is 0.9 per second, so shedding 99% takes ~44 seconds, not the one or
        // two it looks like. Coasting is meant to be long — a world where releasing
        // thrust stops you dead rewards nothing but constant burn.
        for _ in 0..3_000 {
            body.step(0.0, 0.0, &params);
        }
        assert!(
            body.velocity.length() < moving * 0.01,
            "still coasting at {}",
            body.velocity.length()
        );
    }

    #[test]
    fn drag_is_per_second_not_per_tick() {
        // The parameter is documented as velocity retained per second. If it were
        // applied per tick instead, the same number would mean something different at
        // every timestep and changing `dt` would silently change behaviour.
        let params = SimParams::default();
        let mut body = Body::at(Vec3::new(500.0, 500.0, 0.0), 0.0);
        body.velocity = Vec3::new(10.0, 0.0, 0.0);
        let ticks = (1.0 / params.world.dt).round() as usize;
        for _ in 0..ticks {
            body.step(0.0, 0.0, &params);
        }
        assert!(
            (body.velocity.x - 10.0 * params.movement.drag).abs() < 0.05,
            "after one second velocity is {} not {}",
            body.velocity.x,
            10.0 * params.movement.drag
        );
    }

    #[test]
    fn turn_is_signed_and_stays_on_the_z_axis() {
        let params = SimParams::default();
        let mut left = Body::at(Vec3::ZERO, 0.0);
        let mut right = Body::at(Vec3::ZERO, 0.0);
        for _ in 0..30 {
            left.step(0.0, params.movement.max_turn_rate, &params);
            right.step(0.0, -params.movement.max_turn_rate, &params);
        }
        assert!(left.yaw() > 0.0, "positive turn should go left (CCW)");
        assert!(right.yaw() < 0.0, "negative turn should go right");
        assert!(
            (left.yaw() + right.yaw()).abs() < 1e-4,
            "turn is asymmetric"
        );
        for body in [&left, &right] {
            assert!(
                body.orientation.x.abs() < 1e-5 && body.orientation.y.abs() < 1e-5,
                "orientation left the Z-yaw constraint: {:?}",
                body.orientation
            );
        }
    }

    #[test]
    fn turning_takes_effect_before_this_tick_s_thrust() {
        // A turn that only applied next tick would make a single tick of sensing worth
        // nothing, and steering would always lag the thing it was steering at.
        let params = SimParams::default();
        let mut body = Body::at(Vec3::new(500.0, 500.0, 0.0), 0.0);
        // A quarter turn in one tick, then push.
        let rate = core::f32::consts::FRAC_PI_2 / params.world.dt;
        body.step(params.movement.max_thrust, rate, &params);
        assert!(
            body.velocity.y > body.velocity.x.abs(),
            "thrust went the old way: {:?}",
            body.velocity
        );
    }

    #[test]
    fn speed_is_capped_below_a_hash_cell_per_tick() {
        // Past a cell per tick an agent tunnels through neighbours it should have met,
        // and the spatial hash silently stops being a neighbour query (spec §2.3).
        let params = SimParams::default();
        let mut body = Body::at(Vec3::new(500.0, 500.0, 0.0), 0.0);
        for _ in 0..10_000 {
            body.step(params.movement.max_thrust * 1_000.0, 0.0, &params);
        }
        let speed = body.velocity.length();
        assert!(
            speed <= params.movement.max_speed + 1e-3,
            "ran away to {speed}"
        );
        assert!(
            speed * params.world.dt < params.sensing.max_sense_radius(),
            "a tick of travel ({}) outruns the hash cell ({})",
            speed * params.world.dt,
            params.sensing.max_sense_radius()
        );
    }

    #[test]
    fn position_wraps_onto_the_torus() {
        // Movement and `spatial` have to agree about the seam. An agent that walked off
        // the edge and kept going would be found by nothing and would see nothing.
        let params = SimParams::default();
        let size = params.world.size;
        let mut body = Body::at(Vec3::new(size - 0.2, 500.0, 0.0), 0.0);
        body.velocity = Vec3::new(params.movement.max_speed, 0.0, 0.0);
        body.step(0.0, 0.0, &params);
        assert!(body.position.x < 10.0, "did not wrap: {}", body.position.x);
        assert!((0.0..size).contains(&body.position.x));

        let mut back = Body::at(Vec3::new(0.2, 500.0, 0.0), 0.0);
        back.velocity = Vec3::new(-params.movement.max_speed, 0.0, 0.0);
        back.step(0.0, 0.0, &params);
        assert!(back.position.x > size - 10.0, "did not wrap backwards");
    }

    #[test]
    fn movement_stays_on_the_plane() {
        let params = SimParams::default();
        let mut body = Body::at(Vec3::new(500.0, 500.0, 0.0), 1.0);
        body.velocity = Vec3::new(0.0, 0.0, 5.0);
        for _ in 0..100 {
            body.step(params.movement.max_thrust, 0.5, &params);
        }
        assert_eq!(body.position.z, 0.0, "V1 simulates on a plane (spec §2.3)");
        assert_eq!(body.velocity.z, 0.0);
    }

    #[test]
    fn integration_is_deterministic() {
        let params = SimParams::default();
        let run = || {
            let mut body = Body::at(Vec3::new(123.0, 456.0, 0.0), 0.7);
            for i in 0..500 {
                let t = i as f32 * 0.01;
                body.step(math::sin(t).abs(), math::cos(t), &params);
            }
            (body.position, body.velocity, body.orientation)
        };
        assert_eq!(run(), run());
    }
}
