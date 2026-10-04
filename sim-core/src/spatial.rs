//! Uniform grid spatial hash: the only thing standing between this simulation and
//! O(n²) sensing.
//!
//! Cell size is the largest radius any sensor can query, so a neighbour search touches
//! a small block of cells rather than the whole population. The grid is rebuilt every
//! tick with a counting sort, which is cheap enough that keeping it incremental would
//! be the worse trade.
//!
//! The world is a **torus**: it wraps in x and y, so there is no edge for a population
//! to pile against and no corner that is safe by geometry alone. That makes every
//! distance a minimum-image distance and every cell walk a wrapping one.
//!
//! The cell walk is a real triple-nested x/y/z loop with Z collapsed to one layer,
//! not a flat 2D loop. V1 simulates on a plane; this is one of the hedges that makes
//! volumetric 3D an unlock rather than a rewrite (spec §2.3, §9.1). Do not flatten it.
//!
//! Deliberately not here: what the neighbours mean. This module answers "which indices
//! are near this point"; perception decides what to do with them.

use glam::Vec3;

/// Folds a coordinate into `[0, size)`.
#[inline]
pub fn wrap_scalar(v: f32, size: f32) -> f32 {
    let m = libm::fmodf(v, size);
    if m < 0.0 { m + size } else { m }
}

/// Shortest signed separation between two coordinates on a periodic axis.
#[inline]
fn min_image_scalar(d: f32, size: f32) -> f32 {
    let half = size * 0.5;
    if d > half {
        d - size
    } else if d < -half {
        d + size
    } else {
        d
    }
}

/// Shortest vector between two points on the torus. Z is not periodic — V1 pins it
/// to the plane.
#[inline]
pub fn min_image(delta: Vec3, world_size: f32) -> Vec3 {
    Vec3::new(
        min_image_scalar(delta.x, world_size),
        min_image_scalar(delta.y, world_size),
        0.0,
    )
}

/// The cells to visit along one axis, as absolute indices, wrapping and never
/// repeating.
///
/// The no-repeat part is the subtle bit: on a grid small enough that the search radius
/// reaches the whole way round, a naive `-span..=span` walk visits the same cell twice
/// and reports the neighbours inside it twice. When that would happen this walks every
/// cell on the axis exactly once instead.
struct AxisCells {
    next: u32,
    remaining: u32,
    dim: u32,
}

impl AxisCells {
    fn new(base: u32, span: u32, dim: u32) -> Self {
        debug_assert!(dim > 0 && base < dim);
        let width = span.saturating_mul(2).saturating_add(1);
        if width >= dim {
            Self {
                next: 0,
                remaining: dim,
                dim,
            }
        } else {
            Self {
                next: (base + dim - span % dim) % dim,
                remaining: width,
                dim,
            }
        }
    }
}

impl Iterator for AxisCells {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.remaining == 0 {
            return None;
        }
        let current = self.next;
        self.next = if current + 1 == self.dim {
            0
        } else {
            current + 1
        };
        self.remaining -= 1;
        Some(current)
    }
}

/// Grid over the simulation plane, plus the counting-sort buckets rebuilt each tick.
#[derive(Clone, Debug)]
pub struct SpatialHash {
    /// Cells per axis. `dims[2]` is 1 in V1 (spec §9.1).
    dims: [u32; 3],
    /// Side length of one cell per axis. Z spans the whole (single-layer) world.
    cell: [f32; 3],
    world_size: f32,
    /// `cell_starts[c]..cell_starts[c + 1]` indexes [`Self::entries`] for cell `c`.
    cell_starts: Vec<u32>,
    /// Per-cell write cursor for the scatter pass. A field, not a local, so a rebuild
    /// allocates nothing (spec §2.2a).
    cursor: Vec<u32>,
    /// Live indices, grouped by cell and ascending within each cell.
    entries: Vec<u32>,
    live: u32,
}

impl SpatialHash {
    /// Builds a grid whose cells are at least `min_cell_size` across.
    ///
    /// The requested size is a floor, not an exact value. Cells have to tile the torus
    /// evenly or the seam gets a partial cell, and neighbours vanish across it — so
    /// the cell count is rounded down and the real cell size falls out of the division.
    pub fn new(world_size: f32, min_cell_size: f32, capacity: u32) -> Self {
        debug_assert!(world_size > 0.0, "world must have extent");
        debug_assert!(min_cell_size > 0.0, "cells must have extent");
        Self::with_cells_per_axis(
            world_size,
            Self::cells_per_axis_for(world_size, min_cell_size),
            capacity,
        )
    }

    /// The [`Self::cell_size`] that [`Self::new`] would produce, without allocating the
    /// grid — for readers that only need the retune ceiling a world's params imply.
    pub fn cell_size_for(world_size: f32, min_cell_size: f32) -> f32 {
        world_size / Self::cells_per_axis_for(world_size, min_cell_size) as f32
    }

    fn cells_per_axis_for(world_size: f32, min_cell_size: f32) -> u32 {
        ((world_size / min_cell_size) as u32).max(1)
    }

    /// Cells along each horizontal axis.
    pub fn cells_per_axis(&self) -> u32 {
        self.dims[0]
    }

    /// A grid with an exact cell count, so a restored world keeps the grid it was
    /// built with even after a retune lowered the sensing radius (spec section 2.3).
    pub(crate) fn with_cells_per_axis(world_size: f32, per_axis: u32, capacity: u32) -> Self {
        debug_assert!(per_axis > 0, "a grid needs at least one cell");
        // World construction budgets this separately; the standalone constructor
        // still requires a representable cell-start buffer on WASM32 (spec section 2.2a).
        debug_assert!(
            (u64::from(per_axis) * u64::from(per_axis) + 1)
                <= i32::MAX as u64 / size_of::<u32>() as u64,
            "grid cell-start buffer exceeds the portable byte ceiling"
        );
        let dims = [per_axis, per_axis, 1];
        let cell_xy = world_size / per_axis as f32;
        let cells = cell_count(dims);
        Self {
            dims,
            // Z spans the world so the single layer always covers any query.
            cell: [cell_xy, cell_xy, world_size],
            world_size,
            cell_starts: vec![0; cells + 1],
            cursor: vec![0; cells],
            entries: vec![0; capacity as usize],
            live: 0,
        }
    }

    /// Rebuilds the grid from current positions in one counting-sort pass, and writes
    /// each live entry's bucket back into `grid_cell`.
    pub fn rebuild(&mut self, positions: &[Vec3], alive: &[u8], grid_cell: &mut [u32]) {
        debug_assert_eq!(positions.len(), alive.len());
        debug_assert_eq!(positions.len(), grid_cell.len());
        debug_assert!(
            positions.len() <= self.entries.len(),
            "pool outgrew the hash"
        );

        let cells = cell_count(self.dims);
        self.cell_starts.fill(0);

        // Count into cell_starts[c + 1] so the prefix sum below lands in place.
        for (i, &pos) in positions.iter().enumerate() {
            if alive[i] == 0 {
                continue;
            }
            // A non-finite coordinate casts to cell 0 rather than trapping, so without
            // this the whole population appears to pile onto the wrap seam and the
            // actual cause — an over-large force, a divide by zero in an effector — is
            // nowhere near the symptom.
            debug_assert!(
                pos.x.is_finite() && pos.y.is_finite(),
                "agent {i} has a non-finite position: {pos:?}"
            );
            let c = self.cell_of(pos);
            grid_cell[i] = c;
            self.cell_starts[c as usize + 1] += 1;
        }

        for c in 0..cells {
            self.cell_starts[c + 1] += self.cell_starts[c];
        }
        self.live = self.cell_starts[cells];
        self.cursor.copy_from_slice(&self.cell_starts[..cells]);

        // Ascending index, so entries within a cell come out in agent-index order and
        // every neighbour walk is order-stable (spec §2.4). Reads the bucket written
        // above rather than recomputing it, so the two passes cannot disagree.
        for (i, &flag) in alive.iter().enumerate() {
            if flag == 0 {
                continue;
            }
            let c = grid_cell[i] as usize;
            self.entries[self.cursor[c] as usize] = i as u32;
            self.cursor[c] += 1;
        }
    }

    /// Calls `visit(index, offset, distance_squared)` for every live entry within
    /// `radius` of `center`, where `offset` is the minimum-image vector from `center`
    /// to that entry.
    ///
    /// `offset` is handed over rather than left to the caller because across the wrap
    /// seam it is not `position - center`, and recomputing it wrong points a sensor
    /// the long way round the world.
    pub fn for_each_within(
        &self,
        positions: &[Vec3],
        center: Vec3,
        radius: f32,
        mut visit: impl FnMut(u32, Vec3, f32),
    ) {
        debug_assert!(
            radius * 2.0 <= self.world_size,
            "radius past half the world makes the minimum image ambiguous"
        );
        let r2 = radius * radius;
        let base = self.cell_coords(center);
        let span = self.spans(radius);

        // Triple-nested with Z pinned to its single layer (spec §2.3, §9.1).
        for cz in AxisCells::new(base[2], span[2], self.dims[2]) {
            for cy in AxisCells::new(base[1], span[1], self.dims[1]) {
                for cx in AxisCells::new(base[0], span[0], self.dims[0]) {
                    let c = self.cell_index([cx, cy, cz]) as usize;
                    let (from, to) = (
                        self.cell_starts[c] as usize,
                        self.cell_starts[c + 1] as usize,
                    );
                    for &entry in &self.entries[from..to] {
                        let offset = min_image(positions[entry as usize] - center, self.world_size);
                        let d2 = offset.length_squared();
                        if d2 <= r2 {
                            visit(entry, offset, d2);
                        }
                    }
                }
            }
        }
    }

    /// Cells reached on each axis by a query of this radius.
    fn spans(&self, radius: f32) -> [u32; 3] {
        let mut span = [0u32; 3];
        for (s, &size) in span.iter_mut().zip(self.cell.iter()) {
            *s = libm::ceilf(radius / size) as u32;
        }
        span
    }

    fn cell_coords(&self, pos: Vec3) -> [u32; 3] {
        let x = wrap_scalar(pos.x, self.world_size);
        let y = wrap_scalar(pos.y, self.world_size);
        [
            ((x / self.cell[0]) as u32).min(self.dims[0] - 1),
            ((y / self.cell[1]) as u32).min(self.dims[1] - 1),
            0,
        ]
    }

    fn cell_index(&self, coords: [u32; 3]) -> u32 {
        (coords[2] * self.dims[1] + coords[1]) * self.dims[0] + coords[0]
    }

    fn cell_of(&self, pos: Vec3) -> u32 {
        self.cell_index(self.cell_coords(pos))
    }

    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    /// Extent of one cell along x and y.
    ///
    /// Also the ceiling on any sensing radius this world can be retuned to: a query
    /// wider than a cell would have to walk more than one ring, which the neighbour loop
    /// does not do (spec §2.3). `World::set_params` refuses on that basis.
    pub fn cell_size(&self) -> f32 {
        self.cell[0]
    }

    /// Number of live entries in the grid as of the last rebuild.
    pub fn live(&self) -> u32 {
        self.live
    }
}

fn cell_count(dims: [u32; 3]) -> usize {
    dims[0] as usize * dims[1] as usize * dims[2] as usize
}

/// Obviously-correct O(n²) neighbour search, kept as the reference the grid is checked
/// against.
///
/// This is not dead code and must not be deleted once the fast path works. It is what
/// makes optimising the grid safe rather than nerve-wracking: any change to the hash
/// is checked against this on random populations, and a discrepancy is a bug in the
/// fast path by definition (spec §7.8).
pub fn brute_force_within(
    positions: &[Vec3],
    alive: &[u8],
    world_size: f32,
    center: Vec3,
    radius: f32,
    mut visit: impl FnMut(u32, Vec3, f32),
) {
    let r2 = radius * radius;
    for (i, &pos) in positions.iter().enumerate() {
        if alive[i] == 0 {
            continue;
        }
        let offset = min_image(pos - center, world_size);
        let d2 = offset.length_squared();
        if d2 <= r2 {
            visit(i as u32, offset, d2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const WORLD: f32 = 100.0;

    #[test]
    fn representable_grids_are_not_rejected_by_the_old_static_ceiling() {
        let hash = SpatialHash::new(4_097.0, 1.0, 0);
        assert_eq!(hash.dims, [4_097, 4_097, 1]);
        assert_eq!(hash.cell_size(), 1.0);
    }

    fn grid(min_cell: f32, capacity: u32) -> SpatialHash {
        SpatialHash::new(WORLD, min_cell, capacity)
    }

    /// `(positions, alive)` from a compact description, so tests read as coordinates.
    fn population(spec: &[(f32, f32, bool)]) -> (Vec<Vec3>, Vec<u8>, Vec<u32>) {
        let positions = spec.iter().map(|&(x, y, _)| Vec3::new(x, y, 0.0)).collect();
        let alive = spec.iter().map(|&(_, _, a)| a as u8).collect();
        (positions, alive, vec![0; spec.len()])
    }

    /// Sorted `(index, offset, distance²)` triples, so hash and reference are directly
    /// comparable.
    fn collect(run: impl FnOnce(&mut dyn FnMut(u32, Vec3, f32))) -> Vec<(u32, [u32; 2], u32)> {
        let mut out = Vec::new();
        // Bit patterns, not floats: the two paths share one min_image, so agreement
        // should be exact and a tolerance would hide a real divergence.
        run(&mut |i, offset, d2| {
            out.push((i, [offset.x.to_bits(), offset.y.to_bits()], d2.to_bits()))
        });
        out.sort();
        out
    }

    #[test]
    fn cells_tile_the_torus_exactly() {
        // A partial cell at the seam loses neighbours across the wrap.
        for min_cell in [7.0, 10.0, 33.0, 60.0, 150.0] {
            let h = grid(min_cell, 8);
            let [nx, ny, nz] = h.dims();
            assert_eq!(nz, 1, "Z must stay a single layer in V1");
            // A sense radius wider than the world collapses to a single cell — the
            // grid cannot offer more than it has. `SimParams::validate` rejects that
            // configuration long before it reaches here.
            let achievable = min_cell.min(WORLD);
            assert!(
                h.cell_size() >= achievable - 1e-6,
                "cells shrank below the sense radius: {} < {achievable}",
                h.cell_size()
            );
            assert!(
                (nx as f32 * h.cell_size() - WORLD).abs() < 1e-3,
                "grid does not tile"
            );
            assert_eq!(nx, ny);
            assert_eq!(
                SpatialHash::cell_size_for(WORLD, min_cell).to_bits(),
                h.cell_size().to_bits(),
                "the allocation-free estimate must match the built grid"
            );
        }
    }

    #[test]
    fn rebuild_groups_live_entries_and_skips_the_dead() {
        let (pos, alive, mut cells) = population(&[
            (5.0, 5.0, true),
            (5.0, 5.0, false),
            (95.0, 95.0, true),
            (5.0, 5.0, true),
        ]);
        let mut h = grid(10.0, 4);
        h.rebuild(&pos, &alive, &mut cells);
        assert_eq!(h.live(), 3);
        assert_eq!(
            cells[0], cells[3],
            "same position must land in the same cell"
        );
        assert_ne!(cells[0], cells[2]);

        let found = collect(|v| h.for_each_within(&pos, Vec3::new(5.0, 5.0, 0.0), 1.0, v));
        let ids: Vec<u32> = found.iter().map(|t| t.0).collect();
        assert_eq!(
            ids,
            vec![0, 3],
            "dead entry reported, or order not ascending"
        );
    }

    #[test]
    fn neighbours_are_found_across_the_wrap_seam() {
        // Two agents either side of x = 0, four units apart the short way and 96 the
        // long way. An implementation that forgets the torus finds neither.
        let (pos, alive, mut cells) = population(&[(1.0, 50.0, true), (97.0, 50.0, true)]);
        let mut h = grid(10.0, 2);
        h.rebuild(&pos, &alive, &mut cells);

        let found = collect(|v| h.for_each_within(&pos, pos[0], 5.0, v));
        let ids: Vec<u32> = found.iter().map(|t| t.0).collect();
        assert_eq!(ids, vec![0, 1], "missed the neighbour across the seam");

        let mut offset = Vec3::ZERO;
        h.for_each_within(&pos, pos[0], 5.0, |i, o, _| {
            if i == 1 {
                offset = o;
            }
        });
        assert!(
            (offset.x - -4.0).abs() < 1e-4,
            "offset went the long way round: {offset:?}"
        );
    }

    #[test]
    fn a_small_grid_does_not_report_the_same_neighbour_twice() {
        // Radius reaching the whole way round makes a naive -span..=span walk revisit
        // cells, double-counting everyone inside them.
        for min_cell in [60.0, 40.0, 34.0] {
            let (pos, alive, mut cells) = population(&[(10.0, 10.0, true), (60.0, 60.0, true)]);
            let mut h = grid(min_cell, 2);
            h.rebuild(&pos, &alive, &mut cells);
            let found = collect(|v| h.for_each_within(&pos, Vec3::new(10.0, 10.0, 0.0), 49.0, v));
            let ids: Vec<u32> = found.iter().map(|t| t.0).collect();
            let mut unique = ids.clone();
            unique.dedup();
            assert_eq!(
                ids, unique,
                "duplicate reports at cell size {min_cell}: {ids:?}"
            );
        }
    }

    #[test]
    fn entries_within_a_cell_stay_in_ascending_index_order() {
        // Behaviour must not become an artifact of insertion order (spec §2.4).
        let spec: Vec<(f32, f32, bool)> = (0..64)
            .map(|i| (5.0, 5.0 + i as f32 * 0.01, true))
            .collect();
        let (pos, alive, mut cells) = population(&spec);
        let mut h = grid(10.0, 64);
        h.rebuild(&pos, &alive, &mut cells);
        let mut seen = Vec::new();
        h.for_each_within(&pos, Vec3::new(5.0, 5.0, 0.0), 5.0, |i, _, _| seen.push(i));
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(seen, sorted);
    }

    #[test]
    fn a_shrinking_population_leaves_no_ghosts() {
        // Entries past `live` keep the previous rebuild's indices; only cell_starts
        // stops them being read. If that bound ever slips, dead agents keep sensing.
        let pos: Vec<Vec3> = (0..20)
            .map(|i| Vec3::new(i as f32 * 4.0, 10.0, 0.0))
            .collect();
        let mut cells = vec![0u32; 20];
        let mut h = grid(10.0, 20);

        h.rebuild(&pos, &[1u8; 20], &mut cells);
        assert_eq!(h.live(), 20);

        let mut alive = vec![1u8; 20];
        for flag in alive.iter_mut().skip(3) {
            *flag = 0;
        }
        h.rebuild(&pos, &alive, &mut cells);
        assert_eq!(h.live(), 3);

        let found = collect(|v| h.for_each_within(&pos, Vec3::new(40.0, 10.0, 0.0), 45.0, v));
        let ids: Vec<u32> = found.iter().map(|t| t.0).collect();
        assert_eq!(
            ids,
            vec![0, 1, 2],
            "stale entries surfaced as live neighbours"
        );
    }

    #[test]
    fn rebuilding_twice_gives_the_same_grid() {
        let spec: Vec<(f32, f32, bool)> = (0..50)
            .map(|i| ((i * 7 % 100) as f32, (i * 13 % 100) as f32, i % 3 != 0))
            .collect();
        let (pos, alive, mut cells) = population(&spec);
        let mut h = grid(10.0, 50);
        h.rebuild(&pos, &alive, &mut cells);
        let first = collect(|v| h.for_each_within(&pos, Vec3::new(50.0, 50.0, 0.0), 30.0, v));
        h.rebuild(&pos, &alive, &mut cells);
        let second = collect(|v| h.for_each_within(&pos, Vec3::new(50.0, 50.0, 0.0), 30.0, v));
        assert_eq!(first, second);
    }

    proptest! {
        /// The acceptance criterion for the spatial hash: it must agree with the
        /// obviously-correct O(n²) version on random populations (spec §7.8).
        #[test]
        fn matches_the_brute_force_reference(
            spec in prop::collection::vec((0f32..WORLD, 0f32..WORLD, any::<bool>()), 0..120),
            cx in 0f32..WORLD,
            cy in 0f32..WORLD,
            radius in 0.5f32..(WORLD / 2.0),
            min_cell in 3f32..70f32,
        ) {
            let (pos, alive, mut cells) = population(&spec);
            let mut h = SpatialHash::new(WORLD, min_cell, spec.len().max(1) as u32);
            h.rebuild(&pos, &alive, &mut cells);

            let center = Vec3::new(cx, cy, 0.0);
            let fast = collect(|v| h.for_each_within(&pos, center, radius, v));
            let slow = collect(|v| brute_force_within(&pos, &alive, WORLD, center, radius, v));
            prop_assert_eq!(fast, slow);
        }

        /// Positions outside `[0, size)` must fold onto the torus rather than landing
        /// in a clamped edge cell, which would quietly pile every escapee together.
        #[test]
        fn out_of_range_positions_wrap(v in -1000f32..1000f32) {
            let w = wrap_scalar(v, WORLD);
            prop_assert!((0.0..WORLD).contains(&w), "{} wrapped to {}", v, w);
            let turns = (v - w) / WORLD;
            prop_assert!((turns - turns.round()).abs() < 1e-3);
        }

        #[test]
        fn min_image_never_exceeds_half_the_world(dx in -WORLD..WORLD, dy in -WORLD..WORLD) {
            let m = min_image(Vec3::new(dx, dy, 0.0), WORLD);
            prop_assert!(m.x.abs() <= WORLD / 2.0 + 1e-4);
            prop_assert!(m.y.abs() <= WORLD / 2.0 + 1e-4);
            prop_assert_eq!(m.z, 0.0);
        }
    }
}
