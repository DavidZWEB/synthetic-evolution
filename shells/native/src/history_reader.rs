//! Streaming validation of completed version-one species-history archives.
//!
//! Retains counters and at most the configured active species per cohort, not an
//! archive or genealogy. Gaps explicitly limit which lifecycle links can be checked.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use sim_core::LayoutEra;
use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;
use sim_core::ids::{BirthId, SpeciesId};

use crate::Result;
use crate::history_wire::{
    ArchiveRecord, Cohort, Completion, Counts, Decimal, EventRecord, Header, MAX_LINE_BYTES,
    ParentRecord, SCHEMA_VERSION,
};

pub(crate) fn read(path: &Path) -> Result<Completion> {
    if path == Path::new("-") {
        parse(BufReader::new(io::stdin().lock()))
    } else {
        let input = File::open(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not open history {}: {error}", path.display()),
            )
        })?;
        parse(BufReader::new(input))
    }
}

pub(crate) fn parse(mut input: impl BufRead) -> Result<Completion> {
    let mut header = None;
    let mut completion = None;
    let mut states: [CohortState; 2] = Default::default();
    let mut line = Vec::new();
    let mut line_number = 0u64;
    loop {
        line.clear();
        let bytes = (&mut input)
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        line_number += 1;
        let result = (|| -> Result<()> {
            if bytes as u64 > MAX_LINE_BYTES {
                return Err(invalid("history record exceeds 1 MiB").into());
            }
            if line.last() != Some(&b'\n') {
                return Err(invalid("truncated history record (missing newline)").into());
            }
            if completion.is_some() {
                return Err(invalid("record after history completion").into());
            }
            let record: ArchiveRecord = serde_json::from_slice(&line)?;
            match record {
                ArchiveRecord::Header(next) => {
                    if header.is_some() {
                        return Err(invalid("duplicate history header").into());
                    }
                    validate_header(&next, &line)?;
                    header = Some(next);
                }
                ArchiveRecord::Event {
                    cohort,
                    sequence,
                    tick,
                    event,
                } => {
                    let header = header
                        .as_ref()
                        .ok_or_else(|| invalid("history header must be first"))?;
                    states[cohort.index()].event(header, sequence.0, tick.0, event)?;
                }
                ArchiveRecord::Gap {
                    cohort,
                    first_sequence,
                    last_sequence,
                } => {
                    let header = header
                        .as_ref()
                        .ok_or_else(|| invalid("history header must be first"))?;
                    if header.params.species.capacity == 0 {
                        return Err(
                            invalid("disabled classification cannot have history gaps").into()
                        );
                    }
                    states[cohort.index()].gap(first_sequence.0, last_sequence.0)?;
                }
                ArchiveRecord::Complete(next) => {
                    let header = header
                        .as_ref()
                        .ok_or_else(|| invalid("history header must be first"))?;
                    validate_completion(header, &states, &next)?;
                    completion = Some(next);
                }
            }
            Ok(())
        })();
        result.map_err(|error| invalid(format!("history line {line_number}: {error}")))?;
    }
    completion.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "history archive has no completion marker",
        )
        .into()
    })
}

fn validate_header(header: &Header, line: &[u8]) -> Result<()> {
    if header.schema_version != SCHEMA_VERSION
        || header.cohorts != Cohort::ALL
        || header.provenance.phase != 2
        || header.provenance.control != RANDOMIZED_AT_BIRTH_PROTOCOL
    {
        return Err(invalid("unsupported history schema/phase/control protocol").into());
    }
    if header.provenance.sim_version.trim().is_empty()
        || header.provenance.source_revision.trim().is_empty()
        || header.founders == 0
        || header.founders > header.params.world.max_agents
        || header.drain_every.0 == 0
        || header.capacity_per_cohort == 0
        || u64::from(header.capacity_per_cohort)
            * size_of::<Option<sim_core::history::Record>>() as u64
            > i32::MAX as u64
    {
        return Err(invalid("invalid history run provenance or capture configuration").into());
    }
    // SimParams deliberately defaults partial CLI input. An archive instead
    // requires every field so future defaults cannot invent missing provenance.
    let wire: serde_json::Value = serde_json::from_slice(line)?;
    require_fields(
        &wire["data"]["params"],
        &serde_json::to_value(&header.params)?,
    )?;
    header
        .params
        .validate_for_layout(LayoutEra::BirthIdentities)?;
    Ok(())
}

fn require_fields(actual: &serde_json::Value, complete: &serde_json::Value) -> Result<()> {
    if let Some(fields) = complete.as_object() {
        let actual = actual
            .as_object()
            .ok_or_else(|| invalid("history requires complete params"))?;
        for (name, value) in fields {
            let actual = actual
                .get(name)
                .ok_or_else(|| invalid(format!("history params missing {name}")))?;
            require_fields(actual, value)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct CohortState {
    counts: Counts,
    last_tick: Option<u64>,
    greatest_species: Option<SpeciesId>,
    last_founder: Option<BirthId>,
    birth_ids_exhausted: bool,
    active: BTreeSet<SpeciesId>,
    has_gap: bool,
}

impl CohortState {
    fn event(
        &mut self,
        header: &Header,
        sequence: u64,
        tick: u64,
        event: EventRecord,
    ) -> Result<()> {
        if sequence != self.counts.next_sequence.0 || sequence == u64::MAX {
            return Err(invalid("history event sequence is not contiguous").into());
        }
        if tick > header.ticks.0.saturating_sub(1) || self.last_tick.is_some_and(|last| tick < last)
        {
            return Err(invalid("history event tick is out of order or outside the run").into());
        }
        if header.params.species.capacity == 0 {
            return Err(invalid("disabled classification cannot emit species history").into());
        }
        match event {
            EventRecord::SpeciesOrigin {
                species_id,
                founder_birth_id,
                parent_a,
                parent_b,
            } => {
                if species_id.is_null()
                    || self.greatest_species.is_some_and(|last| species_id <= last)
                    || (!founder_birth_id.is_null()
                        && (self.birth_ids_exhausted
                            || self
                                .last_founder
                                .is_some_and(|last| founder_birth_id <= last)))
                {
                    return Err(invalid("invalid or reused species origin identity").into());
                }
                for parent in [parent_a, parent_b] {
                    validate_parent(parent, species_id, founder_birth_id)?;
                    if let ParentRecord::Observed {
                        species_id: Some(id),
                        ..
                    } = parent
                        && !self.has_gap
                        && !self.active.contains(&id)
                    {
                        return Err(invalid("observed parent species has no active origin").into());
                    }
                }
                if let (
                    ParentRecord::Observed { birth_id: a, .. },
                    ParentRecord::Observed { birth_id: b, .. },
                ) = (parent_a, parent_b)
                    && !a.is_null()
                    && a == b
                {
                    return Err(invalid("origin names the same observed parent twice").into());
                }
                if header.ticks.0 == 0
                    && (parent_a != (ParentRecord::Absent {})
                        || parent_b != (ParentRecord::Absent {}))
                {
                    return Err(invalid("a seeding-only run cannot have parental origins").into());
                }
                self.greatest_species = Some(species_id);
                if !founder_birth_id.is_null() {
                    self.last_founder = Some(founder_birth_id);
                } else {
                    self.birth_ids_exhausted = true;
                }
                self.active.insert(species_id);
                if self.active.len() > header.params.species.capacity as usize {
                    return Err(
                        invalid("observed active species exceed classification capacity").into(),
                    );
                }
                self.counts.origins.0 += 1;
            }
            EventRecord::SpeciesExtinct { species_id } => {
                if species_id.is_null() || header.ticks.0 == 0 {
                    return Err(invalid("invalid species extinction").into());
                }
                if !self.active.remove(&species_id) && !self.has_gap {
                    return Err(invalid("extinction without an active observed origin").into());
                }
                self.greatest_species = Some(
                    self.greatest_species
                        .map_or(species_id, |last| last.max(species_id)),
                );
                self.counts.extinctions.0 += 1;
            }
        }
        self.counts.next_sequence = Decimal(sequence + 1);
        self.counts.events.0 += 1;
        self.last_tick = Some(tick);
        Ok(())
    }

    fn gap(&mut self, first: u64, last: u64) -> Result<()> {
        if first != self.counts.next_sequence.0 || last < first || last == u64::MAX {
            return Err(invalid("history gap range is invalid or not contiguous").into());
        }
        self.counts.next_sequence = Decimal(last + 1);
        self.counts.dropped_events.0 += last - first + 1;
        self.counts.gaps.0 += 1;
        self.has_gap = true;
        // Lost extinctions may have retired any observed representative; keeping
        // those as definitely active would reject legitimate post-gap origins.
        self.active.clear();
        Ok(())
    }
}

fn validate_parent(parent: ParentRecord, species: SpeciesId, founder: BirthId) -> Result<()> {
    if let ParentRecord::Observed {
        birth_id,
        species_id,
    } = parent
        && (species_id.is_some_and(|id| id.is_null() || id >= species)
            || (!founder.is_null() && (birth_id.is_null() || birth_id >= founder)))
    {
        return Err(invalid("invalid observed parent identity or species").into());
    }
    Ok(())
}

fn validate_completion(
    header: &Header,
    states: &[CohortState; 2],
    completion: &Completion,
) -> Result<()> {
    if completion.schema_version != header.schema_version
        || completion.provenance != header.provenance
        || completion.ticks != header.ticks
    {
        return Err(invalid("history completion provenance does not match header").into());
    }
    for cohort in Cohort::ALL {
        let row = &completion.cohorts[cohort.index()];
        let counts = &states[cohort.index()].counts;
        if row.cohort != cohort
            || (header.params.species.capacity > 0 && counts.next_sequence.0 == 0)
            || row.counts != *counts
            || row.history_complete != (counts.dropped_events.0 == 0)
            || row.final_state_hash.len() != 16
            || !row
                .final_state_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "invalid history completion cohort, totals, completeness, or hash",
            )
            .into());
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
