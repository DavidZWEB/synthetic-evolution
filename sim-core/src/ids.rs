//! Newtype identifiers for the things that are all `u32` underneath.
//!
//! The distinction that matters: an [`InnovationId`] is a *genome-level identity*,
//! stable across a lineage and comparable between two genomes; a [`NeuronId`] is a
//! *slot in one compiled brain's activation array*, meaningful only inside that
//! agent. Passing one where the other is wanted produces a simulation that runs and
//! is subtly wrong, so they are different types and the compiler checks it.
//!
//! Deliberately not here: any lookup tables or arenas that hold these. Types only.

use serde::{Deserialize, Serialize};

/// Sentinel for "no such entity". `parentB` is always this in V1 (spec §3.4).
pub const NULL_ID: u32 = u32::MAX;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
        #[cfg_attr(test, derive(ts_rs::TS))]
        #[serde(transparent)]
        #[cfg_attr(test, ts(export, export_to = "../../web/src/generated/"))]
        pub struct $name(u32);

        impl $name {
            /// The "no such entity" value.
            pub const NULL: Self = Self(NULL_ID);

            #[inline]
            pub const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// Use for array indexing. Meaningless on [`Self::NULL`], so check first.
            #[inline]
            pub const fn index(self) -> usize {
                self.0 as usize
            }

            #[inline]
            pub const fn raw(self) -> u32 {
                self.0
            }

            #[inline]
            pub const fn is_null(self) -> bool {
                self.0 == NULL_ID
            }
        }

        impl Default for $name {
            /// [`Self::NULL`], deliberately. A default id names nothing, so a gene
            /// with a field left unset fails genome validation as a dangling
            /// reference rather than silently binding to neuron 0.
            #[inline]
            fn default() -> Self {
                Self::NULL
            }
        }

        impl From<usize> for $name {
            #[inline]
            fn from(i: usize) -> Self {
                debug_assert!(i < NULL_ID as usize, "index would collide with NULL_ID");
                Self(i as u32)
            }
        }
    };
}

define_id! {
    /// Slot in the world's fixed-capacity agent pool. Recycled through the free list,
    /// so it identifies a *slot*, not a lineage — a dead agent's id will be reused.
    AgentId
}

define_id! {
    /// Slot in one agent's activation array within the brain arena. Local to that
    /// agent; comparing two agents' `NeuronId`s is meaningless.
    NeuronId
}

define_id! {
    /// Monotonic genome-level identity, NEAT-style. Shared ancestry between two
    /// genomes is visible by matching ids, which is what makes crossover and genetic
    /// distance possible (spec §3.1). The counter is a field on `World`, never a
    /// `static` — a process must be able to hold several worlds (spec §7.2).
    InnovationId
}

define_id! {
    /// Slot in the parts arena. Every agent has exactly one part at its origin in V1;
    /// the indirection exists so Phase 5 morphology is an unlock, not a rewrite
    /// (spec §9.1).
    PartId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_is_distinguishable_from_a_real_id() {
        assert!(AgentId::NULL.is_null());
        assert!(!AgentId::new(0).is_null());
        assert!(!AgentId::new(NULL_ID - 1).is_null());
    }

    #[test]
    fn round_trips_through_raw_and_index() {
        let id = InnovationId::new(4_242);
        assert_eq!(id.raw(), 4_242);
        assert_eq!(id.index(), 4_242);
        assert_eq!(InnovationId::from(4_242usize), id);
    }

    #[test]
    fn ids_order_by_raw_value() {
        let mut ids = [
            InnovationId::new(3),
            InnovationId::new(1),
            InnovationId::new(2),
        ];
        ids.sort();
        assert_eq!(
            ids,
            [
                InnovationId::new(1),
                InnovationId::new(2),
                InnovationId::new(3)
            ]
        );
    }
}
