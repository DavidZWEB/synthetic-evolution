//! The world's random number generator: seeded, serializable, and the only source of
//! randomness in the simulation.
//!
//! Every draw the sim makes comes from a `Rng` owned by the `World`. There is no
//! ambient randomness anywhere — `thread_rng` is unseedable and would make a run
//! unreproducible, which costs the golden hash, replay, and the ability to debug an
//! emergent behaviour you saw once (spec §2.1). The invariants test bans it.
//!
//! Deliberately not here: any per-system RNG. Splitting streams per system is a
//! reasonable thing to want and would change every existing golden hash, so it is a
//! decision to take deliberately, not by accident.

use rand::{Rng as _, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde::{Deserialize, Serialize};

use crate::math;

/// Seeded PCG generator with serializable state, so a world can be checkpointed and
/// resumed mid-run without changing what it produces next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    inner: Pcg64Mcg,
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        Self {
            inner: Pcg64Mcg::seed_from_u64(seed),
        }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.inner.random()
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.inner.random()
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn unit(&mut self) -> f32 {
        // 24 bits is the f32 mantissa; taking the high bits of the draw keeps the
        // spacing uniform, which the low bits of a truncating cast would not.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[low, high)`.
    #[inline]
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        debug_assert!(low.is_finite() && high.is_finite() && low <= high);
        let unit = self.unit();
        let width = high - low;
        if width.is_finite() {
            low + unit * width
        } else {
            // Finite endpoints can straddle a wider-than-f32 interval. Preserve the
            // ordinary path's arithmetic/draws, but avoid infinity times zero here.
            (1.0 - unit) * low + unit * high
        }
    }

    /// Uniform in `[0, n)`. Returns 0 for `n == 0`.
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// Gaussian with the given mean and standard deviation, by Box–Muller.
    ///
    /// Box–Muller produces two independent samples; this discards the second rather
    /// than caching it. The cache would be generator state that has to serialize
    /// identically for a resumed run to continue the same stream, and mutation is not
    /// the hot loop — a wasted draw is cheaper than that correctness surface.
    pub fn normal(&mut self, mean: f32, std_dev: f32) -> f32 {
        debug_assert!(std_dev >= 0.0);
        // Excluding 0 keeps ln() finite; unit() already excludes 1.
        let u1 = 1.0 - self.unit();
        let u2 = self.unit();
        let radius = math::sqrt(-2.0 * math::ln(u1));
        mean + std_dev * radius * math::cos(core::f32::consts::TAU * u2)
    }

    /// Folds generator state into a hash by drawing from a clone.
    ///
    /// `Pcg64Mcg` does not expose its state, and the golden hash has to cover the RNG
    /// — two runs that agree on every position but sit at different points in the
    /// stream will diverge on the next tick, and a hash that misses that is worse than
    /// no hash (spec §7.8). Drawing from a clone is a deterministic function of the
    /// state and leaves the real stream untouched.
    pub fn state_fingerprint(&self) -> u64 {
        let mut probe = self.clone();
        probe.next_u64() ^ probe.next_u64().rotate_left(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_finite_intervals_stay_finite_and_consume_one_draw() {
        let mut rng = Rng::from_seed(7);
        let mut reference = rng.clone();
        for _ in 0..1_000 {
            let value = rng.range(-f32::MAX, f32::MAX);
            assert!(value.is_finite());
            assert!((-f32::MAX..=f32::MAX).contains(&value));
            reference.unit();
            assert_eq!(rng.state_fingerprint(), reference.state_fingerprint());
        }
    }

    #[test]
    fn ordinary_range_arithmetic_is_bit_identical() {
        let mut rng = Rng::from_seed(42);
        let mut reference = rng.clone();
        for (low, high) in [(-4.0, 4.0), (0.05, 2.0), (0.0, 0.0)] {
            for _ in 0..100 {
                let expected = low + reference.unit() * (high - low);
                assert_eq!(rng.range(low, high).to_bits(), expected.to_bits());
            }
        }
    }

    #[test]
    fn same_seed_gives_the_same_stream() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        for _ in 0..1_000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(43);
        let differs = (0..64).any(|_| a.next_u64() != b.next_u64());
        assert!(differs);
    }

    #[test]
    fn unit_stays_in_range() {
        let mut rng = Rng::from_seed(7);
        for _ in 0..100_000 {
            let x = rng.unit();
            assert!((0.0..1.0).contains(&x), "{x}");
        }
    }

    #[test]
    fn normal_has_the_requested_moments() {
        let mut rng = Rng::from_seed(9);
        const N: usize = 200_000;
        let samples: Vec<f32> = (0..N).map(|_| rng.normal(2.0, 0.5)).collect();
        let mean = samples.iter().sum::<f32>() / N as f32;
        let var = samples.iter().map(|s| (s - mean) * (s - mean)).sum::<f32>() / N as f32;
        assert!((mean - 2.0).abs() < 0.01, "mean {mean}");
        assert!(
            (math::sqrt(var) - 0.5).abs() < 0.01,
            "sd {}",
            math::sqrt(var)
        );
    }

    #[test]
    fn fingerprint_tracks_position_in_the_stream() {
        let mut rng = Rng::from_seed(11);
        let before = rng.state_fingerprint();
        // Reading the fingerprint must not itself advance the stream.
        assert_eq!(before, rng.state_fingerprint());
        rng.next_u32();
        assert_ne!(before, rng.state_fingerprint());
    }

    #[test]
    fn state_round_trips_and_resumes_the_same_stream() {
        let mut rng = Rng::from_seed(3);
        for _ in 0..100 {
            rng.next_u64();
        }
        let bytes = postcard::to_allocvec(&rng).expect("rng serializes");
        let mut resumed: Rng = postcard::from_bytes(&bytes).expect("rng deserializes");
        for _ in 0..100 {
            assert_eq!(rng.next_u64(), resumed.next_u64());
        }
    }
}
