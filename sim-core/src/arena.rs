//! Flat arenas for the per-agent data that is not one value per agent.
//!
//! Brains and genomes are variable-length, so they cannot live in the SoA arrays.
//! They live here, in one contiguous buffer, and an agent holds an `(offset, len)`
//! handle into it. Handles are indices, never pointers: an arena has to serialize as a
//! flat buffer for checkpointing, and a pointer does not survive that.
//!
//! Phase 1 has fixed brain topology, so every block is the same stride and allocation
//! is a free-list pop. The handle carries `len` anyway, so Phase 2's variable-length
//! genomes change this allocator and nothing that calls it.
//!
//! # This is where the simulation's memory is
//!
//! Measured at M4, an agent costs ~11.3 KB and the **genome arena is 98% of it**. The
//! SoA state arrays, the brain arena, the pool and the spatial hash come to 233 bytes
//! between them. Arenas are allocated at `max_agents` and never grown, so the cost is
//! committed at `World::new`: 55 MB at 5k agents, 553 MB at the Phase 7 target of 50k.
//! Anything that changes gene count or gene size moves that number quadratically with
//! the pool, so it is worth knowing before adding a field.
//!
//! Roughly 42% of the genome arena is padding. `Gene` is an enum sized by its widest
//! variant — `SensorGene` at 40 bytes — while ~85% of genes are connections with a
//! 20-byte payload. Splitting the arena by gene class recovers about half the genome
//! arena and needs no new machinery; it is the first lever if this ever has to come
//! down, ahead of allocating lazily. Growth is otherwise cheap to add whenever it is
//! wanted, because handles are indices: a `Vec` realloc leaves every existing handle
//! valid. See spec §7.5.
//!
//! Deliberately not here: what the elements mean. This module stores blocks.

use serde::{Deserialize, Serialize};

/// A block of `len` elements starting at `offset`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Block {
    offset: u32,
    len: u32,
}

impl Block {
    /// The empty block, for slots that hold no data.
    pub const EMPTY: Self = Self { offset: 0, len: 0 };

    #[inline]
    pub const fn offset(self) -> u32 {
        self.offset
    }

    #[inline]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline]
    fn range(self) -> core::ops::Range<usize> {
        self.offset as usize..(self.offset + self.len) as usize
    }
}

/// Fixed-capacity arena of equal-stride blocks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arena<T> {
    data: Vec<T>,
    stride: u32,
    free: Vec<u32>,
    live_blocks: u32,
}

impl<T: Clone + Default> Arena<T> {
    /// Room for `blocks` blocks of up to `stride` elements each.
    pub fn with_capacity(blocks: u32, stride: u32) -> Self {
        debug_assert!(stride > 0, "a zero stride can never satisfy an allocation");
        let mut free = Vec::with_capacity(blocks as usize);
        for i in (0..blocks).rev() {
            free.push(i * stride);
        }
        Self {
            data: vec![T::default(); (blocks as usize) * (stride as usize)],
            stride,
            free,
            live_blocks: 0,
        }
    }

    /// Claims a block of `len` elements, zeroed. `None` when the arena is full.
    pub fn alloc(&mut self, len: u32) -> Option<Block> {
        debug_assert!(
            len <= self.stride,
            "block of {len} exceeds stride {}; Phase 1 topology is fixed",
            self.stride
        );
        let offset = self.free.pop()?;
        let block = Block { offset, len };
        // Reset rather than trusting the previous tenant: a brain must not inherit the
        // activations of the agent that died in this slot.
        for slot in &mut self.data[block.range()] {
            *slot = T::default();
        }
        self.live_blocks += 1;
        Some(block)
    }

    /// Returns a block to the free list. Freeing [`Block::EMPTY`] is a no-op.
    ///
    /// `Block::EMPTY` has offset 0, which is also a perfectly valid block start, so
    /// without this guard freeing one would push a duplicate 0 onto the free list and
    /// the next allocation would hand a live agent's block to a second agent.
    pub fn free(&mut self, block: Block) {
        if block.is_empty() {
            return;
        }
        debug_assert_eq!(block.offset % self.stride, 0, "not a block start");
        debug_assert!(block.offset < self.data.len() as u32, "block out of range");
        debug_assert!(
            self.live_blocks > 0,
            "freed more blocks than were allocated"
        );
        self.free.push(block.offset);
        self.live_blocks -= 1;
    }

    #[inline]
    pub fn get(&self, block: Block) -> &[T] {
        &self.data[block.range()]
    }

    #[inline]
    pub fn get_mut(&mut self, block: Block) -> &mut [T] {
        &mut self.data[block.range()]
    }

    #[inline]
    pub fn stride(&self) -> u32 {
        self.stride
    }

    #[inline]
    pub fn live_blocks(&self) -> u32 {
        self.live_blocks
    }

    #[inline]
    pub fn capacity_blocks(&self) -> u32 {
        (self.data.len() / self.stride as usize) as u32
    }

    /// The whole backing buffer, for checkpointing and hashing.
    #[inline]
    pub fn raw(&self) -> &[T] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_do_not_overlap() {
        let mut arena: Arena<f32> = Arena::with_capacity(4, 8);
        let blocks: Vec<Block> = (0..4).map(|_| arena.alloc(8).unwrap()).collect();
        for (i, a) in blocks.iter().enumerate() {
            for b in &blocks[i + 1..] {
                let (a_end, b_end) = (a.offset() + a.len(), b.offset() + b.len());
                assert!(
                    a_end <= b.offset() || b_end <= a.offset(),
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn full_arena_refuses_rather_than_growing() {
        let mut arena: Arena<f32> = Arena::with_capacity(2, 4);
        assert!(arena.alloc(4).is_some());
        assert!(arena.alloc(4).is_some());
        assert!(arena.alloc(4).is_none());
        assert_eq!(arena.live_blocks(), 2);
    }

    #[test]
    fn writes_stay_inside_their_block() {
        let mut arena: Arena<f32> = Arena::with_capacity(3, 4);
        let a = arena.alloc(4).unwrap();
        let b = arena.alloc(4).unwrap();
        arena.get_mut(a).fill(1.0);
        arena.get_mut(b).fill(2.0);
        assert_eq!(arena.get(a), &[1.0; 4]);
        assert_eq!(arena.get(b), &[2.0; 4]);
    }

    #[test]
    fn a_reused_block_does_not_inherit_the_previous_tenant() {
        // A newborn brain starting with a dead agent's activations is a determinism
        // bug that only shows up once the pool starts recycling.
        let mut arena: Arena<f32> = Arena::with_capacity(1, 4);
        let a = arena.alloc(4).unwrap();
        arena.get_mut(a).fill(7.0);
        arena.free(a);
        let b = arena.alloc(4).unwrap();
        assert_eq!(arena.get(b), &[0.0; 4]);
    }

    #[test]
    fn short_blocks_still_get_a_whole_stride() {
        // Phase 2 genomes vary in length; the handle carries len, and the allocator
        // rounds to the stride. Callers must never see another block's data.
        let mut arena: Arena<u32> = Arena::with_capacity(2, 8);
        let a = arena.alloc(3).unwrap();
        let b = arena.alloc(8).unwrap();
        assert_eq!(arena.get(a).len(), 3);
        assert_eq!(arena.get(b).len(), 8);
        assert!(a.offset() + 8 <= b.offset() || b.offset() + 8 <= a.offset());
    }

    #[test]
    fn freeing_the_empty_block_cannot_alias_a_live_one() {
        // Block::EMPTY and the block at offset 0 are indistinguishable by offset, so
        // this used to hand two agents the same brain.
        let mut arena: Arena<f32> = Arena::with_capacity(4, 8);
        let live = arena.alloc(8).unwrap();
        arena.free(Block::EMPTY);
        let next = arena.alloc(8).unwrap();
        assert_ne!(next.offset(), live.offset(), "aliased a live block");
        assert_eq!(arena.live_blocks(), 2);
    }

    #[test]
    fn round_trips_through_postcard() {
        let mut arena: Arena<f32> = Arena::with_capacity(2, 4);
        let a = arena.alloc(4).unwrap();
        arena.get_mut(a).copy_from_slice(&[1.0, 2.0, 3.0, 4.0]);
        let bytes = postcard::to_allocvec(&arena).expect("serializes");
        let back: Arena<f32> = postcard::from_bytes(&bytes).expect("deserializes");
        assert_eq!(back.get(a), &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(back.live_blocks(), 1);
    }
}
