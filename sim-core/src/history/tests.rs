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
