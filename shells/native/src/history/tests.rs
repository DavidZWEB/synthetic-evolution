//! Archive boundary regressions, independent of the simulator's ecological rules.

use std::io::{self, BufReader, Cursor, Write};

use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;
use sim_core::history::{Event, EventKind, Parent};
use sim_core::ids::{BirthId, SpeciesId};
use sim_core::params::SimParams;

use super::*;
use crate::history_reader::parse;
use crate::history_wire::{ArchiveRecord, Decimal};

fn header() -> RunHeader {
    RunHeader {
        schema_version: crate::metrics::SCHEMA_VERSION,
        sim_version: env!("CARGO_PKG_VERSION").to_owned(),
        source_revision: "test-revision".to_owned(),
        phase: 2,
        seed: u64::MAX.to_string(),
        ticks: 100,
        founders: 1,
        sample_every: 10,
        params: SimParams::default(),
        control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
    }
}

fn hashes() -> StateHashes {
    StateHashes {
        evolving: "0123456789abcdef".into(),
        random_control: "fedcba9876543210".into(),
    }
}

fn origin(tick: u64, id: u32) -> Event {
    Event {
        tick,
        kind: EventKind::SpeciesOrigin {
            species_id: SpeciesId::new(id),
            founder_birth_id: BirthId::new(u64::from(id)),
            parent_a: Parent::Absent,
            parent_b: Parent::Absent,
        },
    }
}

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut capture = Capture::new(4).unwrap();
    for recorder in &mut capture.recorders {
        record(recorder, origin(0, 0));
        record(
            recorder,
            Event {
                tick: 9,
                kind: EventKind::SpeciesExtinct {
                    species_id: SpeciesId::new(0),
                },
            },
        );
    }
    ArchiveWriter::new(&mut bytes, &header(), 4)
        .unwrap()
        .finish(&mut capture, &hashes())
        .unwrap();
    bytes
}

fn records(bytes: &[u8]) -> Vec<serde_json::Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn encode(records: &[serde_json::Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, record).unwrap();
        bytes.push(b'\n');
    }
    bytes
}

#[test]
fn fifo_overflow_exports_exact_ordered_gaps_and_completion() {
    let mut capture = Capture::new(1).unwrap();
    let mut bytes = Vec::new();
    let mut writer = ArchiveWriter::new(&mut bytes, &header(), 1).unwrap();
    for recorder in &mut capture.recorders {
        for id in 0..4 {
            record(recorder, origin(0, id));
        }
    }
    writer.drain(&mut capture).unwrap();
    for recorder in &mut capture.recorders {
        for id in 0..4 {
            record(
                recorder,
                Event {
                    tick: 9,
                    kind: EventKind::SpeciesExtinct {
                        species_id: SpeciesId::new(id),
                    },
                },
            );
        }
    }
    writer.finish(&mut capture, &hashes()).unwrap();
    let rows = records(&bytes);
    assert_eq!(rows[1]["data"]["sequence"], "0");
    assert_eq!(rows[2]["data"]["first_sequence"], "1");
    assert_eq!(rows[2]["data"]["last_sequence"], "3");
    assert_eq!(rows[5]["data"]["sequence"], "4");
    assert_eq!(rows[6]["data"]["first_sequence"], "5");
    assert_eq!(rows[6]["data"]["last_sequence"], "7");
    let summary = parse(Cursor::new(bytes)).unwrap();
    for cohort in summary.cohorts {
        assert_eq!(cohort.counts.next_sequence.0, 8);
        assert_eq!(cohort.counts.events.0, 2);
        assert_eq!(cohort.counts.dropped_events.0, 6);
        assert_eq!(cohort.counts.gaps.0, 2);
        assert!(!cohort.history_complete);
    }
}

#[test]
fn exact_large_ids_ticks_sequences_and_nullable_parent_metadata() {
    let big = (1u64 << 53) + 1;
    for value in [0, big, u64::MAX - 1, u64::MAX] {
        let json = serde_json::to_string(&Decimal(value)).unwrap();
        assert_eq!(json, format!("\"{value}\""));
        assert_eq!(serde_json::from_str::<Decimal>(&json).unwrap().0, value);
    }
    let mut run = header();
    run.ticks = u64::MAX;
    let mut bytes = Vec::new();
    let mut capture = Capture::new(4).unwrap();
    record(
        &mut capture.recorders[0],
        Event {
            tick: big,
            kind: EventKind::SpeciesOrigin {
                species_id: SpeciesId::new(9),
                founder_birth_id: BirthId::new(u64::MAX - 1),
                parent_a: Parent::Observed {
                    birth_id: BirthId::new(big),
                    species_id: None,
                },
                parent_b: Parent::Unavailable,
            },
        },
    );
    record(&mut capture.recorders[1], origin(0, 2));
    record(
        &mut capture.recorders[1],
        Event {
            tick: big,
            kind: EventKind::SpeciesOrigin {
                species_id: SpeciesId::new(9),
                founder_birth_id: BirthId::NULL,
                parent_a: Parent::Observed {
                    birth_id: BirthId::NULL,
                    species_id: Some(SpeciesId::new(2)),
                },
                parent_b: Parent::Absent,
            },
        },
    );
    ArchiveWriter::new(&mut bytes, &run, 4)
        .unwrap()
        .finish(&mut capture, &hashes())
        .unwrap();
    let rows = records(&bytes);
    assert_eq!(rows[0]["data"]["provenance"]["seed"], u64::MAX.to_string());
    assert_eq!(rows[1]["data"]["tick"], big.to_string());
    assert_eq!(
        rows[1]["data"]["event"]["founder_birth_id"],
        (u64::MAX - 1).to_string()
    );
    assert_eq!(
        rows[1]["data"]["event"]["parent_a"]["birth_id"],
        big.to_string()
    );
    assert_eq!(
        rows[1]["data"]["event"]["parent_a"]["species_id"],
        serde_json::Value::Null
    );
    assert_eq!(
        rows[3]["data"]["event"]["founder_birth_id"],
        serde_json::Value::Null
    );
    parse(Cursor::new(bytes)).unwrap();

    let gap = ArchiveRecord::Gap {
        cohort: Cohort::Evolving,
        first_sequence: Decimal(big),
        last_sequence: Decimal(u64::MAX - 1),
    };
    let json = serde_json::to_value(gap).unwrap();
    assert_eq!(json["data"]["first_sequence"], big.to_string());
    assert_eq!(json["data"]["last_sequence"], (u64::MAX - 1).to_string());
    serde_json::from_value::<ArchiveRecord>(json).unwrap();
}

#[test]
fn malformed_records_provenance_and_footer_totals_are_rejected() {
    let original = records(&fixture());
    let mutations: &[(&str, serde_json::Value)] = &[
        ("/0/data/schema_version", serde_json::json!(2)),
        ("/0/data/provenance/phase", serde_json::json!(1)),
        (
            "/0/data/provenance/control",
            serde_json::json!("randomized_at_birth"),
        ),
        ("/0/data/provenance/source_revision", serde_json::json!("")),
        (
            "/0/data/provenance/seed",
            serde_json::json!(9007199254740993u64),
        ),
        ("/0/data/drain_every", serde_json::json!("0")),
        ("/0/data/capacity_per_cohort", serde_json::json!(0)),
        ("/0/data/params/species/capacity", serde_json::json!(0)),
        ("/1/data/sequence", serde_json::json!("1")),
        (
            "/1/data/sequence",
            serde_json::json!("18446744073709551615"),
        ),
        ("/1/data/tick", serde_json::json!("01")),
        ("/1/data/tick", serde_json::json!("100")),
        ("/1/data/tick", serde_json::json!("+0")),
        ("/1/data/cohort", serde_json::json!("unknown")),
        ("/1/data/event/species_id", serde_json::json!(u32::MAX)),
        ("/1/data/event/founder_birth_id", serde_json::json!(0)),
        (
            "/1/data/event/founder_birth_id",
            serde_json::json!("18446744073709551615"),
        ),
        (
            "/1/data/event/parent_a",
            serde_json::json!({"status":"absent","birth_id":"0"}),
        ),
        (
            "/1/data/event/parent_a",
            serde_json::json!({"status":"observed","birth_id":"0","species_id":null}),
        ),
        (
            "/1/data/event/parent_a",
            serde_json::json!({"status":"observed","birth_id":null,"species_id":u32::MAX}),
        ),
        (
            "/1/data/event/parent_a",
            serde_json::json!({"status":"observed","birth_id":null}),
        ),
        (
            "/1/data/event/parent_a",
            serde_json::json!({"status":"observed","species_id":null}),
        ),
        ("/2/data/event/species_id", serde_json::json!(1)),
        ("/5/data/schema_version", serde_json::json!(2)),
        ("/5/data/provenance/seed", serde_json::json!("7")),
        ("/5/data/ticks", serde_json::json!("99")),
        (
            "/5/data/cohorts/0/cohort",
            serde_json::json!("random_control"),
        ),
        ("/5/data/cohorts/0/counts/events", serde_json::json!("0")),
        (
            "/5/data/cohorts/0/counts/dropped_events",
            serde_json::json!("1"),
        ),
        (
            "/5/data/cohorts/0/history_complete",
            serde_json::json!(false),
        ),
        (
            "/5/data/cohorts/0/final_state_hash",
            serde_json::json!("not a hash"),
        ),
    ];
    for (pointer, replacement) in mutations {
        let mut value = serde_json::Value::Array(original.clone());
        *value.pointer_mut(pointer).unwrap() = replacement.clone();
        assert!(
            parse(Cursor::new(encode(value.as_array().unwrap()))).is_err(),
            "accepted {pointer} = {replacement}"
        );
    }

    for (row, field) in [(0, "params"), (1, "sequence"), (5, "cohorts")] {
        let mut rows = original.clone();
        rows[row]["data"].as_object_mut().unwrap().remove(field);
        assert!(
            parse(Cursor::new(encode(&rows))).is_err(),
            "missing {field}"
        );
    }
    for field in ["founder_birth_id", "parent_a", "parent_b"] {
        let mut rows = original.clone();
        rows[1]["data"]["event"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            parse(Cursor::new(encode(&rows))).is_err(),
            "missing {field}"
        );
    }
    let mut rows = original.clone();
    rows[0]["data"]["params"]["world"]
        .as_object_mut()
        .unwrap()
        .remove("max_agents");
    assert!(parse(Cursor::new(encode(&rows))).is_err());
    let mut rows = original.clone();
    rows[1]["data"]["tick"] = serde_json::json!("8");
    rows[2]["data"]["tick"] = serde_json::json!("7");
    assert!(parse(Cursor::new(encode(&rows))).is_err());
}

#[test]
fn parents_cannot_contradict_known_species_founders() {
    let mut bytes = Vec::new();
    let mut capture = Capture::new(4).unwrap();
    for recorder in &mut capture.recorders {
        for (species, birth) in [(0, 0), (1, 5)] {
            record(
                recorder,
                Event {
                    tick: 0,
                    kind: EventKind::SpeciesOrigin {
                        species_id: SpeciesId::new(species),
                        founder_birth_id: BirthId::new(birth),
                        parent_a: Parent::Absent,
                        parent_b: Parent::Absent,
                    },
                },
            );
        }
        record(
            recorder,
            Event {
                tick: 1,
                kind: EventKind::SpeciesOrigin {
                    species_id: SpeciesId::new(2),
                    founder_birth_id: BirthId::new(10),
                    parent_a: Parent::Observed {
                        birth_id: BirthId::new(0),
                        species_id: Some(SpeciesId::new(0)),
                    },
                    parent_b: Parent::Absent,
                },
            },
        );
    }
    let mut run = header();
    run.founders = 6;
    ArchiveWriter::new(&mut bytes, &run, 4)
        .unwrap()
        .finish(&mut capture, &hashes())
        .unwrap();
    parse(Cursor::new(&bytes)).unwrap();
    let original = records(&bytes);
    for parent_field in ["parent_a", "parent_b"] {
        for (birth_id, species_id, message) in [
            ("0", Some(1), "predates its species origin"),
            ("4", Some(1), "predates its species origin"),
            ("5", Some(0), "contradicts its recorded species origin"),
            ("5", None, "contradicts its recorded species origin"),
        ] {
            let mut rows = original.clone();
            rows[3]["data"]["event"]["parent_a"] = serde_json::json!({"status":"absent"});
            rows[3]["data"]["event"][parent_field] = serde_json::json!({
                "status":"observed", "birth_id":birth_id, "species_id":species_id,
            });
            let error = parse(Cursor::new(encode(&rows))).unwrap_err();
            assert!(
                error.to_string().contains(message),
                "{parent_field}: {error}"
            );
        }
        for (birth_id, species_id) in [("0", Some(0)), ("5", Some(1)), ("7", Some(1)), ("7", None)]
        {
            let mut rows = original.clone();
            rows[3]["data"]["event"]["parent_a"] = serde_json::json!({"status":"absent"});
            rows[3]["data"]["event"][parent_field] = serde_json::json!({
                "status":"observed", "birth_id":birth_id, "species_id":species_id,
            });
            parse(Cursor::new(encode(&rows))).unwrap();
        }
    }
}

#[test]
fn truncation_duplicate_records_blank_lines_and_oversized_lines_fail() {
    let bytes = fixture();
    parse(BufReader::with_capacity(7, Cursor::new(&bytes))).unwrap();
    for end in [0, 1, bytes.len() / 2, bytes.len() - 1] {
        assert!(parse(Cursor::new(&bytes[..end])).is_err());
    }
    let original = records(&bytes);
    for rows in [
        original[..original.len() - 1].to_vec(),
        original[1..].to_vec(),
        [original.clone(), vec![original[1].clone()]].concat(),
        [original[..1].to_vec(), original.clone()].concat(),
    ] {
        assert!(parse(Cursor::new(encode(&rows))).is_err());
    }
    let mut bytes = fixture();
    bytes.extend_from_slice(b"\n");
    assert!(parse(Cursor::new(bytes)).is_err());
    assert!(parse(Cursor::new(vec![b' '; 1024 * 1024 + 1])).is_err());
}

#[test]
fn gap_ranges_and_exhausted_counter_boundary_validate_without_overflow() {
    let mut rows = records(&fixture());
    rows[1] = serde_json::json!({"kind":"gap","data":{
        "cohort":"evolving","first_sequence":"0","last_sequence":(u64::MAX - 2).to_string()
    }});
    rows[2]["data"]["sequence"] = serde_json::json!((u64::MAX - 1).to_string());
    rows[5]["data"]["cohorts"][0]["counts"] = serde_json::json!({
        "next_sequence":u64::MAX.to_string(), "events":"1", "dropped_events":(u64::MAX - 1).to_string(),
        "gaps":"1", "origins":"0", "extinctions":"1"
    });
    rows[5]["data"]["cohorts"][0]["history_complete"] = serde_json::json!(false);
    parse(Cursor::new(encode(&rows))).unwrap();
    for (first, last) in [("1", "3"), ("3", "1"), ("0", "18446744073709551615")] {
        let mut bad = rows.clone();
        bad[1]["data"]["first_sequence"] = serde_json::json!(first);
        bad[1]["data"]["last_sequence"] = serde_json::json!(last);
        assert!(parse(Cursor::new(encode(&bad))).is_err());
    }
}

struct FailingWriter {
    writes_left: usize,
    fail_flush: bool,
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.writes_left == 0 {
            return Err(io::Error::other("injected write failure"));
        }
        let written = bytes.len().min(self.writes_left);
        self.writes_left -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn writer_and_flush_errors_are_propagated() {
    let output = FailingWriter {
        writes_left: 0,
        fail_flush: false,
    };
    assert!(ArchiveWriter::new(output, &header(), 4).is_err());
    let output = FailingWriter {
        writes_left: usize::MAX,
        fail_flush: true,
    };
    let mut writer = ArchiveWriter::new(output, &header(), 4).unwrap();
    let mut capture = Capture::new(4).unwrap();
    record(&mut capture.recorders[0], origin(0, 0));
    assert!(writer.drain(&mut capture).is_err());
    let output = FailingWriter {
        writes_left: usize::MAX,
        fail_flush: false,
    };
    let mut writer = ArchiveWriter::new(output, &header(), 4).unwrap();
    writer.output.writes_left = 0;
    assert!(
        writer
            .finish(&mut Capture::new(4).unwrap(), &hashes())
            .is_err()
    );
}
