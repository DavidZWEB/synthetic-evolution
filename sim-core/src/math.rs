//! Deterministic scalar math, and the yaw-constrained quaternion helpers.
//!
//! Every transcendental the simulation evaluates goes through here, and every one is
//! a `libm` software implementation. Platform `sin`/`cos`/`exp` are libm builds that
//! differ between native and WASM, so using them makes a native run and a browser run
//! diverge from the same seed with nothing in the logs to point at (spec §7.4).
//! `sim-core/tests/invariants.rs` bans the method forms outright.
//!
//! Deliberately not here: vector and quaternion arithmetic, which is `glam`'s job.
//! This module only adds the transcendentals `glam` does not cover and the yaw
//! constraint V1 imposes on orientation.

use glam::{Quat, Vec3};

#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

#[inline]
pub fn tanh(x: f32) -> f32 {
    libm::tanhf(x)
}

#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[inline]
pub fn pow(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// IEEE-754 specifies `sqrt` exactly, so this one is portable either way. Routed
/// through here for uniformity rather than necessity.
#[inline]
pub fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}

/// Logistic activation, the `σ` of the CTRNN equation in spec §3.2.
#[inline]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

/// Wraps an angle into `(-π, π]`. Sensor azimuths and heading differences are
/// compared as angles, and an unwrapped one silently reads as "behind me".
#[inline]
pub fn wrap_pi(angle: f32) -> f32 {
    let two_pi = core::f32::consts::TAU;
    let wrapped = libm::fmodf(angle + core::f32::consts::PI, two_pi);
    let wrapped = if wrapped < 0.0 {
        wrapped + two_pi
    } else {
        wrapped
    };
    wrapped - core::f32::consts::PI
}

/// An organ's azimuth in range, so adding it to a heading turns with the body: an
/// imported gene may carry any finite azimuth, and at 2^26 an `f32` step is 8, so a
/// quarter turn of yaw added to it rounds away. One already in `[-π, π]` is returned
/// exactly, so every founder and drawn azimuth reads as it always did.
///
/// Out of range, through the azimuth's own sine and cosine, which libm reduces exactly,
/// so the angle keeps the direction it encodes. Wrapping by an `f32` τ would not: at
/// 2^26 adding π rounds away and τ's rounding loses phase, 1.27 rad in all.
#[inline]
pub fn reduce_angle(angle: f32) -> f32 {
    use core::f32::consts::PI;
    if (-PI..=PI).contains(&angle) {
        angle
    } else {
        atan2(sin(angle), cos(angle))
    }
}

/// Orientation as a quaternion, constrained to yaw about Z.
///
/// V1 is a 2D plane and a scalar heading would do. The quaternion is one of the two
/// hedges in spec §2.2 that make the eventual move to volumetric 3D an unlock rather
/// than a rewrite, and it is load-bearing in the serialized genome and world format:
/// every saved world becomes unloadable the day a scalar heading has to grow two more
/// degrees of freedom. Do not simplify to a float (spec §9.1).
#[inline]
pub fn yaw_quat(yaw: f32) -> Quat {
    let half = yaw * 0.5;
    Quat::from_xyzw(0.0, 0.0, sin(half), cos(half))
}

/// Extracts the yaw of a Z-constrained orientation, in `(-π, π]`.
#[inline]
pub fn yaw_of(q: Quat) -> f32 {
    debug_assert!(
        q.x.abs() < 1e-4 && q.y.abs() < 1e-4,
        "orientation left the Z-yaw constraint: {q:?}"
    );
    wrap_pi(2.0 * atan2(q.z, q.w))
}

/// The agent's facing direction: local +X rotated by `q`.
///
/// Goes through `glam`'s quaternion-vector product rather than `(cos yaw, sin yaw)`
/// so the hot path costs no transcendentals at all.
#[inline]
pub fn forward(q: Quat) -> Vec3 {
    q * Vec3::X
}

/// Rotates a Z-constrained orientation by `delta` radians, renormalizing.
///
/// The turn effector takes a rotation *axis*, pinned to Z here (spec §9.1) — this is
/// the yaw-only specialization of that.
#[inline]
pub fn rotate_yaw(q: Quat, delta: f32) -> Quat {
    (yaw_quat(delta) * q).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn reduced_angles_keep_range_values_exactly_and_bring_huge_ones_in() {
        use core::f32::consts::{FRAC_PI_2, PI};
        for angle in [0.0, 0.3, -PI, PI, -0.0] {
            assert_eq!(reduce_angle(angle).to_bits(), angle.to_bits(), "{angle}");
        }
        // At 2^26 a quarter turn added to the raw azimuth rounds away. Reduced, it keeps
        // the direction 2^26 rad encodes, about 2.7073 rad, which wrapping by an f32 τ
        // misplaces by 1.27 rad.
        let huge = 67_108_864.0f32;
        assert_eq!(FRAC_PI_2 + huge, huge);
        let reduced = reduce_angle(huge);
        assert!((reduced - 2.707_31).abs() < 1e-5, "{reduced}");
        assert!((reduce_angle(-huge) + 2.707_31).abs() < 1e-5);
    }

    #[test]
    fn transcendentals_match_known_values() {
        assert!((sin(0.0)).abs() < EPS);
        assert!((sin(core::f32::consts::FRAC_PI_2) - 1.0).abs() < EPS);
        assert!((cos(0.0) - 1.0).abs() < EPS);
        assert!((exp(0.0) - 1.0).abs() < EPS);
        assert!((exp(1.0) - core::f32::consts::E).abs() < EPS);
        assert!((ln(core::f32::consts::E) - 1.0).abs() < EPS);
        assert!((sqrt(9.0) - 3.0).abs() < EPS);
        assert!((pow(2.0, 10.0) - 1024.0).abs() < 1e-3);
        assert!((atan2(1.0, 1.0) - core::f32::consts::FRAC_PI_4).abs() < EPS);
    }

    #[test]
    fn sigmoid_is_centred_and_saturating() {
        assert!((sigmoid(0.0) - 0.5).abs() < EPS);
        assert!(sigmoid(-20.0) < 1e-6);
        assert!(sigmoid(20.0) > 1.0 - 1e-6);
        // Symmetry — a lopsided activation biases every brain in the world.
        assert!((sigmoid(3.0) + sigmoid(-3.0) - 1.0).abs() < EPS);
    }

    #[test]
    fn wrap_pi_lands_in_range() {
        for step in -20..=20 {
            let angle = step as f32 * 0.7;
            let w = wrap_pi(angle);
            assert!(
                w > -core::f32::consts::PI - EPS && w <= core::f32::consts::PI + EPS,
                "{angle} -> {w}"
            );
            // Wrapping may only shift by whole turns.
            let turns = (angle - w) / core::f32::consts::TAU;
            assert!((turns - turns.round()).abs() < 1e-3, "{angle} -> {w}");
        }
    }

    #[test]
    fn yaw_round_trips_through_the_quaternion() {
        for step in -10..=10 {
            let yaw = step as f32 * 0.3;
            assert!(
                (yaw_of(yaw_quat(yaw)) - wrap_pi(yaw)).abs() < 1e-4,
                "yaw {yaw}"
            );
        }
    }

    #[test]
    fn forward_points_along_yaw() {
        let f = forward(yaw_quat(core::f32::consts::FRAC_PI_2));
        assert!(
            f.x.abs() < 1e-4 && (f.y - 1.0).abs() < 1e-4 && f.z.abs() < 1e-6,
            "{f:?}"
        );
    }

    #[test]
    fn rotation_stays_on_the_z_axis() {
        // The forward-compatibility hedge is only useful if V1 actually holds the
        // constraint — an orientation that drifts off Z has no scalar heading to
        // recover (spec §9.1).
        let mut q = yaw_quat(0.0);
        for _ in 0..10_000 {
            q = rotate_yaw(q, 0.017);
        }
        assert!(
            q.x.abs() < 1e-5 && q.y.abs() < 1e-5,
            "drifted off the Z axis: {q:?}"
        );
        assert!(
            (q.length() - 1.0).abs() < 1e-5,
            "not normalized: {}",
            q.length()
        );
    }
}
