//! Bounded chronological capture of species lifecycle observations.
//!
//! Shells own recorders and persistence; World only emits optional callbacks.
//! Overflow produces explicit gaps, never changes to ecology or authoritative state.

use std::collections::TryReserveError;

use crate::ids::{BirthId, SpeciesId};

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

#[cfg(test)]
mod tests {
    //! Recorder ordering, overflow, and sequence-boundary regressions.
    //!
    //! Stream properties supplement hand-worked cases without constructing a World.

    use super::*;
    use proptest::prelude::*;

    fn event(tick: u64) -> Event {
        Event {
            tick,
            kind: EventKind::SpeciesExtinct {
                species_id: SpeciesId::new(0),
            },
        }
    }

    fn drain(recorder: &mut Recorder) -> Vec<Record> {
        std::iter::from_fn(|| recorder.pop()).collect()
    }

    #[test]
    fn queued_prefix_is_retained_and_later_gaps_cannot_be_overtaken() {
        let mut recorder = Recorder::try_new(2).unwrap();
        assert_eq!(recorder.record(event(0)), Ok(Capture::Recorded));
        assert_eq!(recorder.record(event(1)), Ok(Capture::Recorded));
        assert_eq!(recorder.record(event(2)), Ok(Capture::Dropped));
        assert_eq!(recorder.record(event(3)), Ok(Capture::Dropped));
        assert_eq!(recorder.pending_len(), 3);
        assert_eq!(
            recorder.pop(),
            Some(Record::Event {
                sequence: 0,
                event: event(0)
            })
        );
        assert_eq!(recorder.record(event(4)), Ok(Capture::Dropped));
        assert_eq!(
            drain(&mut recorder),
            [
                Record::Event {
                    sequence: 1,
                    event: event(1)
                },
                Record::Gap {
                    first_sequence: 2,
                    last_sequence: 3
                },
                Record::Gap {
                    first_sequence: 4,
                    last_sequence: 4
                },
            ]
        );
        assert_eq!(recorder.dropped_events(), 3);
        assert_eq!(recorder.next_sequence(), 5);
        assert_eq!(recorder.record(event(5)), Ok(Capture::Recorded));
        assert_eq!(
            recorder.pop(),
            Some(Record::Event {
                sequence: 5,
                event: event(5)
            })
        );
        assert_eq!(recorder.pop(), None);
    }

    #[test]
    fn a_final_gap_drains_without_any_later_capture_or_free_slot() {
        let mut recorder = Recorder::try_new(1).unwrap();
        for tick in 0..100 {
            recorder.record(event(tick)).unwrap();
        }
        assert_eq!(
            drain(&mut recorder),
            [
                Record::Event {
                    sequence: 0,
                    event: event(0)
                },
                Record::Gap {
                    first_sequence: 1,
                    last_sequence: 99
                },
            ]
        );
        assert_eq!(recorder.dropped_events(), 99);
        assert_eq!(recorder.pending_len(), 0);
    }

    #[test]
    fn exhaustion_is_explicit_nonwrapping_and_keeps_queued_data_drainable() {
        let mut recorder = Recorder::try_new(1).unwrap();
        recorder.next_sequence = u64::MAX - 1;
        assert_eq!(recorder.record(event(0)), Ok(Capture::Recorded));
        assert_eq!(recorder.record(event(1)), Err(SequenceExhausted));
        assert_eq!(recorder.record(event(2)), Err(SequenceExhausted));
        assert!(recorder.sequence_exhausted());
        assert_eq!(recorder.next_sequence(), u64::MAX);
        assert_eq!(
            recorder.pop(),
            Some(Record::Event {
                sequence: u64::MAX - 1,
                event: event(0),
            })
        );
        assert_eq!(recorder.pop(), None);
    }

    #[test]
    fn invalid_capacities_fail_before_reservation() {
        assert!(matches!(
            Recorder::try_new(0),
            Err(BuildError::ZeroCapacity)
        ));
        assert!(matches!(
            Recorder::try_new(u32::MAX),
            Err(BuildError::CapacityTooLarge)
        ));
    }

    #[test]
    fn recorder_preserves_two_parent_representation_and_unavailable_identity() {
        let event = Event {
            tick: 7,
            kind: EventKind::SpeciesOrigin {
                species_id: SpeciesId::new(2),
                founder_birth_id: BirthId::NULL,
                parent_a: Parent::Observed {
                    birth_id: BirthId::new(0),
                    species_id: Some(SpeciesId::new(0)),
                },
                parent_b: Parent::Observed {
                    birth_id: BirthId::new(1),
                    species_id: None,
                },
            },
        };
        let mut recorder = Recorder::try_new(1).unwrap();
        recorder.record(event).unwrap();
        assert_eq!(recorder.pop(), Some(Record::Event { sequence: 0, event }));
    }

    #[test]
    fn live_parent_observation_survives_unavailable_birth_identity() {
        use crate::genome::{Gene, fixtures::tiny};
        use crate::{AgentId, SimParams, SpawnSpec, World};
        use glam::Vec3;

        let mut params = SimParams::default();
        params.world.max_agents = 2;
        params.plants.max_plants = 0;
        params.species.capacity = 2;
        params.species.threshold = 1e-12;
        let mut world = World::new(42, params).unwrap();
        world.next_birth = u64::MAX;
        let mut genes = tiny();
        let mut spec = SpawnSpec {
            position: Vec3::ZERO,
            yaw: 0.0,
            energy: 0.0,
            size: 3.0,
            signature: Vec3::ONE,
            parent_a: AgentId::NULL,
        };
        let parent = world.spawn(&spec, &genes).unwrap();
        spec.parent_a = parent;
        for gene in &mut genes {
            if let Gene::Connection(connection) = gene {
                connection.weight += 1.0;
            }
        }
        let mut events = Vec::new();
        world
            .spawn_with_history_observer(&spec, &genes, |_| {}, |event| events.push(event))
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, EventKind::SpeciesOrigin {
            founder_birth_id: BirthId::NULL,
            parent_a: Parent::Observed { birth_id: BirthId::NULL, species_id: Some(id) },
            parent_b: Parent::Absent, ..
        } if id == SpeciesId::new(0)));
    }

    proptest! {
        #[test]
        fn arbitrary_capture_and_partial_drains_cover_every_sequence_exactly_once(
            capacity in 1u32..9,
            operations in prop::collection::vec(any::<bool>(), 1..1000),
        ) {
            let mut recorder = Recorder::try_new(capacity).unwrap();
            let mut retained = Vec::new();
            let mut output = Vec::new();
            for capture in operations {
                if capture {
                    let tick = retained.len() as u64;
                    retained.push(recorder.record(event(tick)).unwrap() == Capture::Recorded);
                } else if let Some(record) = recorder.pop() {
                    output.push(record);
                }
                prop_assert!(recorder.pending_len() <= capacity as usize + 1);
            }
            output.extend(drain(&mut recorder));
            let mut next = 0;
            let mut lost = 0;
            for record in output {
                match record {
                    Record::Event { sequence, event: captured } => {
                        prop_assert_eq!(sequence, next);
                        prop_assert!(retained[next as usize]);
                        prop_assert_eq!(captured, event(next));
                        next += 1;
                    }
                    Record::Gap { first_sequence, last_sequence } => {
                        prop_assert_eq!(first_sequence, next);
                        prop_assert!(last_sequence >= first_sequence);
                        for sequence in first_sequence..=last_sequence {
                            prop_assert!(!retained[sequence as usize]);
                        }
                        lost += last_sequence - first_sequence + 1;
                        next = last_sequence + 1;
                    }
                }
            }
            prop_assert_eq!(next, retained.len() as u64);
            prop_assert_eq!(next, recorder.next_sequence());
            prop_assert_eq!(lost, recorder.dropped_events());
            prop_assert_eq!(recorder.pending_len(), 0);
        }
    }
}
