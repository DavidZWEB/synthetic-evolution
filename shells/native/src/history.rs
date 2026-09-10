//! Shell-owned bounded capture and streaming species-history export.
//!
//! Callbacks only enqueue preallocated records. Draining and all I/O happen outside
//! World; this is neither a complete genealogy nor a checkpoint.

use std::io::{self, Write};

use sim_core::history::{Event, Record, Recorder, SequenceExhausted};

use crate::Result;
use crate::cli::HistoryArgs;
use crate::history_wire::{
    ArchiveRecord, Cohort, CohortCompletion, Completion, Counts, Decimal, EventRecord, Header,
    MAX_LINE_BYTES, SCHEMA_VERSION,
};
use crate::metrics::{RunHeader, StateHashes};

#[cfg(test)]
mod tests;

pub(crate) struct Capture {
    pub recorders: [Recorder; 2],
}

impl Capture {
    pub fn new(capacity: u32) -> Result<Self> {
        Ok(Self {
            recorders: [Recorder::try_new(capacity)?, Recorder::try_new(capacity)?],
        })
    }

    pub fn check(&self) -> Result<()> {
        if self.recorders.iter().any(Recorder::sequence_exhausted) {
            return Err(SequenceExhausted.into());
        }
        Ok(())
    }
}

pub(crate) fn record(recorder: &mut Recorder, event: Event) {
    // Recorder latches exhaustion. The shell reports it after stepping, never
    // panicking or doing I/O from a callback in the tick.
    let _ = recorder.record(event);
}

pub(crate) struct ArchiveWriter<W: Write> {
    output: W,
    header: Header,
    counts: [Counts; 2],
}

impl<W: Write> ArchiveWriter<W> {
    pub fn new(mut output: W, run: &RunHeader, capacity: u32) -> Result<Self> {
        let header = Header::new(run, capacity)?;
        write_record(
            &mut output,
            &ArchiveRecord::Header(Box::new(header.clone())),
        )?;
        Ok(Self {
            output,
            header,
            counts: Default::default(),
        })
    }

    pub fn drain(&mut self, capture: &mut Capture) -> Result<()> {
        capture.check()?;
        for cohort in Cohort::ALL {
            let counts = &mut self.counts[cohort.index()];
            while let Some(record) = capture.recorders[cohort.index()].pop() {
                let record = match record {
                    Record::Event { sequence, event } => {
                        counts.next_sequence = Decimal(sequence + 1);
                        counts.events.0 += 1;
                        match event.kind.into() {
                            EventRecord::SpeciesOrigin { .. } => counts.origins.0 += 1,
                            EventRecord::SpeciesExtinct { .. } => counts.extinctions.0 += 1,
                        }
                        ArchiveRecord::Event {
                            cohort,
                            sequence: Decimal(sequence),
                            tick: Decimal(event.tick),
                            event: event.kind.into(),
                        }
                    }
                    Record::Gap {
                        first_sequence,
                        last_sequence,
                    } => {
                        counts.next_sequence = Decimal(last_sequence + 1);
                        counts.dropped_events.0 += last_sequence - first_sequence + 1;
                        counts.gaps.0 += 1;
                        ArchiveRecord::Gap {
                            cohort,
                            first_sequence: Decimal(first_sequence),
                            last_sequence: Decimal(last_sequence),
                        }
                    }
                };
                write_record(&mut self.output, &record)?;
            }
        }
        self.output.flush()?;
        Ok(())
    }

    pub fn finish(mut self, capture: &mut Capture, hashes: &StateHashes) -> Result<()> {
        self.drain(capture)?;
        let cohorts = Cohort::ALL.map(|cohort| {
            let counts = self.counts[cohort.index()].clone();
            debug_assert_eq!(
                counts.next_sequence.0,
                capture.recorders[cohort.index()].next_sequence()
            );
            debug_assert_eq!(
                counts.dropped_events.0,
                capture.recorders[cohort.index()].dropped_events()
            );
            CohortCompletion {
                cohort,
                history_complete: counts.dropped_events.0 == 0,
                counts,
                final_state_hash: match cohort {
                    Cohort::Evolving => hashes.evolving.clone(),
                    Cohort::RandomControl => hashes.random_control.clone(),
                },
            }
        });
        write_record(
            &mut self.output,
            &ArchiveRecord::Complete(Completion {
                schema_version: SCHEMA_VERSION,
                provenance: self.header.provenance,
                ticks: self.header.ticks,
                cohorts,
            }),
        )?;
        self.output.flush()?;
        Ok(())
    }
}

pub(crate) fn validate_export(run: &RunHeader, capacity: u32) -> Result<()> {
    let header = Header::new(run, capacity)?;
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
        writeln!(
            output,
            "completed {} ticks; species history schema {}",
            summary.ticks.0, summary.schema_version
        )?;
        for cohort in summary.cohorts {
            writeln!(
                output,
                "{:?}: {} origins, {} extinctions retained; {} dropped events in {} gaps; hash {}",
                cohort.cohort,
                cohort.counts.origins.0,
                cohort.counts.extinctions.0,
                cohort.counts.dropped_events.0,
                cohort.counts.gaps.0,
                cohort.final_state_hash,
            )?;
        }
    }
    output.flush()?;
    Ok(())
}
