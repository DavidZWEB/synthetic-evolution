//! Shell-owned bounded capture and streaming species-history export.
//!
//! Callbacks only enqueue preallocated records and, when requested, stage origin
//! representatives in preallocated buffers. Draining and all I/O happen outside
//! World; this is neither a complete genealogy nor a checkpoint.

use std::io::{self, Write};

use sim_core::genome::Gene;
use sim_core::history::{
    Capture as Captured, Event, Record, Recorder, RepresentativeBuffer, SequenceExhausted,
};

use crate::Result;
use crate::cli::HistoryArgs;
use crate::history_wire::{
    ArchiveRecord, CaptureEnd, Cohort, CohortCompletion, Completion, Counts, Decimal, EventRecord,
    Header, MAX_LINE_BYTES, RepresentativeRecord, UnavailableRepresentative,
};
use crate::metrics::{HistoryAvailability, RunHeader, StateHashes};

pub(crate) struct CohortCapture {
    pub recorder: Recorder,
    representatives: Option<RepresentativeBuffer>,
}

impl CohortCapture {
    fn new(capacity: u32, representative_genes: Option<u32>) -> Result<Self> {
        Ok(Self {
            recorder: Recorder::try_new(capacity)?,
            // A recorder holds at most `capacity` undrained origins, so staging never
            // needs more genome slots than that.
            representatives: representative_genes
                .map(|genes| RepresentativeBuffer::try_new(genes, capacity))
                .transpose()?,
        })
    }

    /// History callback: queue the event and stage an origin's representative.
    pub fn record(&mut self, event: Event, representative: Option<&[Gene]>) {
        // Recorder latches exhaustion. The shell reports it after stepping, never
        // panicking or doing I/O from a callback in the tick.
        if self.recorder.record(event) == Ok(Captured::Recorded)
            && let (Some(buffer), Some(genes)) = (&mut self.representatives, representative)
        {
            // A refusal leaves nothing staged, so the drain archives it as unavailable.
            let _ = buffer.store(self.recorder.next_sequence() - 1, genes);
        }
    }
}

pub(crate) struct Capture {
    pub cohorts: [CohortCapture; 2],
}

impl Capture {
    pub fn new(capacity: u32, representative_genes: Option<u32>) -> Result<Self> {
        Ok(Self {
            cohorts: [
                CohortCapture::new(capacity, representative_genes)?,
                CohortCapture::new(capacity, representative_genes)?,
            ],
        })
    }

    pub fn check(&self) -> Result<()> {
        if self
            .cohorts
            .iter()
            .any(|cohort| cohort.recorder.sequence_exhausted())
        {
            return Err(SequenceExhausted.into());
        }
        Ok(())
    }
}

pub(crate) struct ArchiveWriter<W: Write> {
    output: W,
    header: Header,
    counts: [Counts; 2],
}

impl<W: Write> ArchiveWriter<W> {
    pub fn new(
        mut output: W,
        run: &RunHeader,
        capacity: u32,
        representative_genes: Option<u32>,
    ) -> Result<Self> {
        let header = Header::new(run, capacity, representative_genes)?;
        write_record(
            &mut output,
            &ArchiveRecord::Header(Box::new(header.clone())),
        )?;
        let mut counts = Counts::default();
        if representative_genes.is_some() {
            counts.representatives = Some(Decimal(0));
            counts.unavailable_representatives = Some(Decimal(0));
        }
        Ok(Self {
            output,
            header,
            counts: [counts.clone(), counts],
        })
    }

    pub fn drain(&mut self, capture: &mut Capture) -> Result<()> {
        capture.check()?;
        for cohort in Cohort::ALL {
            let counts = &mut self.counts[cohort.index()];
            let source = &mut capture.cohorts[cohort.index()];
            while let Some(record) = source.recorder.pop() {
                let line = match record {
                    Record::Event { sequence, event } => {
                        counts.next_sequence = Decimal(sequence + 1);
                        counts.events.0 += 1;
                        let event_record: EventRecord = event.kind.into();
                        let origin = matches!(event_record, EventRecord::SpeciesOrigin { .. });
                        if origin {
                            counts.origins.0 += 1;
                        } else {
                            counts.extinctions.0 += 1;
                        }
                        let representative = source
                            .representatives
                            .as_mut()
                            .filter(|_| origin)
                            .map(|buffer| match buffer.take(sequence) {
                                Some(genes) => RepresentativeRecord::Recorded {
                                    genes: genes.to_vec(),
                                },
                                None => RepresentativeRecord::Unavailable {
                                    reason: UnavailableRepresentative::CapturePressure,
                                },
                            });
                        let mut record = ArchiveRecord::Event {
                            cohort,
                            sequence: Decimal(sequence),
                            tick: Decimal(event.tick),
                            event: event_record,
                            representative,
                        };
                        let line = match encode_record(&record) {
                            Ok(line) => line,
                            Err(_) if origin && source.representatives.is_some() => {
                                if let ArchiveRecord::Event { representative, .. } = &mut record {
                                    *representative = Some(RepresentativeRecord::Unavailable {
                                        reason: UnavailableRepresentative::LineLimit,
                                    });
                                }
                                encode_record(&record)?
                            }
                            Err(error) => return Err(error),
                        };
                        if let ArchiveRecord::Event {
                            representative: Some(representative),
                            ..
                        } = &record
                        {
                            let count = match representative {
                                RepresentativeRecord::Recorded { .. } => {
                                    &mut counts.representatives
                                }
                                RepresentativeRecord::Unavailable { .. } => {
                                    &mut counts.unavailable_representatives
                                }
                            };
                            count.as_mut().expect("version three counts").0 += 1;
                        }
                        line
                    }
                    Record::Gap {
                        first_sequence,
                        last_sequence,
                    } => {
                        counts.next_sequence = Decimal(last_sequence + 1);
                        counts.dropped_events.0 += last_sequence - first_sequence + 1;
                        counts.gaps.0 += 1;
                        encode_record(&ArchiveRecord::Gap {
                            cohort,
                            first_sequence: Decimal(first_sequence),
                            last_sequence: Decimal(last_sequence),
                        })?
                    }
                };
                self.output.write_all(&line)?;
            }
        }
        self.output.flush()?;
        Ok(())
    }

    /// Cumulative counts as of the last drain, matching what the archive now holds.
    pub fn availability(&self) -> [HistoryAvailability; 2] {
        self.counts.clone().map(|counts| HistoryAvailability {
            capacity: self.header.capacity_per_cohort,
            retained_events: counts.events.0,
            dropped_events: counts.dropped_events.0,
            gaps: counts.gaps.0,
        })
    }

    pub fn finish(mut self, capture: &mut Capture, hashes: &StateHashes) -> Result<()> {
        self.drain(capture)?;
        let cohorts = Cohort::ALL.map(|cohort| {
            let counts = self.counts[cohort.index()].clone();
            let recorder = &capture.cohorts[cohort.index()].recorder;
            debug_assert_eq!(counts.next_sequence.0, recorder.next_sequence());
            debug_assert_eq!(counts.dropped_events.0, recorder.dropped_events());
            CohortCompletion {
                cohort,
                history_complete: counts.dropped_events.0 == 0,
                counts,
                final_state_hash: Some(match cohort {
                    Cohort::Evolving => hashes.evolving.clone(),
                    Cohort::RandomControl => hashes.random_control.clone(),
                }),
            }
        });
        write_record(
            &mut self.output,
            &ArchiveRecord::Complete(Completion {
                schema_version: self.header.schema_version,
                run_id: None,
                provenance: self.header.provenance,
                ticks: self.header.ticks.expect("native export has planned ticks"),
                capture_end: None,
                cohorts: cohorts.to_vec(),
            }),
        )?;
        self.output.flush()?;
        Ok(())
    }
}

pub(crate) fn validate_export(
    run: &RunHeader,
    capacity: u32,
    representative_genes: Option<u32>,
) -> Result<()> {
    if let Some(genes) = representative_genes
        && genes < run.params.storage.max_genes
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "--representative-genes {genes} cannot hold one maximum-size genome ({} genes)",
                run.params.storage.max_genes
            ),
        )
        .into());
    }
    let header = Header::new(run, capacity, representative_genes)?;
    encode_record(&ArchiveRecord::Header(Box::new(header)))?;
    Ok(())
}

fn write_record(writer: &mut impl Write, record: &ArchiveRecord) -> Result<()> {
    writer.write_all(&encode_record(record)?)?;
    Ok(())
}

fn encode_record(record: &ArchiveRecord) -> Result<Vec<u8>> {
    struct Line(Vec<u8>);
    impl Write for Line {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self
                .0
                .len()
                .checked_add(bytes.len())
                .is_none_or(|length| length as u64 >= MAX_LINE_BYTES)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "history record exceeds 1 MiB",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut line = Line(Vec::new());
    serde_json::to_writer(&mut line, record)?;
    line.0.push(b'\n');
    Ok(line.0)
}

pub fn summarize(args: HistoryArgs) -> Result<()> {
    let summary = crate::history_reader::read(&args.history)?;
    let mut output = io::stdout().lock();
    if args.json {
        serde_json::to_writer_pretty(&mut output, &summary)?;
        writeln!(output)?;
    } else {
        write_summary(&mut output, &summary)?;
    }
    output.flush()?;
    Ok(())
}

fn write_summary(output: &mut impl Write, summary: &Completion) -> Result<()> {
    match summary.capture_end {
        None | Some(CaptureEnd::Finished) => writeln!(
            output,
            "completed {} ticks; species history schema {}",
            summary.ticks.0, summary.schema_version
        )?,
        Some(reason) => writeln!(
            output,
            "{}capture prefix through {} ticks ({}); species history schema {}",
            if reason.is_incomplete() {
                "incomplete "
            } else {
                ""
            },
            summary.ticks.0,
            reason.as_str(),
            summary.schema_version
        )?,
    }
    for cohort in &summary.cohorts {
        let counts = &cohort.counts;
        let representatives = match (counts.representatives, counts.unavailable_representatives) {
            (Some(archived), Some(unavailable)) => format!(
                "; {} representatives archived, {} unavailable",
                archived.0, unavailable.0
            ),
            _ => String::new(),
        };
        writeln!(
            output,
            "{:?}: {} origins, {} extinctions retained; {} dropped events in {} gaps{representatives}; hash {}",
            cohort.cohort,
            counts.origins.0,
            counts.extinctions.0,
            counts.dropped_events.0,
            counts.gaps.0,
            cohort.final_state_hash.as_deref().unwrap_or("unavailable"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Archive boundary regressions, independent of the simulator's ecological rules.

    use std::io::{self, BufReader, Cursor, Write};

    use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;
    use sim_core::history::{Event, EventKind, Parent};
    use sim_core::ids::{BirthId, SpeciesId};
    use sim_core::params::SimParams;

    use super::*;
    use crate::history_reader::parse;
    use crate::history_wire::{ArchiveRecord, Decimal};

    fn record(cohort: &mut CohortCapture, event: Event) {
        cohort.record(event, None);
    }

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
        let mut capture = Capture::new(4, None).unwrap();
        for recorder in &mut capture.cohorts {
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
        ArchiveWriter::new(&mut bytes, &header(), 4, None)
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

    fn browser_fixture(cohorts: &[Cohort], reason: CaptureEnd) -> Vec<serde_json::Value> {
        let mut rows = records(&fixture());
        let cohort_names = serde_json::to_value(cohorts).unwrap();
        rows.retain(|row| {
            row["kind"] == "header"
                || row["kind"] == "complete"
                || cohort_names
                    .as_array()
                    .unwrap()
                    .contains(&row["data"]["cohort"])
        });
        rows[0]["data"]["schema_version"] = serde_json::json!(2);
        rows[0]["data"]["run_id"] = serde_json::json!("browser-run-1");
        rows[0]["data"]["cohorts"] = cohort_names.clone();
        rows[0]["data"]["ticks"] = serde_json::Value::Null;
        rows[0]["data"]["drain_every"] = serde_json::Value::Null;
        let footer = rows.last_mut().unwrap();
        footer["data"]["schema_version"] = serde_json::json!(2);
        footer["data"]["run_id"] = serde_json::json!("browser-run-1");
        footer["data"]["ticks"] = serde_json::json!("10");
        footer["data"]["capture_end"] = serde_json::to_value(reason).unwrap();
        footer["data"]["cohorts"]
            .as_array_mut()
            .unwrap()
            .retain(|row| cohort_names.as_array().unwrap().contains(&row["cohort"]));
        for row in footer["data"]["cohorts"].as_array_mut().unwrap() {
            row["history_complete"] = serde_json::json!(!reason.is_incomplete());
            if reason.is_incomplete() {
                row["final_state_hash"] = serde_json::Value::Null;
            }
        }
        if reason == CaptureEnd::Finished {
            rows[0]["data"]["ticks"] = serde_json::json!("10");
        }
        rows
    }

    fn empty_browser_fixture(reason: CaptureEnd) -> Vec<serde_json::Value> {
        let mut rows = browser_fixture(&[Cohort::Evolving], reason);
        rows.retain(|row| row["kind"] != "event");
        rows[1]["data"]["ticks"] = serde_json::json!("0");
        rows[1]["data"]["cohorts"][0]["counts"] = serde_json::to_value(Counts::default()).unwrap();
        rows
    }

    #[test]
    fn native_v1_export_and_summary_keep_original_shape() {
        let bytes = fixture();
        let rows = records(&bytes);
        let header = &rows[0]["data"];
        let footer = &rows.last().unwrap()["data"];
        assert_eq!(header["schema_version"], 1);
        assert_eq!(
            header["cohorts"],
            serde_json::json!(["evolving", "random_control"])
        );
        assert_eq!(header["ticks"], "100");
        assert_eq!(header["drain_every"], "10");
        assert!(header.get("run_id").is_none());
        assert!(footer.get("run_id").is_none());
        assert!(footer.get("capture_end").is_none());
        let summary = parse(Cursor::new(bytes)).unwrap();
        assert_eq!(serde_json::to_value(&summary).unwrap(), *footer);
        let mut output = Vec::new();
        write_summary(&mut output, &summary).unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .starts_with("completed 100 ticks;")
        );
        for (row, field) in [(0, "run_id"), (5, "run_id"), (5, "capture_end")] {
            for value in [serde_json::Value::Null, serde_json::json!("snapshot")] {
                let mut bad = rows.clone();
                bad[row]["data"][field] = value;
                assert!(parse(Cursor::new(encode(&bad))).is_err());
            }
        }
        for field in ["ticks", "drain_every"] {
            let mut bad = rows.clone();
            bad[0]["data"][field] = serde_json::Value::Null;
            assert!(parse(Cursor::new(encode(&bad))).is_err());
        }
        let mut bad = rows.clone();
        bad[5]["data"]["cohorts"][0]["final_state_hash"] = serde_json::Value::Null;
        assert!(parse(Cursor::new(encode(&bad))).is_err());
    }

    #[test]
    fn browser_single_world_and_paired_cohorts_preserve_capture_end_semantics() {
        for cohorts in [
            &[Cohort::Evolving][..],
            &[Cohort::RandomControl][..],
            &Cohort::ALL[..],
        ] {
            for reason in [
                CaptureEnd::Finished,
                CaptureEnd::Snapshot,
                CaptureEnd::Stopped,
                CaptureEnd::Reseeded,
                CaptureEnd::ParamsChanged,
                CaptureEnd::Unfinalized,
                CaptureEnd::StorageLimit,
                CaptureEnd::StorageError,
                CaptureEnd::CaptureError,
            ] {
                let rows = browser_fixture(cohorts, reason);
                let summary = parse(Cursor::new(encode(&rows))).unwrap();
                assert_eq!(summary.capture_end, Some(reason));
                assert_eq!(summary.run_id.as_deref(), Some("browser-run-1"));
                assert_eq!(summary.cohorts.len(), cohorts.len());
                let json = serde_json::to_value(&summary).unwrap();
                assert_eq!(json, rows.last().unwrap()["data"]);
                for (row, &cohort) in summary.cohorts.iter().zip(cohorts) {
                    assert_eq!(row.cohort, cohort);
                    assert_eq!(row.counts.events.0, 2);
                    assert_eq!(row.history_complete, !reason.is_incomplete());
                    assert_eq!(row.final_state_hash.is_none(), reason.is_incomplete());
                }
                let mut output = Vec::new();
                write_summary(&mut output, &summary).unwrap();
                let text = String::from_utf8(output).unwrap();
                if reason == CaptureEnd::Finished {
                    assert!(text.starts_with("completed 10 ticks;"));
                } else {
                    assert!(!text.contains("completed"));
                    assert!(text.contains("capture prefix through 10 ticks"));
                    assert!(text.contains(reason.as_str()));
                    assert_eq!(text.starts_with("incomplete "), reason.is_incomplete());
                }
            }
        }
    }

    #[test]
    fn browser_root_only_snapshot_preserves_the_zero_tick_seeding_exception() {
        let mut rows = browser_fixture(&[Cohort::RandomControl], CaptureEnd::Snapshot);
        rows.remove(2);
        rows[2]["data"]["ticks"] = serde_json::json!("0");
        rows[2]["data"]["cohorts"][0]["counts"] = serde_json::json!({
            "next_sequence":"1", "events":"1", "dropped_events":"0",
            "gaps":"0", "origins":"1", "extinctions":"0"
        });
        for planned in [
            serde_json::Value::Null,
            serde_json::json!("0"),
            serde_json::json!("100"),
        ] {
            rows[0]["data"]["ticks"] = planned;
            let summary = parse(Cursor::new(encode(&rows))).unwrap();
            assert_eq!(summary.ticks.0, 0);
        }
        rows[0]["data"]["ticks"] = serde_json::Value::Null;
        for parent in [
            serde_json::json!({"status":"unavailable"}),
            serde_json::json!({"status":"observed","birth_id":"0","species_id":null}),
        ] {
            let mut bad = rows.clone();
            bad[0]["data"]["founders"] = serde_json::json!(2);
            bad[1]["data"]["event"]["founder_birth_id"] = serde_json::json!("1");
            bad[1]["data"]["event"]["parent_a"] = parent;
            let error = parse(Cursor::new(encode(&bad))).unwrap_err();
            assert!(
                error.to_string().contains("captured tick boundary"),
                "{error}"
            );
        }
        let mut bad = rows.clone();
        bad[1]["data"]["tick"] = serde_json::json!("1");
        assert!(parse(Cursor::new(encode(&bad))).is_err());
        let mut bad = browser_fixture(&[Cohort::Evolving], CaptureEnd::Snapshot);
        bad[2]["data"]["tick"] = serde_json::json!("0");
        bad[3]["data"]["ticks"] = serde_json::json!("0");
        let error = parse(Cursor::new(encode(&bad))).unwrap_err();
        assert!(
            error.to_string().contains("captured tick boundary"),
            "{error}"
        );
    }

    #[test]
    fn zero_tick_origins_require_available_ids_below_requested_founders() {
        for fixture in [
            &include_bytes!("../tests/fixtures/history-v1.ndjson")[..],
            &include_bytes!("../tests/fixtures/history-v2-root.ndjson")[..],
        ] {
            let original = records(fixture);
            let browser = original[0]["data"]["schema_version"] == 2;
            for with_gaps in [false, true] {
                let mut rows = vec![original[0].clone()];
                rows[0]["data"]["founders"] = serde_json::json!(4);
                for origin in &original[1..original.len() - 1] {
                    let mut origin = origin.clone();
                    origin["data"]["event"]["founder_birth_id"] = serde_json::json!("3");
                    if with_gaps {
                        rows.push(serde_json::json!({"kind":"gap","data":{
                            "cohort":origin["data"]["cohort"],
                            "first_sequence":"0", "last_sequence":"0"
                        }}));
                        origin["data"]["sequence"] = serde_json::json!("1");
                    }
                    rows.push(origin);
                    if with_gaps {
                        rows.push(serde_json::json!({"kind":"gap","data":{
                            "cohort":rows.last().unwrap()["data"]["cohort"],
                            "first_sequence":"2", "last_sequence":"2"
                        }}));
                    }
                }
                rows.push(original.last().unwrap().clone());
                for cohort in rows.last_mut().unwrap()["data"]["cohorts"]
                    .as_array_mut()
                    .unwrap()
                {
                    if with_gaps {
                        cohort["counts"] = serde_json::json!({
                            "next_sequence":"3", "events":"1", "dropped_events":"2",
                            "gaps":"2", "origins":"1", "extinctions":"0"
                        });
                        cohort["history_complete"] = serde_json::json!(false);
                    }
                }
                let reasons: &[Option<CaptureEnd>] = if browser {
                    &[
                        Some(CaptureEnd::Snapshot),
                        Some(CaptureEnd::Unfinalized),
                        Some(CaptureEnd::StorageLimit),
                        Some(CaptureEnd::StorageError),
                        Some(CaptureEnd::CaptureError),
                    ]
                } else {
                    &[None]
                };
                for &reason in reasons {
                    let plans = if browser {
                        vec![
                            serde_json::Value::Null,
                            serde_json::json!("0"),
                            serde_json::json!("100"),
                        ]
                    } else {
                        vec![serde_json::json!("0")]
                    };
                    for planned in plans {
                        let mut valid = rows.clone();
                        valid[0]["data"]["ticks"] = planned.clone();
                        if let Some(reason) = reason {
                            let footer = &mut valid.last_mut().unwrap()["data"];
                            footer["capture_end"] = serde_json::to_value(reason).unwrap();
                            for cohort in footer["cohorts"].as_array_mut().unwrap() {
                                cohort["history_complete"] =
                                    serde_json::json!(!with_gaps && !reason.is_incomplete());
                                if reason.is_incomplete() {
                                    cohort["final_state_hash"] = serde_json::Value::Null;
                                }
                            }
                        }
                        parse(Cursor::new(encode(&valid))).unwrap();
                        for index in 1..valid.len() - 1 {
                            if valid[index]["kind"] != "event" {
                                continue;
                            }
                            for birth in [
                                serde_json::json!("4"),
                                serde_json::json!("9007199254740993"),
                                serde_json::json!("18446744073709551614"),
                                serde_json::Value::Null,
                            ] {
                                let mut bad = valid.clone();
                                bad[index]["data"]["event"]["founder_birth_id"] = birth.clone();
                                let error = parse(Cursor::new(encode(&bad))).unwrap_err();
                                let expected_line =
                                    if planned == "0" { index + 1 } else { bad.len() };
                                assert!(
                                    error
                                        .to_string()
                                        .contains(&format!("history line {expected_line}:")),
                                    "{reason:?}, {planned}, {birth}: {error}"
                                );
                                assert!(
                                    error.to_string().contains("seeding-only")
                                        || error.to_string().contains("captured tick boundary"),
                                    "{error}"
                                );
                                if !browser || planned == "0" {
                                    bad[0]["data"]["ticks"] = serde_json::json!("1");
                                }
                                bad.last_mut().unwrap()["data"]["ticks"] = serde_json::json!("1");
                                for status in ["absent", "unavailable"] {
                                    bad[index]["data"]["event"]["parent_a"] =
                                        serde_json::json!({"status":status});
                                    parse(Cursor::new(encode(&bad))).unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn incomplete_browser_prefix_can_contain_only_header_and_footer() {
        for reason in [
            CaptureEnd::Unfinalized,
            CaptureEnd::StorageLimit,
            CaptureEnd::StorageError,
            CaptureEnd::CaptureError,
        ] {
            let rows = empty_browser_fixture(reason);
            let summary = parse(Cursor::new(encode(&rows))).unwrap();
            assert_eq!(summary.cohorts[0].counts, Counts::default());
            assert!(!summary.cohorts[0].history_complete);
            assert_eq!(summary.cohorts[0].final_state_hash, None);
            assert!(parse(Cursor::new(encode(&rows[..1]))).is_err());
            let mut bad = rows.clone();
            bad[1]["data"]["cohorts"][0]["history_complete"] = serde_json::json!(true);
            assert!(parse(Cursor::new(encode(&bad))).is_err());
            let mut with_hash = rows.clone();
            with_hash[1]["data"]["cohorts"][0]["final_state_hash"] =
                serde_json::json!("0123456789abcdef");
            parse(Cursor::new(encode(&with_hash))).unwrap();
        }
        for reason in [
            CaptureEnd::Finished,
            CaptureEnd::Snapshot,
            CaptureEnd::Stopped,
            CaptureEnd::Reseeded,
            CaptureEnd::ParamsChanged,
        ] {
            let mut rows = empty_browser_fixture(reason);
            rows[0]["data"]["ticks"] = serde_json::json!("0");
            assert!(parse(Cursor::new(encode(&rows))).is_err());
            rows[0]["data"]["params"]["species"]["capacity"] = serde_json::json!(0);
            parse(Cursor::new(encode(&rows))).unwrap();
        }
    }

    #[test]
    fn browser_gap_prefix_counts_are_exact_and_limit_parent_certainty() {
        for reason in [CaptureEnd::Snapshot, CaptureEnd::StorageLimit] {
            let mut rows = browser_fixture(&[Cohort::Evolving], reason);
            rows[1] = serde_json::json!({"kind":"gap","data":{
                "cohort":"evolving","first_sequence":"0","last_sequence":"0"
            }});
            rows[3]["data"]["cohorts"][0]["counts"] = serde_json::json!({
                "next_sequence":"2", "events":"1", "dropped_events":"1",
                "gaps":"1", "origins":"0", "extinctions":"1"
            });
            rows[3]["data"]["cohorts"][0]["history_complete"] = serde_json::json!(false);
            parse(Cursor::new(encode(&rows))).unwrap();
            for field in [
                "next_sequence",
                "events",
                "dropped_events",
                "gaps",
                "origins",
                "extinctions",
            ] {
                let mut bad = rows.clone();
                bad[3]["data"]["cohorts"][0]["counts"][field] = serde_json::json!("9");
                assert!(parse(Cursor::new(encode(&bad))).is_err(), "{field}");
            }
            let mut bad = rows.clone();
            bad[1]["data"]["cohort"] = serde_json::json!("random_control");
            let error = parse(Cursor::new(encode(&bad))).unwrap_err();
            assert!(error.to_string().contains("absent from header"), "{error}");
        }
    }

    #[test]
    fn browser_requires_matching_cohorts_run_identity_and_required_nullable_metadata() {
        let original = browser_fixture(&[Cohort::Evolving], CaptureEnd::Snapshot);
        let mutations = [
            ("/0/data/schema_version", serde_json::json!(3)),
            ("/0/data/run_id", serde_json::json!("")),
            ("/0/data/run_id", serde_json::json!("x".repeat(129))),
            ("/0/data/run_id", serde_json::Value::Null),
            ("/0/data/cohorts", serde_json::json!([])),
            (
                "/0/data/cohorts",
                serde_json::json!(["evolving", "evolving"]),
            ),
            (
                "/0/data/cohorts",
                serde_json::json!(["random_control", "evolving"]),
            ),
            (
                "/0/data/cohorts",
                serde_json::json!(["evolving", "random_control", "evolving"]),
            ),
            ("/0/data/drain_every", serde_json::json!("0")),
            ("/0/data/capacity_per_cohort", serde_json::json!(u32::MAX)),
            ("/0/data/params/world", serde_json::json!({})),
            ("/1/data/cohort", serde_json::json!("random_control")),
            ("/3/data/run_id", serde_json::json!("different-run")),
            ("/3/data/run_id", serde_json::Value::Null),
            ("/3/data/schema_version", serde_json::json!(1)),
            ("/3/data/capture_end", serde_json::json!("crashed")),
            ("/3/data/capture_end", serde_json::Value::Null),
            ("/3/data/provenance/seed", serde_json::json!("1")),
            ("/3/data/cohorts", serde_json::json!([])),
            (
                "/3/data/cohorts/0/cohort",
                serde_json::json!("random_control"),
            ),
            (
                "/3/data/cohorts/0/final_state_hash",
                serde_json::Value::Null,
            ),
            (
                "/3/data/cohorts/0/final_state_hash",
                serde_json::json!("0123456789ABCDEF"),
            ),
            (
                "/3/data/cohorts/0/final_state_hash",
                serde_json::json!("0123456789abcde"),
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
        for (row, field) in [
            (0, "run_id"),
            (0, "ticks"),
            (0, "drain_every"),
            (3, "run_id"),
            (3, "capture_end"),
        ] {
            let mut rows = original.clone();
            rows[row]["data"].as_object_mut().unwrap().remove(field);
            assert!(
                parse(Cursor::new(encode(&rows))).is_err(),
                "missing {field}"
            );
        }
        let mut rows = original.clone();
        rows[3]["data"]["cohorts"][0]
            .as_object_mut()
            .unwrap()
            .remove("final_state_hash");
        assert!(parse(Cursor::new(encode(&rows))).is_err());
        let mut rows = browser_fixture(&Cohort::ALL, CaptureEnd::Snapshot);
        rows.last_mut().unwrap()["data"]["cohorts"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(parse(Cursor::new(encode(&rows))).is_err());
        let mut rows = original.clone();
        let extra = rows[3]["data"]["cohorts"][0].clone();
        rows[3]["data"]["cohorts"]
            .as_array_mut()
            .unwrap()
            .push(extra);
        assert!(parse(Cursor::new(encode(&rows))).is_err());
        let mut rows = original;
        for row in [0, 3] {
            rows[row]["data"]["run_id"] = serde_json::json!("x".repeat(128));
        }
        rows[0]["data"]["drain_every"] = serde_json::json!("1");
        parse(Cursor::new(encode(&rows))).unwrap();
    }

    #[test]
    fn browser_run_ids_count_unicode_scalars_and_reject_unpaired_surrogates() {
        let original = browser_fixture(&[Cohort::Evolving], CaptureEnd::Snapshot);
        for (id, accepted) in [
            (" ".to_owned(), true),
            ("opaque/not-a-uuid".to_owned(), true),
            ("\u{1f9ec}".repeat(128), true),
            ("\u{1f9ec}".repeat(129), false),
        ] {
            let mut rows = original.clone();
            for index in [0, 3] {
                rows[index]["data"]["run_id"] = serde_json::json!(id);
            }
            assert_eq!(
                parse(Cursor::new(encode(&rows))).is_ok(),
                accepted,
                "{} Unicode scalars",
                id.chars().count()
            );
        }
        for pointer in [
            "/run_id",
            "/provenance/sim_version",
            "/provenance/source_revision",
        ] {
            let mut rows = original.clone();
            for index in [0, 3] {
                *rows[index]["data"].pointer_mut(pointer).unwrap() =
                    serde_json::json!("WIRE_SURROGATE");
            }
            parse(Cursor::new(encode(&rows))).unwrap();
            let text = String::from_utf8(encode(&rows)).unwrap();
            for escaped in [r#""\ud800""#, r#""\udfff""#] {
                let invalid = text.replace("\"WIRE_SURROGATE\"", escaped);
                let error = parse(Cursor::new(invalid)).unwrap_err();
                assert!(error.to_string().contains("history line 1:"), "{error}");
            }
        }
    }

    #[test]
    fn browser_footer_validates_planned_and_captured_tick_boundaries_and_hashes() {
        for reason in [
            CaptureEnd::Finished,
            CaptureEnd::Snapshot,
            CaptureEnd::Stopped,
            CaptureEnd::Reseeded,
            CaptureEnd::ParamsChanged,
            CaptureEnd::Unfinalized,
            CaptureEnd::StorageLimit,
            CaptureEnd::StorageError,
            CaptureEnd::CaptureError,
        ] {
            let original = browser_fixture(&[Cohort::Evolving], reason);
            for boundary in ["0", "1", "9"] {
                let mut bad = original.clone();
                bad[3]["data"]["ticks"] = serde_json::json!(boundary);
                assert!(
                    parse(Cursor::new(encode(&bad))).is_err(),
                    "{reason:?}, {boundary}"
                );
            }
            let mut bad = original.clone();
            bad[0]["data"]["ticks"] = serde_json::json!("9");
            assert!(parse(Cursor::new(encode(&bad))).is_err());
            let mut bad = original.clone();
            bad[0]["data"]["ticks"] = serde_json::json!("10");
            bad[3]["data"]["ticks"] = serde_json::json!("11");
            assert!(parse(Cursor::new(encode(&bad))).is_err());
            let mut bad = original.clone();
            bad[2]["data"]["tick"] = serde_json::json!("10");
            assert!(parse(Cursor::new(encode(&bad))).is_err());
            let mut bad = original.clone();
            bad[3]["data"]["cohorts"][0]["final_state_hash"] = serde_json::json!("not a hash");
            assert!(parse(Cursor::new(encode(&bad))).is_err());
            let mut rows = original.clone();
            rows[3]["data"]["cohorts"][0]["final_state_hash"] = serde_json::Value::Null;
            assert_eq!(
                parse(Cursor::new(encode(&rows))).is_ok(),
                reason.is_incomplete()
            );
            let mut rows = original.clone();
            rows[0]["data"]["ticks"] = serde_json::json!("11");
            assert_eq!(
                parse(Cursor::new(encode(&rows))).is_ok(),
                reason != CaptureEnd::Finished
            );
            rows[0]["data"]["ticks"] = serde_json::Value::Null;
            assert_eq!(
                parse(Cursor::new(encode(&rows))).is_ok(),
                reason != CaptureEnd::Finished
            );
        }
    }

    #[test]
    fn browser_ids_ticks_and_gap_sequences_remain_exact_beyond_javascript_integers() {
        let big = (1u64 << 53) + 1;
        let mut rows = browser_fixture(&[Cohort::Evolving], CaptureEnd::Snapshot);
        rows[1] = serde_json::json!({"kind":"gap","data":{
            "cohort":"evolving", "first_sequence":"0", "last_sequence":(big - 1).to_string()
        }});
        rows.insert(
            2,
            serde_json::json!({"kind":"event","data":{
                "cohort":"evolving", "sequence":big.to_string(), "tick":big.to_string(),
                "event":{
                    "kind":"species_origin", "species_id":4,
                    "founder_birth_id":(u64::MAX - 1).to_string(),
                    "parent_a":{"status":"observed","birth_id":big.to_string(),"species_id":null},
                    "parent_b":{"status":"unavailable"}
                }
            }}),
        );
        rows[3]["data"]["sequence"] = serde_json::json!((big + 1).to_string());
        rows[3]["data"]["tick"] = serde_json::json!((u64::MAX - 1).to_string());
        rows[3]["data"]["event"]["species_id"] = serde_json::json!(4);
        rows[4]["data"]["ticks"] = serde_json::json!(u64::MAX.to_string());
        rows[4]["data"]["cohorts"][0]["counts"] = serde_json::json!({
            "next_sequence":(big + 2).to_string(), "events":"2", "dropped_events":big.to_string(),
            "gaps":"1", "origins":"1", "extinctions":"1"
        });
        rows[4]["data"]["cohorts"][0]["history_complete"] = serde_json::json!(false);
        let summary = parse(Cursor::new(encode(&rows))).unwrap();
        assert_eq!(summary.ticks.0, u64::MAX);
        assert_eq!(summary.provenance.seed.0, u64::MAX);
        assert_eq!(summary.cohorts[0].counts.next_sequence.0, big + 2);
        assert_eq!(summary.cohorts[0].counts.dropped_events.0, big);
        for pointer in [
            "/2/data/sequence",
            "/2/data/tick",
            "/2/data/event/founder_birth_id",
            "/2/data/event/parent_a/birth_id",
            "/4/data/ticks",
        ] {
            for invalid in [
                serde_json::json!(big),
                serde_json::json!("01"),
                serde_json::json!("+1"),
                serde_json::json!("18446744073709551616"),
            ] {
                let mut value = serde_json::Value::Array(rows.clone());
                *value.pointer_mut(pointer).unwrap() = invalid;
                assert!(
                    parse(Cursor::new(encode(value.as_array().unwrap()))).is_err(),
                    "{pointer}"
                );
            }
        }
        let mut bad = rows.clone();
        bad[2]["data"]["event"]["founder_birth_id"] = serde_json::json!(u64::MAX.to_string());
        assert!(parse(Cursor::new(encode(&bad))).is_err());
        for field in ["birth_id", "species_id"] {
            let mut bad = rows.clone();
            bad[2]["data"]["event"]["parent_a"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(parse(Cursor::new(encode(&bad))).is_err());
        }
        let mut valid = rows;
        valid[2]["data"]["event"]["founder_birth_id"] = serde_json::Value::Null;
        valid[2]["data"]["event"]["parent_a"]["birth_id"] = serde_json::Value::Null;
        parse(Cursor::new(encode(&valid))).unwrap();
    }

    #[test]
    fn browser_line_limit_includes_the_newline() {
        let mut rows = browser_fixture(&[Cohort::Evolving], CaptureEnd::Snapshot);
        let current_length = serde_json::to_vec(&rows[0]).unwrap().len() + 1;
        let current_revision = rows[0]["data"]["provenance"]["source_revision"]
            .as_str()
            .unwrap()
            .len();
        let revision = "x".repeat(MAX_LINE_BYTES as usize - current_length + current_revision);
        for index in [0, 3] {
            rows[index]["data"]["provenance"]["source_revision"] = serde_json::json!(revision);
        }
        assert_eq!(
            serde_json::to_vec(&rows[0]).unwrap().len() + 1,
            MAX_LINE_BYTES as usize
        );
        parse(Cursor::new(encode(&rows))).unwrap();
        rows[0]["data"]["provenance"]["source_revision"] =
            serde_json::json!(format!("{revision}x"));
        let error = parse(Cursor::new(encode(&rows))).unwrap_err();
        assert!(error.to_string().contains("exceeds 1 MiB"), "{error}");
    }

    #[test]
    fn shared_interoperability_fixtures_parse_and_native_export_is_byte_identical() {
        let native = include_bytes!("../tests/fixtures/history-v1.ndjson");
        let root = parse(Cursor::new(include_bytes!(
            "../tests/fixtures/history-v2-root.ndjson"
        )))
        .unwrap();
        assert_eq!(root.capture_end, Some(CaptureEnd::Snapshot));
        assert_eq!(root.cohorts.len(), 1);
        assert_eq!(root.cohorts[0].cohort, Cohort::RandomControl);
        let prefix = parse(Cursor::new(include_bytes!(
            "../tests/fixtures/history-v2-prefix.ndjson"
        )))
        .unwrap();
        assert_eq!(prefix.capture_end, Some(CaptureEnd::StorageLimit));
        assert_eq!(prefix.cohorts[0].counts.dropped_events.0, (1u64 << 53) + 1);
        assert_eq!(prefix.cohorts[0].final_state_hash, None);

        let summary = parse(Cursor::new(native)).unwrap();
        let rows = records(native);
        let header: Header = serde_json::from_value(rows[0]["data"].clone()).unwrap();
        let run = RunHeader {
            sim_version: header.provenance.sim_version,
            source_revision: header.provenance.source_revision,
            phase: header.provenance.phase,
            seed: header.provenance.seed.0.to_string(),
            control: header.provenance.control,
            ticks: header.ticks.unwrap().0,
            founders: header.founders,
            sample_every: header.drain_every.unwrap().0,
            params: header.params,
            ..self::header()
        };
        let mut bytes = Vec::new();
        let mut capture = Capture::new(header.capacity_per_cohort, None).unwrap();
        for recorder in &mut capture.cohorts {
            record(recorder, origin(0, 0));
        }
        ArchiveWriter::new(&mut bytes, &run, header.capacity_per_cohort, None)
            .unwrap()
            .finish(
                &mut capture,
                &StateHashes {
                    evolving: summary.cohorts[0].final_state_hash.clone().unwrap(),
                    random_control: summary.cohorts[1].final_state_hash.clone().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(bytes, native);
    }

    #[test]
    fn fifo_overflow_exports_exact_ordered_gaps_and_completion() {
        let mut capture = Capture::new(1, None).unwrap();
        let mut bytes = Vec::new();
        let mut writer = ArchiveWriter::new(&mut bytes, &header(), 1, None).unwrap();
        for recorder in &mut capture.cohorts {
            for id in 0..4 {
                record(recorder, origin(0, id));
            }
        }
        writer.drain(&mut capture).unwrap();
        for recorder in &mut capture.cohorts {
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
        let mut capture = Capture::new(4, None).unwrap();
        record(
            &mut capture.cohorts[0],
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
        record(&mut capture.cohorts[1], origin(0, 2));
        record(
            &mut capture.cohorts[1],
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
        ArchiveWriter::new(&mut bytes, &run, 4, None)
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
        let mut capture = Capture::new(4, None).unwrap();
        for recorder in &mut capture.cohorts {
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
        ArchiveWriter::new(&mut bytes, &run, 4, None)
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
            for (birth_id, species_id) in
                [("0", Some(0)), ("5", Some(1)), ("7", Some(1)), ("7", None)]
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
        assert!(ArchiveWriter::new(output, &header(), 4, None).is_err());
        let output = FailingWriter {
            writes_left: usize::MAX,
            fail_flush: true,
        };
        let mut writer = ArchiveWriter::new(output, &header(), 4, None).unwrap();
        let mut capture = Capture::new(4, None).unwrap();
        record(&mut capture.cohorts[0], origin(0, 0));
        assert!(writer.drain(&mut capture).is_err());
        let output = FailingWriter {
            writes_left: usize::MAX,
            fail_flush: false,
        };
        let mut writer = ArchiveWriter::new(output, &header(), 4, None).unwrap();
        writer.output.writes_left = 0;
        assert!(
            writer
                .finish(&mut Capture::new(4, None).unwrap(), &hashes())
                .is_err()
        );
    }

    fn neurons(count: u32) -> Vec<Gene> {
        (1..=count)
            .map(|id| {
                Gene::Neuron(sim_core::genome::NeuronGene {
                    id: sim_core::ids::InnovationId::new(id),
                    tau: 1.0,
                    ..Default::default()
                })
            })
            .collect()
    }

    fn representative_archive(staging: u32, genomes: &[Vec<Gene>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut capture = Capture::new(8, Some(staging)).unwrap();
        for cohort in &mut capture.cohorts {
            for (id, genes) in genomes.iter().enumerate() {
                cohort.record(origin(0, id as u32), Some(genes));
            }
            cohort.record(
                Event {
                    tick: 9,
                    kind: EventKind::SpeciesExtinct {
                        species_id: SpeciesId::new(0),
                    },
                },
                None,
            );
        }
        let mut run = header();
        run.founders = genomes.len() as u32;
        ArchiveWriter::new(&mut bytes, &run, 8, Some(1024))
            .unwrap()
            .finish(&mut capture, &hashes())
            .unwrap();
        bytes
    }

    #[test]
    fn version_three_archives_representatives_or_says_why_not() {
        // The second genome no longer fits once the first is staged.
        let genomes = [neurons(700), neurons(400), neurons(3)];
        let bytes = representative_archive(1024, &genomes);
        let rows = records(&bytes);
        assert_eq!(rows[0]["data"]["schema_version"], 3);
        assert_eq!(rows[0]["data"]["representative_genes"], 1024);
        let origins: Vec<_> = rows
            .iter()
            .filter(|row| row["data"]["event"]["kind"] == "species_origin")
            .collect();
        assert_eq!(origins.len(), 6);
        for cohort in origins.chunks(3) {
            let recorded: Vec<Gene> =
                serde_json::from_value(cohort[0]["data"]["representative"]["genes"].clone())
                    .unwrap();
            assert_eq!(recorded, genomes[0], "archived genes are exact");
            assert_eq!(
                cohort[1]["data"]["representative"],
                serde_json::json!({"status": "unavailable", "reason": "capture_pressure"})
            );
            assert_eq!(
                cohort[2]["data"]["representative"]["genes"]
                    .as_array()
                    .unwrap()
                    .len(),
                3
            );
        }
        assert!(
            rows.iter()
                .filter(|row| row["data"]["event"]["kind"] == "species_extinct")
                .all(|row| row["data"].get("representative").is_none())
        );
        let summary = parse(Cursor::new(&bytes)).unwrap();
        assert_eq!(summary.schema_version, 3);
        for cohort in &summary.cohorts {
            assert_eq!(cohort.counts.representatives, Some(Decimal(2)));
            assert_eq!(cohort.counts.unavailable_representatives, Some(Decimal(1)));
            assert!(
                cohort.history_complete,
                "an unavailable genome is not an event gap"
            );
        }
        let mut human = Vec::new();
        write_summary(&mut human, &summary).unwrap();
        assert!(
            String::from_utf8(human)
                .unwrap()
                .contains("0 gaps; 2 representatives archived, 1 unavailable; hash")
        );
    }

    #[test]
    fn genomes_too_large_for_one_line_are_unavailable_not_truncated() {
        let mut bytes = Vec::new();
        let mut capture = Capture::new(2, Some(20_000)).unwrap();
        for cohort in &mut capture.cohorts {
            cohort.record(origin(0, 0), Some(&neurons(15_000)));
        }
        let mut run = header();
        run.ticks = 1;
        ArchiveWriter::new(&mut bytes, &run, 2, Some(1024))
            .unwrap()
            .finish(&mut capture, &hashes())
            .unwrap();
        let rows = records(&bytes);
        assert_eq!(
            rows[1]["data"]["representative"],
            serde_json::json!({"status": "unavailable", "reason": "line_limit"})
        );
        let summary = parse(Cursor::new(&bytes)).unwrap();
        assert_eq!(
            summary.cohorts[0].counts.unavailable_representatives,
            Some(Decimal(1))
        );
    }

    #[test]
    fn representatives_are_required_exactly_on_version_three_origins() {
        let valid = records(&representative_archive(4096, &[neurons(3), neurons(2)]));
        parse(Cursor::new(encode(&valid))).unwrap();
        let origin_row = valid
            .iter()
            .position(|row| row["data"]["event"]["kind"] == "species_origin")
            .unwrap();
        let extinct_row = valid
            .iter()
            .position(|row| row["data"]["event"]["kind"] == "species_extinct")
            .unwrap();
        let mut unsorted = neurons(3);
        unsorted.swap(0, 2);
        type Mutation = Box<dyn Fn(&mut Vec<serde_json::Value>)>;
        let cases: Vec<(&str, Mutation)> = vec![
            (
                "missing on an origin",
                Box::new(move |rows| {
                    rows[origin_row]["data"]
                        .as_object_mut()
                        .unwrap()
                        .remove("representative");
                }),
            ),
            (
                "present on an extinction",
                Box::new(move |rows| {
                    rows[extinct_row]["data"]["representative"] =
                        serde_json::json!({"status": "unavailable", "reason": "capture_pressure"});
                }),
            ),
            (
                "invalid genome",
                Box::new(move |rows| {
                    rows[origin_row]["data"]["representative"]["genes"] =
                        serde_json::to_value(&unsorted).unwrap();
                }),
            ),
            (
                "larger than the genome limit",
                Box::new(move |rows| {
                    rows[origin_row]["data"]["representative"]["genes"] =
                        serde_json::to_value(neurons(1025)).unwrap();
                }),
            ),
            (
                "unknown unavailable reason",
                Box::new(move |rows| {
                    rows[origin_row]["data"]["representative"] =
                        serde_json::json!({"status": "unavailable", "reason": "guessed"});
                }),
            ),
            (
                "null representative",
                Box::new(move |rows| {
                    rows[origin_row]["data"]["representative"] = serde_json::Value::Null;
                }),
            ),
            (
                "staging below one maximum-size genome",
                Box::new(|rows| rows[0]["data"]["representative_genes"] = 1023.into()),
            ),
            (
                "version three without staging",
                Box::new(|rows| {
                    rows[0]["data"]
                        .as_object_mut()
                        .unwrap()
                        .remove("representative_genes");
                }),
            ),
            (
                "completion undercounts representatives",
                Box::new(|rows| {
                    let last = rows.len() - 1;
                    rows[last]["data"]["cohorts"][0]["counts"]["representatives"] = "1".into();
                }),
            ),
            (
                "completion omits representative counts",
                Box::new(|rows| {
                    let last = rows.len() - 1;
                    rows[last]["data"]["cohorts"][0]["counts"]
                        .as_object_mut()
                        .unwrap()
                        .remove("unavailable_representatives");
                }),
            ),
        ];
        for (name, mutate) in cases {
            let mut rows = valid.clone();
            mutate(&mut rows);
            assert!(
                parse(Cursor::new(encode(&rows))).is_err(),
                "accepted {name}"
            );
        }

        let mut version_one = records(&fixture());
        version_one[1]["data"]["representative"] =
            serde_json::json!({"status": "unavailable", "reason": "capture_pressure"});
        assert!(parse(Cursor::new(encode(&version_one))).is_err());
        let mut version_one = records(&fixture());
        version_one[0]["data"]["representative_genes"] = 4096.into();
        assert!(parse(Cursor::new(encode(&version_one))).is_err());
    }

    #[test]
    fn browser_shaped_version_three_keeps_version_two_rules() {
        let mut rows = records(include_bytes!("../tests/fixtures/history-v2-root.ndjson"));
        let last = rows.len() - 1;
        let mut archived = 0;
        for row in &mut rows {
            match row["kind"].as_str().unwrap() {
                "header" => {
                    row["data"]["schema_version"] = 3.into();
                    row["data"]["representative_genes"] = 1024.into();
                }
                "event" if row["data"]["event"]["kind"] == "species_origin" => {
                    row["data"]["representative"] = serde_json::json!({
                        "status": "recorded",
                        "genes": serde_json::to_value(neurons(2)).unwrap(),
                    });
                    archived += 1;
                }
                _ => {}
            }
        }
        rows[last]["data"]["schema_version"] = 3.into();
        for cohort in rows[last]["data"]["cohorts"].as_array_mut().unwrap() {
            cohort["counts"]["representatives"] = archived.to_string().into();
            cohort["counts"]["unavailable_representatives"] = "0".into();
        }
        assert!(archived > 0);
        let summary = parse(Cursor::new(encode(&rows))).unwrap();
        assert!(summary.run_id.is_some() && summary.capture_end.is_some());
        rows[last]["data"]
            .as_object_mut()
            .unwrap()
            .remove("capture_end");
        assert!(
            parse(Cursor::new(encode(&rows))).is_err(),
            "browser-shaped archives still need a capture end"
        );
    }

    #[test]
    fn export_rejects_staging_that_cannot_hold_one_valid_genome() {
        let run = header();
        assert!(validate_export(&run, 4, Some(1023)).is_err());
        validate_export(&run, 4, Some(1024)).unwrap();
        validate_export(&run, 4, None).unwrap();
    }

    #[test]
    fn archived_representatives_equal_the_classifiers_stored_copy() {
        let mut params = SimParams::default();
        params.world.max_agents = 8;
        params.plants.max_plants = 1;
        params.species.threshold = 1e-12;
        params.brain.weight_init_scale = 0.5;
        let mut world = sim_core::world::World::new(3, params.clone()).unwrap();
        let mut capture = Capture::new(8, Some(65_536)).unwrap();
        let cohort = &mut capture.cohorts[0];
        assert_eq!(
            world.seed_founders_with_history_observer(
                3,
                |_| {},
                |_| {},
                |event, genome| cohort.record(event, genome)
            ),
            3
        );
        let mut run = header();
        run.params = params;
        run.founders = 3;
        let mut bytes = Vec::new();
        ArchiveWriter::new(&mut bytes, &run, 8, Some(65_536))
            .unwrap()
            .drain(&mut capture)
            .unwrap();
        let rows = records(&bytes);
        let origins: Vec<_> = rows[1..]
            .iter()
            .filter(|row| row["data"]["cohort"] == "evolving")
            .collect();
        assert!(
            world.species_count() > 1,
            "distinct founders form distinct species"
        );
        assert_eq!(origins.len() as u32, world.species_count());
        for row in origins {
            let id = SpeciesId::new(row["data"]["event"]["species_id"].as_u64().unwrap() as u32);
            let archived: Vec<Gene> =
                serde_json::from_value(row["data"]["representative"]["genes"].clone()).unwrap();
            assert_eq!(Some(&archived[..]), world.species().representative(id));
        }
    }
}
