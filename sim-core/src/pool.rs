//! Fixed-capacity slot allocation with a free list.
//!
//! Agents are slots in parallel arrays, not objects, so "spawning" means claiming an
//! index and "dying" means returning it. Capacity is fixed at construction and never
//! grows: the tick must not allocate (spec §2.2a), and in the browser a
//! grown WASM heap detaches every JS view over the snapshot (spec §7.3).
//!
//! Deliberately not here: what lives in those slots. This module knows only which
//! indices are in use.

use crate::ids::AgentId;

/// Which slots are live, and which are free to claim.
///
/// Owns the `alive` flags rather than duplicating them alongside, because two sources
/// of truth for liveness is the kind of disagreement that shows up as an agent
/// rendering after it dies.
#[derive(Clone, Debug)]
pub struct SlotPool {
    /// One byte per slot, matching the `Uint8Array` the render snapshot exports
    /// (spec §2.2b).
    alive: Vec<u8>,
    /// Stack of free indices. LIFO, so allocation is a pop rather than a scan — and
    /// deterministic, because deaths resolve in agent-index order (spec §2.4).
    free: Vec<u32>,
    live_count: u32,
}

impl SlotPool {
    pub fn with_capacity(capacity: u32) -> Self {
        let cap = capacity as usize;
        let mut free = Vec::with_capacity(cap);
        // Descending, so the first allocations come off the front of the arrays and
        // a small population stays cache-local rather than scattered across the pool.
        for i in (0..capacity).rev() {
            free.push(i);
        }
        Self {
            alive: vec![0u8; cap],
            free,
            live_count: 0,
        }
    }

    /// Claims a free slot, or `None` when the pool is full.
    ///
    /// A full pool is a normal condition — the population hit its ceiling — not an
    /// error. Callers drop the birth.
    pub fn alloc(&mut self) -> Option<AgentId> {
        let index = self.free.pop()?;
        debug_assert_eq!(
            self.alive[index as usize], 0,
            "free list handed out a live slot"
        );
        self.alive[index as usize] = 1;
        self.live_count += 1;
        Some(AgentId::new(index))
    }

    /// Returns a slot to the free list. Freeing a dead slot, or [`AgentId::NULL`], is
    /// a no-op, so a double-death cannot corrupt the free list into handing the same
    /// index out twice.
    ///
    /// Checked rather than indexed: `AgentId::NULL` is a value of this method's own
    /// parameter type and is what `parent_a`/`parent_b` hold, so it has to be a no-op
    /// rather than an out-of-range panic.
    pub fn free(&mut self, id: AgentId) -> bool {
        let Some(flag) = self.alive.get_mut(id.index()) else {
            return false;
        };
        if *flag == 0 {
            return false;
        }
        *flag = 0;
        self.live_count -= 1;
        self.free.push(id.raw());
        true
    }

    #[inline]
    pub fn is_alive(&self, id: AgentId) -> bool {
        self.alive.get(id.index()).is_some_and(|&a| a != 0)
    }

    #[inline]
    pub fn live_count(&self) -> u32 {
        self.live_count
    }

    #[inline]
    pub fn capacity(&self) -> u32 {
        self.alive.len() as u32
    }

    /// The raw flags, for the render snapshot.
    #[inline]
    pub fn alive_flags(&self) -> &[u8] {
        &self.alive
    }

    /// Live slots in ascending index order.
    ///
    /// Ascending on purpose: every system that touches shared state must resolve in
    /// agent-index order, or behaviour becomes an artifact of pool layout (spec §2.4).
    pub fn iter_live(&self) -> impl Iterator<Item = AgentId> + '_ {
        self.alive
            .iter()
            .enumerate()
            .filter(|&(_, &a)| a != 0)
            .map(|(i, _)| AgentId::new(i as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_until_full_then_reports_full() {
        let mut pool = SlotPool::with_capacity(3);
        let ids: Vec<_> = (0..3)
            .map(|_| pool.alloc().expect("capacity available"))
            .collect();
        assert_eq!(pool.live_count(), 3);
        assert!(pool.alloc().is_none(), "a full pool must refuse, not grow");
        for id in ids {
            assert!(pool.is_alive(id));
        }
    }

    #[test]
    fn first_allocations_are_the_low_indices() {
        // Keeps a small population contiguous at the front of the SoA arrays.
        let mut pool = SlotPool::with_capacity(100);
        assert_eq!(pool.alloc().unwrap().raw(), 0);
        assert_eq!(pool.alloc().unwrap().raw(), 1);
        assert_eq!(pool.alloc().unwrap().raw(), 2);
    }

    #[test]
    fn freed_slots_are_reused() {
        let mut pool = SlotPool::with_capacity(2);
        let a = pool.alloc().unwrap();
        let b = pool.alloc().unwrap();
        assert!(pool.free(a));
        assert!(!pool.is_alive(a));
        assert_eq!(pool.live_count(), 1);
        assert_eq!(
            pool.alloc(),
            Some(a),
            "the freed slot should come straight back"
        );
        assert!(pool.is_alive(b));
    }

    #[test]
    fn double_free_does_not_duplicate_a_slot() {
        // Otherwise the same index is handed to two agents and they share a body.
        let mut pool = SlotPool::with_capacity(4);
        let a = pool.alloc().unwrap();
        assert!(pool.free(a));
        assert!(!pool.free(a));
        assert_eq!(pool.live_count(), 0);

        let mut seen = Vec::new();
        while let Some(id) = pool.alloc() {
            assert!(!seen.contains(&id), "slot {id:?} handed out twice");
            seen.push(id);
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn freeing_the_null_sentinel_is_a_no_op() {
        let mut pool = SlotPool::with_capacity(4);
        let a = pool.alloc().unwrap();
        assert!(
            !pool.free(AgentId::NULL),
            "NULL must not be treated as a slot"
        );
        assert_eq!(pool.live_count(), 1);
        assert!(pool.is_alive(a));
    }

    #[test]
    fn iteration_is_ascending_and_live_only() {
        let mut pool = SlotPool::with_capacity(8);
        let ids: Vec<_> = (0..8).map(|_| pool.alloc().unwrap()).collect();
        pool.free(ids[1]);
        pool.free(ids[5]);
        pool.free(ids[6]);
        let live: Vec<u32> = pool.iter_live().map(|id| id.raw()).collect();
        assert_eq!(live, vec![0, 2, 3, 4, 7]);
    }

    #[test]
    fn live_count_matches_iteration_under_churn() {
        let mut pool = SlotPool::with_capacity(64);
        let mut held: Vec<AgentId> = Vec::new();
        for round in 0..200 {
            for _ in 0..7 {
                if let Some(id) = pool.alloc() {
                    held.push(id);
                }
            }
            if round % 2 == 0 {
                for _ in 0..5 {
                    if let Some(id) = held.pop() {
                        pool.free(id);
                    }
                }
            }
            assert_eq!(pool.live_count() as usize, pool.iter_live().count());
        }
    }
}
