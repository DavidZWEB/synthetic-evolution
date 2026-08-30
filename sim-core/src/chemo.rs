//! The pheromone field: a grid of scalar concentrations the chemo sensor reads.
//!
//! A 3D grid whose Z extent V1 pins to a single cell, for the same reason the spatial
//! hash keeps a triple-nested loop — going volumetric should be an unclamping, not a
//! rewrite (spec §9.1). Channels are separate fields stacked in one buffer, because a
//! food trail and an alarm signal want different half-lives and must not blur into each
//! other (spec §5.5).
//!
//! Like the world it covers, the grid is a **torus**: cell walks and the finite
//! differences that make a gradient both wrap, so there is no seam where a trail
//! mysteriously stops.
//!
//! **Diffusion is what makes this field smellable at all.** A deposit lands in one
//! cell, so without spreading, the gradient is zero everywhere except the single cell
//! holding food — an agent one cell away senses nothing, and there is no slope to
//! climb. That makes diffusion part of the sensor working rather than part of the
//! economy, which is why it is here at M6 rather than with the plants at M7.
//!
//! Deliberately not here: what puts anything into the field. Plants, the `emit_chemo`
//! effector, and the energy ledger that has to account for both arrive at M7.

use core::mem;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::params::ChemoParams;
use crate::spatial::wrap_scalar;

/// Scalar concentrations over the world, one grid per channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChemoField {
    /// Channel-major: each channel's grid is contiguous, so a per-channel decay or
    /// diffusion pass is a linear walk rather than a strided one.
    cells: Vec<f32>,
    /// Double buffer for diffusion, which cannot be done in place — a cell updated
    /// early would be read as a neighbour by the cells after it, and the field would
    /// smear in whatever direction the loop happens to run.
    ///
    /// Sized once alongside `cells` so the tick allocates nothing.
    scratch: Vec<f32>,
    dims: [u32; 3],
    channels: usize,
    world_size: f32,
    /// World units per cell on x and y. Derived from `dims`, kept so sampling is a
    /// multiply rather than a divide.
    inv_cell: f32,
    cell_size: f32,
}

impl ChemoField {
    /// Allocates the grid at full size. Like every other buffer in the sim it is sized
    /// once and never grown (spec §7.3).
    pub fn new(params: &ChemoParams, world_size: f32) -> Self {
        let dims = params.cells;
        let channels = params.channels();
        let per_channel = dims[0] as usize * dims[1] as usize * dims[2] as usize;
        let cell_size = world_size / dims[0] as f32;
        Self {
            cells: vec![0.0; per_channel * channels],
            scratch: vec![0.0; per_channel * channels],
            dims,
            channels,
            world_size,
            inv_cell: 1.0 / cell_size,
            cell_size,
        }
    }

    #[inline]
    pub fn channels(&self) -> usize {
        self.channels
    }

    #[inline]
    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    #[inline]
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Grid coordinates of a world position, wrapped onto the torus.
    #[inline]
    fn coords(&self, position: Vec3) -> [u32; 3] {
        let x = wrap_scalar(position.x, self.world_size) * self.inv_cell;
        let y = wrap_scalar(position.y, self.world_size) * self.inv_cell;
        [
            (x as u32).min(self.dims[0] - 1),
            (y as u32).min(self.dims[1] - 1),
            0,
        ]
    }

    /// Index of a cell within one channel's grid. Coordinates wrap.
    #[inline]
    fn index(&self, coords: [u32; 3]) -> usize {
        cell_index(self.dims, coords)
    }

    /// Cells in one channel's grid.
    #[inline]
    fn per_channel(&self) -> usize {
        self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize
    }

    /// Offset of a channel's grid within the buffer.
    #[inline]
    fn channel_base(&self, channel: usize) -> usize {
        channel * self.per_channel()
    }

    /// Concentration in the cell containing `position`. Zero for a channel that does
    /// not exist, so a mutated sensor gene naming channel 9 goes blind rather than
    /// panicking.
    #[inline]
    pub fn sample(&self, channel: usize, position: Vec3) -> f32 {
        if channel >= self.channels {
            return 0.0;
        }
        self.cells[self.channel_base(channel) + self.index(self.coords(position))]
    }

    /// Direction and steepness of increasing concentration at `position`, as a central
    /// difference over `radius`.
    ///
    /// The sensor's own radius sets the baseline rather than a fixed one cell: a wide
    /// chemoreceptor is one that averages over distance, so it reads a shallow gradient
    /// a narrow one would miss and ignores local noise a narrow one would chase. That
    /// makes `radius` a trait worth evolving instead of a constant in disguise.
    ///
    /// Z is always zero — V1 is a plane (spec §2.3).
    pub fn gradient(&self, channel: usize, position: Vec3, radius: f32) -> Vec3 {
        if channel >= self.channels {
            return Vec3::ZERO;
        }
        // At least one cell, or both samples land in the same cell and every gradient
        // in the world reads as exactly zero.
        let step = radius.max(self.cell_size);
        let along = |axis: Vec3| {
            let ahead = self.sample(channel, position + axis * step);
            let behind = self.sample(channel, position - axis * step);
            (ahead - behind) / (2.0 * step)
        };
        Vec3::new(along(Vec3::X), along(Vec3::Y), 0.0)
    }

    /// Adds `amount` to the cell containing `position`. A no-op for a channel that does
    /// not exist.
    ///
    /// Deposits into a single cell rather than splatting across neighbours: diffusion is
    /// the mechanism that spreads a deposit, and doing both would blur trails twice
    /// (M7, spec §5.5).
    pub fn deposit(&mut self, channel: usize, position: Vec3, amount: f32) {
        if channel >= self.channels {
            return;
        }
        let index = self.channel_base(channel) + self.index(self.coords(position));
        self.cells[index] += amount;
    }

    /// Spreads every channel by one tick of diffusion, then fades it by its own decay
    /// rate. Step 8 of the tick, after whatever deposited into it (spec §2.4).
    ///
    /// Per-channel decay, because a food trail and an alarm want different half-lives
    /// and a single rate would force them to share one (spec §5.5). Both rates are
    /// clamped rather than trusted: they arrive from JS at runtime, and a decay above 1
    /// is an exponentially growing field that fills with `inf` inside a minute.
    pub fn update(&mut self, params: &ChemoParams) {
        let rate = params.diffuse.clamp(0.0, 1.0);
        let per_channel = self.per_channel();
        let dims = self.dims;

        for channel in 0..self.channels {
            let range = channel * per_channel..(channel + 1) * per_channel;
            let src = &self.cells[range.clone()];
            let dst = &mut self.scratch[range];
            // A real triple-nested x/y/z walk with Z collapsed to its single layer, for
            // the same reason the spatial hash keeps one: V1 is a plane and 3D should
            // be an unclamping (spec §2.3, §9.1). Do not flatten it.
            for z in 0..dims[2] {
                for y in 0..dims[1] {
                    for x in 0..dims[0] {
                        let here = cell_index(dims, [x, y, z]);
                        let (sum, count) = neighbour_sum(src, dims, [x, y, z]);
                        // An axis one cell deep has no neighbours but itself, and
                        // averaging a cell against itself is a no-op, not a diffusion.
                        dst[here] = if count == 0.0 {
                            src[here]
                        } else {
                            src[here] + rate * (sum / count - src[here])
                        };
                    }
                }
            }
        }
        mem::swap(&mut self.cells, &mut self.scratch);

        for channel in 0..self.channels {
            let keep = params
                .decay
                .get(channel)
                .copied()
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            for cell in &mut self.cells[channel * per_channel..(channel + 1) * per_channel] {
                *cell *= keep;
            }
        }
    }

    /// Every cell of one channel, for tests and for whatever reads the field whole.
    /// Empty for a channel that does not exist.
    #[inline]
    pub fn channel(&self, channel: usize) -> &[f32] {
        if channel >= self.channels {
            return &[];
        }
        let base = self.channel_base(channel);
        &self.cells[base..base + self.per_channel()]
    }

    /// Zeroes every channel.
    pub fn clear(&mut self) {
        self.cells.fill(0.0);
    }
}

/// Flat index of a cell within one channel's grid. Coordinates wrap.
#[inline]
fn cell_index(dims: [u32; 3], coords: [u32; 3]) -> usize {
    let x = coords[0] % dims[0];
    let y = coords[1] % dims[1];
    let z = coords[2] % dims[2];
    ((z * dims[1] + y) * dims[0] + x) as usize
}

/// Sum of the face-adjacent neighbours of a cell, and how many there were.
///
/// An axis with a single cell contributes nothing: its "neighbours" both wrap back onto
/// the cell itself, and folding a cell into its own average would make diffusion a
/// weaker no-op that still looks like it is working.
#[inline]
fn neighbour_sum(cells: &[f32], dims: [u32; 3], at: [u32; 3]) -> (f32, f32) {
    let mut sum = 0.0;
    let mut count = 0.0;
    for axis in 0..3 {
        if dims[axis] < 2 {
            continue;
        }
        let mut ahead = at;
        let mut behind = at;
        ahead[axis] = (at[axis] + 1) % dims[axis];
        behind[axis] = (at[axis] + dims[axis] - 1) % dims[axis];
        sum += cells[cell_index(dims, ahead)] + cells[cell_index(dims, behind)];
        count += 2.0;
    }
    (sum, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> ChemoField {
        ChemoField::new(&ChemoParams::default(), 1_000.0)
    }

    #[test]
    fn a_deposit_reads_back_where_it_was_left() {
        let mut f = field();
        let spot = Vec3::new(500.0, 500.0, 0.0);
        assert_eq!(f.sample(0, spot), 0.0);
        f.deposit(0, spot, 2.5);
        assert_eq!(f.sample(0, spot), 2.5);
        // ...and nowhere else.
        assert_eq!(f.sample(0, Vec3::new(100.0, 100.0, 0.0)), 0.0);
    }

    #[test]
    fn deposits_into_one_cell_accumulate() {
        let mut f = field();
        let spot = Vec3::new(12.0, 34.0, 0.0);
        f.deposit(0, spot, 1.0);
        f.deposit(0, spot, 0.5);
        assert_eq!(f.sample(0, spot), 1.5);
    }

    #[test]
    fn the_gradient_points_uphill() {
        // The one property the whole food-seeking behaviour rests on: an agent that
        // steers along this vector gets closer to the source.
        let mut f = field();
        let source = Vec3::new(500.0, 500.0, 0.0);
        f.deposit(0, source, 10.0);

        let west = Vec3::new(500.0 - f.cell_size(), 500.0, 0.0);
        let g = f.gradient(0, west, f.cell_size());
        assert!(
            g.x > 0.0,
            "gradient should point east toward the source: {g}"
        );
        assert_eq!(g.z, 0.0, "V1 is a plane");

        let south = Vec3::new(500.0, 500.0 - f.cell_size(), 0.0);
        let g = f.gradient(0, south, f.cell_size());
        assert!(
            g.y > 0.0,
            "gradient should point north toward the source: {g}"
        );
    }

    #[test]
    fn a_flat_field_has_no_gradient() {
        let f = field();
        let g = f.gradient(0, Vec3::new(250.0, 750.0, 0.0), 40.0);
        assert_eq!(g, Vec3::ZERO);
    }

    #[test]
    fn the_gradient_never_reads_a_single_cell_against_itself() {
        // A radius under one cell used to sample the same cell twice and report zero,
        // which reads as "no food anywhere" rather than "this nose is short-sighted".
        let mut f = field();
        let source = Vec3::new(500.0, 500.0, 0.0);
        f.deposit(0, source, 10.0);
        let west = Vec3::new(500.0 - f.cell_size(), 500.0, 0.0);
        for radius in [0.0, 0.001, 1.0, f.cell_size() * 0.5] {
            let g = f.gradient(0, west, radius);
            assert!(g.x > 0.0, "radius {radius} went blind: {g}");
        }
    }

    #[test]
    fn diffusion_spreads_a_deposit_into_a_climbable_slope() {
        // The whole reason diffusion is here at M6 and not with the plants: a raw
        // deposit occupies one cell, so the gradient is zero everywhere except inside
        // it. Nothing can climb that. Spreading is what turns food into a signal.
        let mut f = field();
        let source = Vec3::new(500.0, 500.0, 0.0);
        let params = ChemoParams {
            decay: vec![1.0],
            diffuse: 0.5,
            ..ChemoParams::default()
        };
        f.deposit(0, source, 1_000.0);

        let near = Vec3::new(500.0 + f.cell_size() * 3.0, 500.0, 0.0);
        let far = Vec3::new(500.0 + f.cell_size() * 6.0, 500.0, 0.0);
        assert_eq!(f.sample(0, near), 0.0, "a raw deposit reaches nowhere");

        for _ in 0..150 {
            f.update(&params);
        }
        assert!(f.sample(0, near) > f.sample(0, far), "no slope to climb");
        assert!(f.sample(0, far) > 0.0, "did not reach six cells out");
        assert!(
            f.gradient(0, near, f.cell_size()).x < 0.0,
            "gradient east of the source should point back west toward it"
        );
    }

    #[test]
    fn diffusion_conserves_what_it_spreads() {
        // Diffusion moves concentration, it does not create or destroy it. Decay is the
        // only sink, and it has to be the only one or the field quietly gains signal
        // from nowhere.
        let mut f = field();
        let params = ChemoParams {
            decay: vec![1.0],
            diffuse: 0.4,
            ..ChemoParams::default()
        };
        f.deposit(0, Vec3::new(300.0, 700.0, 0.0), 100.0);
        f.deposit(0, Vec3::new(800.0, 100.0, 0.0), 40.0);
        let before: f32 = f.channel(0).iter().sum();

        for _ in 0..50 {
            f.update(&params);
        }
        let after: f32 = f.channel(0).iter().sum();
        assert!(
            (after - before).abs() < before * 1e-3,
            "total went {before} -> {after}"
        );
    }

    #[test]
    fn decay_is_per_channel() {
        // A trail and an alarm want different half-lives. One shared rate would force
        // every signal in the world to fade at the same speed (spec §5.5).
        let params = ChemoParams {
            decay: vec![0.5, 1.0],
            diffuse: 0.0,
            ..ChemoParams::default()
        };
        let mut f = ChemoField::new(&params, 1_000.0);
        let spot = Vec3::new(400.0, 400.0, 0.0);
        f.deposit(0, spot, 1.0);
        f.deposit(1, spot, 1.0);

        f.update(&params);
        assert!((f.sample(0, spot) - 0.5).abs() < 1e-6, "fast channel");
        assert!((f.sample(1, spot) - 1.0).abs() < 1e-6, "slow channel");
    }

    #[test]
    fn a_runaway_decay_rate_cannot_grow_the_field() {
        // Params arrive from JS at runtime, so "nobody would set that" is not a
        // guarantee. A decay above 1 is an exponentially growing field that reaches
        // `inf` in under a minute of sim time (spec §7.6).
        let params = ChemoParams {
            decay: vec![4.0],
            diffuse: 9.0,
            ..ChemoParams::default()
        };
        let mut f = ChemoField::new(&params, 1_000.0);
        f.deposit(0, Vec3::new(500.0, 500.0, 0.0), 1.0);
        for _ in 0..200 {
            f.update(&params);
        }
        let total: f32 = f.channel(0).iter().sum();
        assert!(
            total.is_finite() && total <= 1.0 + 1e-3,
            "field grew to {total}"
        );
    }

    #[test]
    fn diffusion_does_not_smear_in_the_direction_of_the_loop() {
        // Diffusing in place would let a cell updated early be read as a neighbour by
        // the cells after it, dragging the field toward increasing x and y. Symmetry
        // around a single source is what says the double buffer is really being used.
        let mut f = field();
        let params = ChemoParams {
            decay: vec![1.0],
            diffuse: 0.5,
            ..ChemoParams::default()
        };
        let source = Vec3::new(500.0, 500.0, 0.0);
        f.deposit(0, source, 1_000.0);
        for _ in 0..20 {
            f.update(&params);
        }
        let step = f.cell_size() * 4.0;
        let east = f.sample(0, source + Vec3::X * step);
        let west = f.sample(0, source - Vec3::X * step);
        let north = f.sample(0, source + Vec3::Y * step);
        let south = f.sample(0, source - Vec3::Y * step);
        assert!((east - west).abs() < east * 1e-3, "{east} vs {west}");
        assert!((north - south).abs() < north * 1e-3, "{north} vs {south}");
        assert!((east - north).abs() < east * 1e-3, "{east} vs {north}");
    }

    #[test]
    fn the_field_wraps_like_the_world_it_covers() {
        // A trail laid just inside one edge must be smellable from just inside the
        // other. The world is a torus and the field has to agree, or there is a seam
        // where scent inexplicably stops (spec §2.3).
        let mut f = field();
        f.deposit(0, Vec3::new(5.0, 500.0, 0.0), 4.0);
        assert_eq!(
            f.sample(0, Vec3::new(1_005.0, 500.0, 0.0)),
            4.0,
            "a position past the far edge should wrap onto the deposit"
        );
        assert_eq!(f.sample(0, Vec3::new(-995.0, 500.0, 0.0)), 4.0);

        // And the gradient across the seam points the short way round.
        let g = f.gradient(0, Vec3::new(998.0, 500.0, 0.0), f.cell_size());
        assert!(g.x > 0.0, "gradient did not cross the seam: {g}");
    }

    #[test]
    fn channels_do_not_bleed_into_each_other() {
        // A trail and an alarm are different signals; if they shared cells, evolving to
        // follow one would mean following both (spec §5.5).
        let params = ChemoParams {
            decay: vec![0.98, 0.9],
            ..ChemoParams::default()
        };
        let mut f = ChemoField::new(&params, 1_000.0);
        let spot = Vec3::new(400.0, 400.0, 0.0);
        f.deposit(1, spot, 3.0);
        assert_eq!(f.sample(0, spot), 0.0);
        assert_eq!(f.sample(1, spot), 3.0);
    }

    #[test]
    fn an_out_of_range_channel_reads_blind_rather_than_panicking() {
        // A sensor gene's channel param is a mutable float. Nothing stops a lineage
        // from naming a channel that does not exist, and `sim-core` must not panic in
        // release on anything that passed validation (CLAUDE.md).
        let mut f = field();
        f.deposit(99, Vec3::ZERO, 5.0);
        assert_eq!(f.sample(99, Vec3::ZERO), 0.0);
        assert_eq!(f.gradient(99, Vec3::ZERO, 10.0), Vec3::ZERO);
    }

    #[test]
    fn every_position_lands_in_a_real_cell() {
        // Sampling is index arithmetic on a wrapped float; a position on an exact edge,
        // far outside the world, or negative must still index inside the buffer.
        let f = field();
        let size = 1_000.0;
        for &x in &[-3_000.0, -0.0001, 0.0, size - 0.0001, size, 4_000.5] {
            for &y in &[-3_000.0, 0.0, size, 4_000.5] {
                assert_eq!(f.sample(0, Vec3::new(x, y, 0.0)), 0.0, "{x},{y} panicked?");
            }
        }
    }
}
