//! Bounded chronological capture of species lifecycle observations.
//!
//! Shells own recorders and persistence; World only emits optional callbacks.
//! Overflow produces explicit gaps, never changes to ecology or authoritative state.

use std::collections::TryReserveError;

use crate::ids::{BirthId, SpeciesId};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parent {
    Absent,
    /// A parent was declared, but its live identity could not be observed at admission.
    Unavailable,
    Observed {
        birth_id: BirthId,
        /// None means the observed parent was unclassified, not a founder.
        species_id: Option<SpeciesId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    SpeciesOrigin {
        species_id: SpeciesId,
        founder_birth_id: BirthId,
        parent_a: Parent,
        parent_b: Parent,
    },
    SpeciesExtinct {
        species_id: SpeciesId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// The simulation tick being processed, including tick zero during seeding.
    pub tick: u64,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Event {
        sequence: u64,
        event: Event,
    },
    /// Inclusive sequence range of events dropped under capture pressure.
    Gap {
        first_sequence: u64,
        last_sequence: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    Recorded,
    Dropped,
}

#[derive(Debug)]
pub enum BuildError {
    ZeroCapacity,
    CapacityTooLarge,
    Reservation(TryReserveError),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("history capacity must be positive"),
            Self::CapacityTooLarge => {
                f.write_str("history buffer exceeds the portable byte ceiling")
            }
            Self::Reservation(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Reservation(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceExhausted;

impl core::fmt::Display for SequenceExhausted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("history sequence space exhausted; capture is incomplete")
    }
}

impl core::error::Error for SequenceExhausted {}

/// A preallocated FIFO plus one pending, coalesced gap outside its occupied slots.
///
/// Records have sequence numbers 0 through u64::MAX-1. An exhausted recorder rejects
/// further capture explicitly and remains drainable; it never wraps or edits World.
#[derive(Debug)]
pub struct Recorder {
    slots: Vec<Option<Record>>,
    head: usize,
    len: usize,
    pending_gap: Option<(u64, u64)>,
    next_sequence: u64,
    dropped: u64,
    exhausted: bool,
}

impl Recorder {
    pub fn try_new(capacity: u32) -> Result<Self, BuildError> {
        if capacity == 0 {
            return Err(BuildError::ZeroCapacity);
        }

        if u64::from(capacity)
            .checked_mul(size_of::<Option<Record>>() as u64)
            .is_none_or(|bytes| bytes > i32::MAX as u64)
        {
            return Err(BuildError::CapacityTooLarge);
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity as usize)
            .map_err(BuildError::Reservation)?;
        slots.resize(capacity as usize, None);
        Ok(Self {
            slots,
            head: 0,
            len: 0,
            pending_gap: None,
            next_sequence: 0,
            dropped: 0,
            exhausted: false,
        })
    }

    /// Capture callback events in their source order without allocating.
    ///
    /// A pending gap must precede any newer event. It can consume the last free slot,
    /// causing that newer event to start another gap rather than overtake lost history.
    pub fn record(&mut self, event: Event) -> Result<Capture, SequenceExhausted> {
        let sequence = self.next_sequence;
        let Some(next) = sequence.checked_add(1) else {
            self.exhausted = true;
            return Err(SequenceExhausted);
        };
        self.next_sequence = next;
        if let Some((first_sequence, last_sequence)) = self.pending_gap {
            if self.len == self.slots.len() {
                self.drop_event(sequence);
                return Ok(Capture::Dropped);
            }
            self.pending_gap = None;
            self.push(Record::Gap {
                first_sequence,
                last_sequence,
            });
        }
        if self.len == self.slots.len() {
            self.drop_event(sequence);
            return Ok(Capture::Dropped);
        }
        self.push(Record::Event { sequence, event });
        Ok(Capture::Recorded)
    }

    /// Drain in sequence order, including any final gap even when capture has stopped.
    pub fn pop(&mut self) -> Option<Record> {
        if self.len == 0 {
            return self
                .pending_gap
                .take()
                .map(|(first_sequence, last_sequence)| Record::Gap {
                    first_sequence,
                    last_sequence,
                });
        }
        let record = self.slots[self.head].take().expect("occupied history slot");
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
        Some(record)
    }

    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }
    pub fn pending_len(&self) -> usize {
        self.len + usize::from(self.pending_gap.is_some())
    }
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
    pub fn dropped_events(&self) -> u64 {
        self.dropped
    }
    pub fn sequence_exhausted(&self) -> bool {
        self.exhausted
    }

    fn push(&mut self, record: Record) {
        debug_assert!(self.len < self.slots.len());
        let index = (self.head + self.len) % self.slots.len();
        debug_assert!(self.slots[index].is_none());
        self.slots[index] = Some(record);
        self.len += 1;
    }

    fn drop_event(&mut self, sequence: u64) {
        // Dropped events are a subset of issued sequences, so this cannot overflow.
        self.dropped += 1;
        match &mut self.pending_gap {
            Some((_, last)) => {
                debug_assert_eq!(*last + 1, sequence);
                *last = sequence;
            }
            None => self.pending_gap = Some((sequence, sequence)),
        }
    }
}
