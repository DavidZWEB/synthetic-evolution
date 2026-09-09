//! Optional synchronous observations of species lifecycle transitions.
//!
//! Counts belong to observers, not authoritative classification or ecological state.
//! No history buffer, I/O, or per-transition allocation is introduced here.

use serde::{Deserialize, Serialize};

use super::Unclassified;
use crate::ids::SpeciesId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeciesEvent {
    Created(SpeciesId),
    Extinct(SpeciesId),
    Unclassified(Unclassified),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeciesEventCounts {
    pub created: u64,
    pub extinct: u64,
    pub unclassified_capacity: u64,
    pub unclassified_id_exhausted: u64,
    pub unclassified_genome_too_large: u64,
    pub unclassified_member_count_exhausted: u64,
    pub unclassified_storage: u64,
}

impl SpeciesEventCounts {
    pub fn record(&mut self, event: SpeciesEvent) {
        let counter = match event {
            SpeciesEvent::Created(_) => &mut self.created,
            SpeciesEvent::Extinct(_) => &mut self.extinct,
            SpeciesEvent::Unclassified(reason) => match reason {
                Unclassified::Capacity => &mut self.unclassified_capacity,
                Unclassified::IdExhausted => &mut self.unclassified_id_exhausted,
                Unclassified::GenomeTooLarge => &mut self.unclassified_genome_too_large,
                Unclassified::MemberCountExhausted => &mut self.unclassified_member_count_exhausted,
                Unclassified::Storage(_) => &mut self.unclassified_storage,
            },
        };
        *counter = counter.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::AllocationFailure;

    #[test]
    fn each_event_updates_only_its_own_counter_and_saturates() {
        let events = [
            SpeciesEvent::Created(SpeciesId::new(0)),
            SpeciesEvent::Extinct(SpeciesId::new(0)),
            SpeciesEvent::Unclassified(Unclassified::Capacity),
            SpeciesEvent::Unclassified(Unclassified::IdExhausted),
            SpeciesEvent::Unclassified(Unclassified::GenomeTooLarge),
            SpeciesEvent::Unclassified(Unclassified::MemberCountExhausted),
            SpeciesEvent::Unclassified(Unclassified::Storage(AllocationFailure::InsufficientSpace)),
        ];
        for (selected, event) in events.into_iter().enumerate() {
            let mut counts = SpeciesEventCounts::default();
            counts.record(event);
            let actual = [
                counts.created,
                counts.extinct,
                counts.unclassified_capacity,
                counts.unclassified_id_exhausted,
                counts.unclassified_genome_too_large,
                counts.unclassified_member_count_exhausted,
                counts.unclassified_storage,
            ];
            for (index, count) in actual.into_iter().enumerate() {
                assert_eq!(count, u64::from(index == selected));
            }
        }
        let mut counts = SpeciesEventCounts {
            created: u64::MAX,
            ..Default::default()
        };
        counts.record(SpeciesEvent::Created(SpeciesId::new(1)));
        assert_eq!(counts.created, u64::MAX);
    }
}
