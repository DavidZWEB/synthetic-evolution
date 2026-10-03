//! Preallocated, variable-length storage with deterministic first-fit allocation.
//!
//! Owns elements and address-ordered free spans, coalescing adjacent spans on release
//! without moving live blocks (spec section 2.2a). It does not choose world budgets,
//! manage births, or grow/compact storage.

use std::collections::TryReserveError;

use super::Block;

/// A construction request that cannot reserve portable backing buffers.
#[derive(Debug)]
pub enum ArenaBuildError {
    /// An element or metadata buffer exceeds the shared native/WASM32 byte ceiling.
    CapacityTooLarge,
    /// The host could not reserve a representable buffer.
    Reservation(TryReserveError),
}

impl core::fmt::Display for ArenaBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CapacityTooLarge => f.write_str("arena buffer exceeds the portable byte ceiling"),
            Self::Reservation(error) => write!(f, "arena reservation failed: {error}"),
        }
    }
}

impl core::error::Error for ArenaBuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::CapacityTooLarge => None,
            Self::Reservation(error) => Some(error),
        }
    }
}

/// A refused allocation. Every refusal leaves the arena unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationFailure {
    BlockLimit,
    InsufficientSpace,
    /// Enough elements are free, but no single span can satisfy the request.
    Fragmented,
}

impl core::fmt::Display for AllocationFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::BlockLimit => "arena live-block limit reached",
            Self::InsufficientSpace => "arena has insufficient free elements",
            Self::Fragmented => "arena free space is fragmented",
        })
    }
}

impl core::error::Error for AllocationFailure {}

/// Fixed storage for up to `max_blocks` simultaneous nonempty allocations.
///
/// Capacity counts elements, not bytes or per-block stride. Allocation chooses the
/// lowest-address span that fits; release coalesces neighbours. Neither operation
/// grows a buffer or moves live data, and zero-length blocks consume no resources.
#[derive(Debug)]
pub struct VariableArena<T> {
    data: Vec<T>,
    free: Vec<Block>,
    max_blocks: u32,
    live_blocks: u32,
    free_elements: u32,
    reset_value: T,
}

impl<T: Copy + Default> VariableArena<T> {
    /// Reserves both backing buffers before the arena can enter a tick.
    ///
    /// The byte ceiling is a representation limit, not a host memory budget.
    /// Zero capacity or a zero block limit permits only empty allocations.
    pub fn try_with_capacity(capacity: u32, max_blocks: u32) -> Result<Self, ArenaBuildError> {
        // Every nonempty live block can separate two free spans. Even during release,
        // at most min(capacity, max_blocks) + 1 spans are needed (spec section 2.2a).
        let spans = if capacity == 0 {
            0
        } else {
            u64::from(capacity.min(max_blocks)) + 1
        };
        for (count, width) in [
            (u64::from(capacity), size_of::<T>() as u64),
            (spans, size_of::<Block>() as u64),
        ] {
            if count
                .checked_mul(width)
                .is_none_or(|bytes| bytes > i32::MAX as u64)
            {
                return Err(ArenaBuildError::CapacityTooLarge);
            }
        }

        let mut data = Vec::new();
        data.try_reserve_exact(capacity as usize)
            .map_err(ArenaBuildError::Reservation)?;
        let mut free = Vec::new();
        free.try_reserve_exact(spans as usize)
            .map_err(ArenaBuildError::Reservation)?;
        // Cache once so allocation resets with plain copies, never Default/Clone
        // calls supplied by the element type inside the hot loop.
        let reset_value = T::default();
        data.resize(capacity as usize, reset_value);
        if capacity != 0 {
            free.push(Block {
                offset: 0,
                len: capacity,
            });
        }
        Ok(Self {
            data,
            free,
            max_blocks,
            live_blocks: 0,
            free_elements: capacity,
            reset_value,
        })
    }

    /// Claims a contiguous block reset to `T::default()`.
    ///
    /// For nonempty requests, refusal precedence is block limit, total free space,
    /// then fragmentation. An empty request always succeeds, even in a full arena.
    pub fn alloc(&mut self, len: u32) -> Result<Block, AllocationFailure> {
        if len == 0 {
            return Ok(Block::EMPTY);
        }
        if self.live_blocks == self.max_blocks {
            return Err(AllocationFailure::BlockLimit);
        }
        if len > self.free_elements {
            return Err(AllocationFailure::InsufficientSpace);
        }
        let index = self
            .free
            .iter()
            .position(|span| span.len >= len)
            .ok_or(AllocationFailure::Fragmented)?;
        let block = Block {
            offset: self.free[index].offset,
            len,
        };
        if self.free[index].len == len {
            self.free.remove(index);
        } else {
            self.free[index].offset += len;
            self.free[index].len -= len;
        }
        for value in &mut self.data[block.range()] {
            *value = self.reset_value;
        }
        self.live_blocks += 1;
        self.free_elements -= len;
        Ok(block)
    }

    /// Releases an exact, live block returned by this arena. Empty blocks are a no-op.
    ///
    /// As with `Arena::free`, foreign, already-freed, or partial blocks violate the
    /// caller contract; serialized handles must be validated at their input boundary.
    pub fn free(&mut self, block: Block) {
        if block.is_empty() {
            return;
        }
        debug_assert!(self.live_blocks > 0, "no live block to free");
        debug_assert!(block.offset <= self.capacity(), "block starts out of range");
        debug_assert!(
            block.len <= self.capacity() - block.offset,
            "block ends out of range"
        );
        let end = block.offset + block.len;
        let index = self.free.partition_point(|span| span.offset < block.offset);
        let left = index.checked_sub(1).map(|i| self.free[i]);
        let right = self.free.get(index).copied();
        debug_assert!(
            left.is_none_or(|span| span.offset + span.len <= block.offset),
            "block overlaps free space on its left"
        );
        debug_assert!(
            right.is_none_or(|span| end <= span.offset),
            "block overlaps free space on its right"
        );

        match (
            left.is_some_and(|span| span.offset + span.len == block.offset),
            right.is_some_and(|span| end == span.offset),
        ) {
            (true, true) => {
                self.free[index - 1].len += block.len + self.free[index].len;
                self.free.remove(index);
            }
            (true, false) => self.free[index - 1].len += block.len,
            (false, true) => {
                self.free[index].offset = block.offset;
                self.free[index].len += block.len;
            }
            (false, false) => {
                debug_assert!(self.free.len() < self.free.capacity());
                self.free.insert(index, block);
            }
        }
        self.live_blocks -= 1;
        self.free_elements += block.len;
    }

    pub fn get(&self, block: Block) -> &[T] {
        &self.data[block.range()]
    }

    pub fn get_mut(&mut self, block: Block) -> &mut [T] {
        &mut self.data[block.range()]
    }

    pub fn capacity(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn live_blocks(&self) -> u32 {
        self.live_blocks
    }

    pub fn free_elements(&self) -> u32 {
        self.free_elements
    }

    pub fn largest_free_block(&self) -> u32 {
        self.free.iter().map(|span| span.len).max().unwrap_or(0)
    }

    /// Canonical, coalesced, address-ordered free spans for diagnostics/state hashing.
    pub fn free_spans(&self) -> &[Block] {
        &self.free
    }

    /// Re-establishes a saved placement in an arena with no live blocks.
    ///
    /// Coalesced free spans are exactly the gaps between live blocks, so the live set
    /// alone determines future first-fit placement (spec section 2.2a). Blocks are
    /// untrusted: out-of-range, overlapping, non-canonical empty, or too many blocks
    /// are refused and leave the arena unchanged. Contents are written afterwards.
    pub(crate) fn restore_placement(&mut self, blocks: &[Block]) -> Result<(), &'static str> {
        debug_assert_eq!(self.live_blocks, 0, "restore requires an empty arena");
        let capacity = self.capacity();
        let mut live: Vec<Block> = Vec::with_capacity(blocks.len());
        for &block in blocks {
            if block.len == 0 {
                if block != Block::EMPTY {
                    return Err("empty arena blocks must be canonical");
                }
                continue;
            }
            if block.offset > capacity || block.len > capacity - block.offset {
                return Err("arena block is out of range");
            }
            live.push(block);
        }
        if live.len() > self.max_blocks as usize {
            return Err("arena has more live blocks than its limit");
        }
        live.sort_unstable_by_key(|block| block.offset);
        let mut free = Vec::with_capacity(self.free.capacity());
        let mut cursor = 0;
        for block in &live {
            if block.offset < cursor {
                return Err("arena blocks overlap");
            }
            if block.offset > cursor {
                free.push(Block {
                    offset: cursor,
                    len: block.offset - cursor,
                });
            }
            cursor = block.offset + block.len;
        }
        if cursor < capacity {
            free.push(Block {
                offset: cursor,
                len: capacity - cursor,
            });
        }
        let used: u32 = live.iter().map(|block| block.len).sum();
        self.free.clear();
        self.free.extend_from_slice(&free);
        self.live_blocks = live.len() as u32;
        self.free_elements = capacity - used;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zero_length_requests_never_consume_a_block() {
        for (capacity, limit) in [(0, 0), (0, u32::MAX), (8, 0), (8, 1)] {
            let mut arena = VariableArena::<u32>::try_with_capacity(capacity, limit).unwrap();
            for _ in 0..10 {
                assert_eq!(arena.alloc(0), Ok(Block::EMPTY));
                arena.free(Block::EMPTY);
            }
            assert_eq!(arena.live_blocks(), 0);
            assert_eq!(arena.free_elements(), capacity);
            assert!(arena.get(Block::EMPTY).is_empty());
        }
        let mut arena = VariableArena::<u32>::try_with_capacity(8, 0).unwrap();
        assert_eq!(arena.alloc(1), Err(AllocationFailure::BlockLimit));
        let mut arena = VariableArena::<u32>::try_with_capacity(0, 4).unwrap();
        assert_eq!(arena.alloc(1), Err(AllocationFailure::InsufficientSpace));
    }

    #[test]
    fn chooses_first_fitting_address_not_the_tightest_fit() {
        let mut arena = VariableArena::<u32>::try_with_capacity(20, 6).unwrap();
        let first = arena.alloc(6).unwrap();
        let middle = arena.alloc(4).unwrap();
        let later = arena.alloc(3).unwrap();
        let last = arena.alloc(7).unwrap();
        arena.free(first);
        arena.free(later);
        let chosen = arena.alloc(3).unwrap();
        assert_eq!(chosen.offset(), 0);
        assert_eq!(arena.get(middle).len(), 4);
        assert_eq!(arena.get(last).len(), 7);
    }

    #[test]
    fn coalesces_no_neighbour_left_right_and_both() {
        let mut arena = VariableArena::<u32>::try_with_capacity(16, 4).unwrap();
        let a = arena.alloc(2).unwrap();
        let b = arena.alloc(3).unwrap();
        let c = arena.alloc(4).unwrap();
        let d = arena.alloc(5).unwrap();
        arena.free(a);
        assert_eq!(arena.free_spans(), &[a, Block { offset: 14, len: 2 }]);
        arena.free(b);
        assert_eq!(arena.free_spans()[0], Block { offset: 0, len: 5 });
        arena.free(d);
        assert_eq!(arena.free_spans()[1], Block { offset: 9, len: 7 });
        arena.free(c);
        assert_eq!(arena.free_spans(), &[Block { offset: 0, len: 16 }]);
        assert_eq!(arena.free_elements(), 16);
        assert_eq!(arena.live_blocks(), 0);
        assert_eq!(arena.alloc(16), Ok(Block { offset: 0, len: 16 }));
    }

    #[test]
    fn reuse_resets_only_the_new_tenants_elements() {
        let mut arena = VariableArena::<u32>::try_with_capacity(12, 3).unwrap();
        let a = arena.alloc(5).unwrap();
        let b = arena.alloc(7).unwrap();
        arena.get_mut(a).fill(42);
        arena.get_mut(b).fill(99);
        arena.free(a);
        let replacement = arena.alloc(3).unwrap();
        assert_eq!(replacement.offset(), a.offset());
        assert_eq!(arena.get(replacement), &[0; 3]);
        assert_eq!(arena.get(b), &[99; 7]);
        arena.free(Block::EMPTY);
        assert_eq!(arena.live_blocks(), 2);
    }

    #[test]
    fn refusals_distinguish_block_capacity_and_fragmentation_without_changes() {
        let mut arena = VariableArena::<u32>::try_with_capacity(12, 4).unwrap();
        let blocks: Vec<_> = (0..4).map(|_| arena.alloc(3).unwrap()).collect();
        assert_eq!(arena.alloc(1), Err(AllocationFailure::BlockLimit));
        assert_eq!(arena.alloc(0), Ok(Block::EMPTY));
        arena.free(blocks[0]);
        arena.free(blocks[2]);
        let before = (arena.data.clone(), arena.free.clone(), arena.live_blocks());
        assert_eq!(arena.alloc(4), Err(AllocationFailure::Fragmented));
        assert_eq!(arena.alloc(7), Err(AllocationFailure::InsufficientSpace));
        assert_eq!(
            arena.alloc(u32::MAX),
            Err(AllocationFailure::InsufficientSpace)
        );
        assert_eq!(arena.data, before.0);
        assert_eq!(arena.free, before.1);
        assert_eq!(arena.live_blocks(), before.2);
        assert_eq!(arena.free_elements(), 6);
        assert_eq!(arena.largest_free_block(), 3);
    }

    #[test]
    fn a_block_limit_can_refuse_even_with_contiguous_space() {
        let mut arena = VariableArena::<u32>::try_with_capacity(12, 1).unwrap();
        let block = arena.alloc(1).unwrap();
        let before = arena.free.clone();
        assert_eq!(arena.alloc(2), Err(AllocationFailure::BlockLimit));
        assert_eq!(arena.free_spans(), before);
        assert_eq!(arena.free_elements(), 11);
        arena.free(block);
        assert!(arena.alloc(12).is_ok());
    }

    #[test]
    fn rejects_unrepresentable_data_or_metadata_before_reservation() {
        assert!(matches!(
            VariableArena::<u64>::try_with_capacity(u32::MAX, 1),
            Err(ArenaBuildError::CapacityTooLarge)
        ));
        assert!(matches!(
            VariableArena::<()>::try_with_capacity(u32::MAX, u32::MAX),
            Err(ArenaBuildError::CapacityTooLarge)
        ));
    }

    fn reference_spans(occupied: &[bool]) -> Vec<Block> {
        let mut free = Vec::new();
        let mut cursor = 0;
        while cursor < occupied.len() {
            if occupied[cursor] {
                cursor += 1;
                continue;
            }
            let offset = cursor;
            while cursor < occupied.len() && !occupied[cursor] {
                cursor += 1;
            }
            free.push(Block {
                offset: offset as u32,
                len: (cursor - offset) as u32,
            });
        }
        free
    }

    #[test]
    fn restored_placement_refuses_invalid_saved_blocks_without_changing_the_arena() {
        let mut arena = VariableArena::<u32>::try_with_capacity(10, 2).unwrap();
        for blocks in [
            &[Block { offset: 8, len: 3 }][..],
            &[Block { offset: 0, len: 4 }, Block { offset: 3, len: 2 }],
            &[
                Block { offset: 0, len: 1 },
                Block { offset: 2, len: 1 },
                Block { offset: 4, len: 1 },
            ],
            &[Block { offset: 5, len: 0 }],
        ] {
            assert!(
                arena.restore_placement(blocks).is_err(),
                "accepted {blocks:?}"
            );
            assert_eq!(arena.free_spans(), &[Block { offset: 0, len: 10 }]);
            assert_eq!((arena.live_blocks(), arena.free_elements()), (0, 10));
        }
    }

    proptest! {
        /// Restoring only the surviving live blocks reproduces the allocator's own
        /// free spans and therefore every later first-fit decision.
        #[test]
        fn restored_placement_matches_the_allocation_history_it_came_from(
            capacity in 0u32..80,
            max_blocks in 0u32..16,
            operations in prop::collection::vec((any::<bool>(), 0u32..100), 0..300),
            next in prop::collection::vec(0u32..40, 0..8),
        ) {
            let mut arena = VariableArena::<u32>::try_with_capacity(capacity, max_blocks).unwrap();
            let mut live: Vec<Block> = Vec::new();
            for (allocate, value) in operations {
                if allocate {
                    if let Ok(block) = arena.alloc(value % 32) {
                        live.push(block);
                    }
                } else if !live.is_empty() {
                    let block = live.swap_remove(value as usize % live.len());
                    arena.free(block);
                }
            }
            let mut restored = VariableArena::<u32>::try_with_capacity(capacity, max_blocks).unwrap();
            restored.restore_placement(&live).unwrap();
            prop_assert_eq!(restored.free_spans(), arena.free_spans());
            prop_assert_eq!(restored.live_blocks(), arena.live_blocks());
            prop_assert_eq!(restored.free_elements(), arena.free_elements());
            for len in next {
                prop_assert_eq!(restored.alloc(len), arena.alloc(len));
            }
        }

        #[test]
        fn matches_an_elementwise_reference_after_every_operation(
            capacity in 0u32..80,
            max_blocks in 0u32..16,
            operations in prop::collection::vec((any::<bool>(), 0u32..100), 0..300),
        ) {
            let mut arena = VariableArena::<u32>::try_with_capacity(capacity, max_blocks).unwrap();
            let metadata_capacity = arena.free.capacity();
            let data_capacity = arena.data.capacity();
            let mut occupied = vec![false; capacity as usize];
            let mut live: Vec<(Block, u32)> = Vec::new();
            for (step, (allocate, value)) in operations.into_iter().enumerate() {
                if allocate {
                    let len = value as usize;
                    let free_count = occupied.iter().filter(|&&used| !used).count();
                    let expected = if len == 0 {
                        Ok(Block::EMPTY)
                    } else if live.len() == max_blocks as usize {
                        Err(AllocationFailure::BlockLimit)
                    } else if len > free_count {
                        Err(AllocationFailure::InsufficientSpace)
                    } else {
                        occupied.windows(len).position(|cells| cells.iter().all(|&used| !used))
                            .map(|offset| Block { offset: offset as u32, len: value })
                            .ok_or(AllocationFailure::Fragmented)
                    };
                    let result = arena.alloc(value);
                    prop_assert_eq!(result, expected);
                    if let Ok(block) = result {
                        prop_assert!(arena.get(block).iter().all(|&value| value == 0));
                        if !block.is_empty() {
                            let mark = step as u32 + 1;
                            occupied[block.range()].fill(true);
                            arena.get_mut(block).fill(mark);
                            live.push((block, mark));
                        }
                    }
                } else if !live.is_empty() {
                    let (block, _) = live.swap_remove(value as usize % live.len());
                    occupied[block.range()].fill(false);
                    arena.free(block);
                } else {
                    arena.free(Block::EMPTY);
                }
                prop_assert_eq!(arena.live_blocks(), live.len() as u32);
                prop_assert_eq!(arena.free_elements(), occupied.iter().filter(|&&used| !used).count() as u32);
                let spans = reference_spans(&occupied);
                prop_assert_eq!(arena.largest_free_block(), spans.iter().map(|s| s.len).max().unwrap_or(0));
                prop_assert_eq!(arena.free_spans(), spans.as_slice());
                prop_assert_eq!(arena.free.capacity(), metadata_capacity);
                prop_assert_eq!(arena.data.capacity(), data_capacity);
                for &(block, mark) in &live {
                    prop_assert!(arena.get(block).iter().all(|&value| value == mark));
                }
            }
        }
    }
}
