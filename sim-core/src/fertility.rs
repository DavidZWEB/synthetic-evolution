//! Fertility: where on the torus a plant can establish (spec §5.3).
//!
//! A static map of smoothed value noise, periodic across the world, drawn once when
//! the world is built and consulted whenever a plant site is chosen. It decides where
//! plants live, not how fast they grow: a dense patch already takes more of the input
//! because every plant is offered an equal share (spec §5.1).
//!
//! Deliberately not here: the plants themselves, or any change over time. Seasons and
//! disturbance are Phase 6 work, and a map that moved would need saving rather than
//! regenerating from the seed.

use glam::Vec3;

use crate::math;
use crate::params::SimParams;
use crate::rng::Rng;
use crate::spatial::wrap_scalar;

/// Candidate sites tried before the last one is kept regardless.
///
/// A bound on work rather than a tunable: with `patchiness` validated to at most
/// [`crate::params::MAX_PATCHINESS`] the acceptance rate stays far above `1 / 1024`,
/// so the cap exists only to guarantee that choosing a site cannot stall a tick.
const MAX_TRIES: u32 = 1_024;

/// The fertility map, or the uniform world when `patchiness` is zero.
#[derive(Clone, Debug)]
pub(crate) struct Fertility {
    /// Lattice points per axis; zero for a uniform world, which draws nothing.
    cells: u32,
    cell_size: f32,
    world_size: f32,
    /// Row-major lattice values in `[0, 1)`.
    values: Vec<f32>,
    /// The largest lattice value. Smoothed interpolation never exceeds it, so a
    /// candidate there is always accepted.
    peak: f32,
    patchiness: f32,
}

impl Fertility {
    /// Lattice points per axis for these params, zero when the world is uniform.
    ///
    /// Shared with storage accounting so the budget charges exactly what is built.
    pub(crate) fn cells_per_axis(params: &SimParams) -> u32 {
        let plants = &params.plants;
        if plants.patchiness == 0.0 {
            return 0;
        }
        // Saturating: validation bounds the result before anything is built.
        (libm::roundf(params.world.size / plants.patch_scale) as u32).max(1)
    }

    /// Draws the lattice from `rng`, or draws nothing for a uniform world so the plant
    /// sites that follow are exactly Phase 1's.
    pub(crate) fn new(params: &SimParams, rng: &mut Rng) -> Self {
        let cells = Self::cells_per_axis(params);
        let values: Vec<f32> = (0..u64::from(cells) * u64::from(cells))
            .map(|_| rng.unit())
            .collect();
        let peak = values.iter().copied().fold(0.0, f32::max);
        let world_size = params.world.size;
        Self {
            cells,
            cell_size: if cells == 0 {
                world_size
            } else {
                world_size / cells as f32
            },
            world_size,
            values,
            peak,
            patchiness: params.plants.patchiness,
        }
    }

    /// Fertility at `at`, in `[0, peak]`: the lattice smoothly interpolated, wrapping
    /// across the torus. One everywhere in a uniform world.
    pub(crate) fn at(&self, at: Vec3) -> f32 {
        if self.cells == 0 {
            return 1.0;
        }
        let x = wrap_scalar(at.x, self.world_size) / self.cell_size;
        let y = wrap_scalar(at.y, self.world_size) / self.cell_size;
        let (x0, y0) = (x.floor(), y.floor());
        let (sx, sy) = (smoothstep(x - x0), smoothstep(y - y0));
        // The modulo also folds a coordinate that rounds up to exactly `cells`.
        let (x0, y0) = (x0 as u32 % self.cells, y0 as u32 % self.cells);
        let (x1, y1) = ((x0 + 1) % self.cells, (y0 + 1) % self.cells);
        let value = |cx: u32, cy: u32| self.values[(cy * self.cells + cx) as usize];
        let low = lerp(value(x0, y0), value(x1, y0), sx);
        let high = lerp(value(x0, y1), value(x1, y1), sx);
        lerp(low, high, sy)
    }

    /// Whether a plant can establish at `at`: with probability
    /// `(fertility / peak)^patchiness`. A uniform world accepts without drawing.
    pub(crate) fn accepts(&self, at: Vec3, rng: &mut Rng) -> bool {
        if self.cells == 0 || self.peak <= 0.0 {
            return true;
        }
        let odds = math::pow((self.at(at) / self.peak).min(1.0), self.patchiness);
        rng.unit() < odds
    }

    /// A site drawn by `propose` that this map accepts. After [`MAX_TRIES`] rejections
    /// the last candidate is kept, so the loop always ends.
    pub(crate) fn site(&self, rng: &mut Rng, mut propose: impl FnMut(&mut Rng) -> Vec3) -> Vec3 {
        let mut candidate = Vec3::ZERO;
        for _ in 0..MAX_TRIES {
            candidate = propose(rng);
            if self.accepts(candidate, rng) {
                break;
            }
        }
        candidate
    }

    /// A uniform candidate anywhere on the torus.
    pub(crate) fn anywhere(&self, rng: &mut Rng) -> Vec3 {
        // x before y, matching Phase 1's uniform scatter draw for draw.
        let x = rng.range(0.0, self.world_size);
        let y = rng.range(0.0, self.world_size);
        Vec3::new(x, y, 0.0)
    }

    #[inline]
    pub(crate) fn world_size(&self) -> f32 {
        self.world_size
    }
}

/// Cubic ease, so the map has no creases at lattice lines.
#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patchy(patchiness: f32, scale: f32) -> (Fertility, SimParams) {
        let mut params = SimParams::default();
        params.plants.patchiness = patchiness;
        params.plants.patch_scale = scale;
        (Fertility::new(&params, &mut Rng::from_seed(3)), params)
    }

    #[test]
    fn a_uniform_world_draws_nothing_and_accepts_everywhere() {
        let params = SimParams::default();
        let mut rng = Rng::from_seed(5);
        let before = rng.state_fingerprint();
        let map = Fertility::new(&params, &mut rng);
        assert!(map.accepts(Vec3::new(10.0, 20.0, 0.0), &mut rng));
        assert_eq!(rng.state_fingerprint(), before, "a uniform world drew");
        assert_eq!(map.at(Vec3::new(400.0, 3.0, 0.0)), 1.0);
    }

    #[test]
    fn uniform_sites_are_phase_ones_scatter_draw_for_draw() {
        let (map, params) = patchy(0.0, 150.0);
        let mut ours = Rng::from_seed(11);
        let mut theirs = Rng::from_seed(11);
        let size = params.world.size;
        for _ in 0..100 {
            let expected = Vec3::new(theirs.range(0.0, size), theirs.range(0.0, size), 0.0);
            assert_eq!(map.site(&mut ours, |rng| map.anywhere(rng)), expected);
        }
    }

    #[test]
    fn the_map_wraps_across_the_torus_and_stays_under_its_peak() {
        let (map, params) = patchy(2.0, 150.0);
        let size = params.world.size;
        let mut rng = Rng::from_seed(8);
        for _ in 0..500 {
            let p = Vec3::new(rng.range(0.0, size), rng.range(0.0, size), 0.0);
            let value = map.at(p);
            assert!((0.0..=map.peak).contains(&value), "{value} at {p:?}");
            let shifted = map.at(p + Vec3::new(size, -size, 0.0));
            assert!((value - shifted).abs() < 1e-4, "{value} vs {shifted}");
        }
    }

    #[test]
    fn the_map_is_continuous_across_lattice_lines() {
        let (map, _) = patchy(2.0, 125.0);
        // 125 divides the 1000-unit world, so x = 250 is a lattice line.
        let left = map.at(Vec3::new(250.0 - 1e-3, 70.0, 0.0));
        let right = map.at(Vec3::new(250.0 + 1e-3, 70.0, 0.0));
        assert!((left - right).abs() < 1e-3, "{left} vs {right}");
    }

    #[test]
    fn patchy_sites_land_where_the_soil_is_fertile() {
        let (map, params) = patchy(4.0, 150.0);
        let size = params.world.size;
        let mut rng = Rng::from_seed(21);
        let placed: f32 = (0..2_000)
            .map(|_| map.at(map.site(&mut rng, |rng| map.anywhere(rng))))
            .sum::<f32>()
            / 2_000.0;
        let anywhere: f32 = (0..2_000)
            .map(|_| map.at(Vec3::new(rng.range(0.0, size), rng.range(0.0, size), 0.0)))
            .sum::<f32>()
            / 2_000.0;
        assert!(
            placed > anywhere + 0.15,
            "placed plants average {placed}, the world {anywhere}"
        );
    }

    #[test]
    fn the_same_seed_draws_the_same_map() {
        let (a, _) = patchy(3.0, 90.0);
        let (b, _) = patchy(3.0, 90.0);
        assert_eq!(a.values, b.values);
        assert_eq!(a.cells, 11);
    }
}
