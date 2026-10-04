//! Streaming validation of species-history archives and explicit capture prefixes.
//!
//! Retains counters and at most the configured active species per cohort, not an
//! archive or genealogy. Gaps explicitly limit which lifecycle links can be checked.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;
use sim_core::ids::{BirthId, SpeciesId};

use crate::Result;
use crate::history_wire::{
    ArchiveRecord, BROWSER_SCHEMA_VERSION, CaptureEnd, Cohort, Completion, Counts, Decimal,
    EventRecord, Header, MAX_LINE_BYTES, ParentRecord, REPRESENTATIVE_SCHEMA_VERSION,
    RESUMED_SCHEMA_VERSION, RepresentativeRecord, SCHEMA_VERSION,
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

pub(crate) fn parse(input: impl BufRead) -> Result<Completion> {
    parse_archive(input).map(|(_, completion)| completion)
}

/// Validates an archive and returns its header alongside the completion.
pub(crate) fn parse_archive(mut input: impl BufRead) -> Result<(Header, Completion)> {
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
                ArchiveRecord::Header(mut next) => {
                    if header.is_some() {
                        return Err(invalid("duplicate history header").into());
                    }
                    restore_later_params(&mut next.params, &line)?;
                    validate_header(&next, &line)?;
                    for state in &mut states {
                        if next.representative_genes.is_some() {
                            state.counts.representatives = Some(Decimal(0));
                            state.counts.unavailable_representatives = Some(Decimal(0));
                        }
                        // A resumed segment cannot see lineage before its first tick,
                        // which is exactly what a gap means (spec §7.10).
                        state.has_gap = next.resumed_from_tick.is_some();
                    }
                    header = Some(next);
                }
                ArchiveRecord::Event {
                    cohort,
                    sequence,
                    tick,
                    event,
                    representative,
                } => {
                    let header = header
                        .as_ref()
                        .ok_or_else(|| invalid("history header must be first"))?;
                    validate_cohort(header, cohort)?;
                    let state = &mut states[cohort.index()];
                    state.representative(header, &event, representative.as_ref())?;
                    state.event(header, sequence.0, tick.0, event)?;
                }
                ArchiveRecord::Gap {
                    cohort,
                    first_sequence,
                    last_sequence,
                } => {
                    let header = header
                        .as_ref()
                        .ok_or_else(|| invalid("history header must be first"))?;
                    validate_cohort(header, cohort)?;
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
    match (header, completion) {
        (Some(header), Some(completion)) => Ok((*header, completion)),
        _ => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "history archive has no completion marker",
        )
        .into()),
    }
}

/// Version three keeps either earlier shape: native files name no run, browser files do.
fn native_shape(header: &Header) -> bool {
    match header.schema_version {
        SCHEMA_VERSION => true,
        REPRESENTATIVE_SCHEMA_VERSION | RESUMED_SCHEMA_VERSION => header.run_id.is_none(),
        _ => false,
    }
}

fn validate_header(header: &Header, line: &[u8]) -> Result<()> {
    let representatives = header.schema_version == REPRESENTATIVE_SCHEMA_VERSION;
    let resumed = header.schema_version == RESUMED_SCHEMA_VERSION;
    let shape = match header.schema_version {
        SCHEMA_VERSION
        | BROWSER_SCHEMA_VERSION
        | REPRESENTATIVE_SCHEMA_VERSION
        | RESUMED_SCHEMA_VERSION => {
            if native_shape(header) {
                header.run_id.is_none()
                    && header.cohorts == Cohort::ALL
                    && header.ticks.is_some()
                    && header.drain_every.is_some()
            } else {
                header
                    .run_id
                    .as_ref()
                    .is_some_and(|id| !id.is_empty() && id.chars().count() <= 128)
                    && (header.cohorts == Cohort::ALL
                        || header.cohorts == [Cohort::Evolving]
                        || header.cohorts == [Cohort::RandomControl])
            }
        }
        _ => false,
    };
    // Version four may carry representatives or not; version three always does.
    let staging = match header.representative_genes {
        None => !representatives,
        Some(genes) => {
            (representatives || resumed)
                && genes >= header.params.storage.max_genes
                && u64::from(genes) * size_of::<sim_core::genome::Gene>() as u64 <= i32::MAX as u64
        }
    };
    let resume = match header.resumed_from_tick {
        None => !resumed,
        Some(tick) => resumed && header.ticks.is_none_or(|end| tick.0 <= end.0),
    };
    let supported_schema = shape && staging && resume;
    if !supported_schema
        || header.provenance.phase != 2
        || header.provenance.control != RANDOMIZED_AT_BIRTH_PROTOCOL
    {
        return Err(invalid("unsupported history schema/phase/control protocol").into());
    }
    if header.provenance.sim_version.trim().is_empty()
        || header.provenance.source_revision.trim().is_empty()
        || header.founders == 0
        || header.founders > header.params.world.max_agents
        || header.drain_every.is_some_and(|interval| interval.0 == 0)
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
    header.params.validate()?;
    Ok(())
}

fn validate_cohort(header: &Header, cohort: Cohort) -> Result<()> {
    if !header.cohorts.contains(&cohort) {
        return Err(invalid("history row cohort is absent from header").into());
    }
    Ok(())
}

/// Params fields added after archives were first written. An archive written before
/// one existed ran without it, so its value is zero, not today's default.
const LATER_PARAMS: &[&str] = &[
    "add_oscillator_rate",
    "grazing_lag",
    "patchiness",
    "patch_scale",
];

/// Serde fills an omitted later field from today's default; reset it to the zero the
/// archived run actually used.
fn restore_later_params(params: &mut sim_core::SimParams, line: &[u8]) -> Result<()> {
    let wire: serde_json::Value = serde_json::from_slice(line)?;
    let absent = |path: &str| wire.pointer(path).is_none();
    if absent("/data/params/mutation/structural/add_oscillator_rate") {
        params.mutation.structural.add_oscillator_rate = 0.0;
    }
    if absent("/data/params/plants/grazing_lag") {
        params.plants.grazing_lag = 0.0;
    }
    if absent("/data/params/plants/patchiness") {
        params.plants.patchiness = 0.0;
    }
    if absent("/data/params/plants/patch_scale") {
        params.plants.patch_scale = 0.0;
    }
    Ok(())
}

fn require_fields(actual: &serde_json::Value, complete: &serde_json::Value) -> Result<()> {
    if let Some(fields) = complete.as_object() {
        let actual = actual
            .as_object()
            .ok_or_else(|| invalid("history requires complete params"))?;
        for (name, value) in fields {
            let Some(actual) = actual.get(name) else {
                if LATER_PARAMS.contains(&name.as_str()) {
                    continue;
                }
                return Err(invalid(format!("history params missing {name}")).into());
            };
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
    active: BTreeMap<SpeciesId, BirthId>,
    has_gap: bool,
    has_nonseeding_event: bool,
}

impl CohortState {
    /// Version three requires every origin to archive its representative or say why
    /// it could not; earlier versions and extinctions carry none.
    fn representative(
        &mut self,
        header: &Header,
        event: &EventRecord,
        representative: Option<&RepresentativeRecord>,
    ) -> Result<()> {
        let expected = header.representative_genes.is_some()
            && matches!(event, EventRecord::SpeciesOrigin { .. });
        match representative {
            None if !expected => Ok(()),
            Some(RepresentativeRecord::Recorded { genes }) if expected => {
                if genes.len() > header.params.storage.max_genes as usize
                    || sim_core::genome::validate(genes).is_err()
                {
                    return Err(invalid("archived representative is not a valid genome").into());
                }
                increment(&mut self.counts.representatives);
                Ok(())
            }
            Some(RepresentativeRecord::Unavailable { .. }) if expected => {
                increment(&mut self.counts.unavailable_representatives);
                Ok(())
            }
            _ => Err(invalid(
                "representatives must accompany every version-three origin and nothing else",
            )
            .into()),
        }
    }

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
        // A resumed segment has no seeding tick: its events fall strictly inside it.
        let outside = match header.resumed_from_tick {
            Some(start) => tick < start.0 || header.ticks.is_some_and(|end| tick >= end.0),
            None => header
                .ticks
                .is_some_and(|end| tick > end.0.saturating_sub(1)),
        };
        if outside || self.last_tick.is_some_and(|last| tick < last) {
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
                        birth_id,
                        species_id,
                    } = parent
                    {
                        if let Some(id) = species_id {
                            if let Some(origin_birth) = self.active.get(&id) {
                                if !birth_id.is_null() && birth_id < *origin_birth {
                                    return Err(invalid(
                                        "observed parent predates its species origin",
                                    )
                                    .into());
                                }
                            } else if !self.has_gap {
                                return Err(invalid(
                                    "observed parent species has no active origin",
                                )
                                .into());
                            }
                        }
                        if !birth_id.is_null()
                            && self.active.iter().any(|(id, origin_birth)| {
                                *origin_birth == birth_id && Some(*id) != species_id
                            })
                        {
                            return Err(invalid(
                                "observed parent contradicts its recorded species origin",
                            )
                            .into());
                        }
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
                let has_parent =
                    parent_a != (ParentRecord::Absent {}) || parent_b != (ParentRecord::Absent {});
                if header.ticks == Some(Decimal(0)) && has_parent {
                    return Err(invalid("a seeding-only run cannot have parental origins").into());
                }
                // BirthIds start at zero and refused admissions consume no ID (spec §3.4).
                let seeding_founder = !founder_birth_id.is_null()
                    && founder_birth_id.raw() < u64::from(header.founders);
                if header.ticks == Some(Decimal(0)) && !seeding_founder {
                    return Err(invalid(
                        "a seeding-only origin requires an available birth ID below the founder count",
                    )
                    .into());
                }
                self.has_nonseeding_event |= has_parent || !seeding_founder;
                self.greatest_species = Some(species_id);
                if !founder_birth_id.is_null() {
                    self.last_founder = Some(founder_birth_id);
                } else {
                    self.birth_ids_exhausted = true;
                }
                self.active.insert(species_id, founder_birth_id);
                if self.active.len() > header.params.species.capacity as usize {
                    return Err(
                        invalid("observed active species exceed classification capacity").into(),
                    );
                }
                self.counts.origins.0 += 1;
            }
            EventRecord::SpeciesExtinct { species_id } => {
                if species_id.is_null() || header.ticks == Some(Decimal(0)) {
                    return Err(invalid("invalid species extinction").into());
                }
                if self.active.remove(&species_id).is_none() && !self.has_gap {
                    return Err(invalid("extinction without an active observed origin").into());
                }
                self.greatest_species = Some(
                    self.greatest_species
                        .map_or(species_id, |last| last.max(species_id)),
                );
                self.counts.extinctions.0 += 1;
                self.has_nonseeding_event = true;
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

fn increment(count: &mut Option<Decimal>) {
    if let Some(count) = count {
        count.0 += 1;
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
        || completion.run_id != header.run_id
    {
        return Err(invalid("history completion provenance does not match header").into());
    }
    let incomplete = match (native_shape(header), completion.capture_end) {
        (true, None) if header.ticks == Some(completion.ticks) => false,
        (false, Some(reason))
            if header
                .ticks
                .is_none_or(|planned| completion.ticks.0 <= planned.0)
                && (reason != CaptureEnd::Finished || header.ticks == Some(completion.ticks)) =>
        {
            reason.is_incomplete()
        }
        _ => return Err(invalid("invalid history capture end reason or tick boundary").into()),
    };
    if completion.cohorts.len() != header.cohorts.len() {
        return Err(invalid("history completion cohorts do not match header").into());
    }
    for (&cohort, row) in header.cohorts.iter().zip(&completion.cohorts) {
        let state = &states[cohort.index()];
        let counts = &state.counts;
        let valid_hash = match &row.final_state_hash {
            Some(hash) => {
                hash.len() == 16
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
            None => incomplete,
        };
        if state
            .last_tick
            .is_some_and(|tick| tick > completion.ticks.0.saturating_sub(1))
            || (completion.ticks.0 == 0 && state.has_nonseeding_event)
        {
            return Err(invalid("history events are outside the captured tick boundary").into());
        }
        if row.cohort != cohort
            // Seeding always records origins; a resumed segment may be quiet.
            || (!incomplete
                && header.resumed_from_tick.is_none()
                && header.params.species.capacity > 0
                && counts.next_sequence.0 == 0)
            || row.counts != *counts
            || row.history_complete != (!incomplete && counts.dropped_events.0 == 0)
            || !valid_hash
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_without_a_later_param_read_it_as_the_zero_they_ran() {
        for (fixture, present) in [
            (
                &include_bytes!("../tests/fixtures/history-v2-root.ndjson")[..],
                false,
            ),
            (
                &include_bytes!("../tests/fixtures/history-v1.ndjson")[..],
                true,
            ),
        ] {
            let line = fixture
                .split_inclusive(|&byte| byte == b'\n')
                .next()
                .unwrap();
            let ArchiveRecord::Header(mut header) = serde_json::from_slice(line).unwrap() else {
                panic!("first record is the header");
            };
            if !present {
                assert_ne!(
                    header.params.mutation.structural.add_oscillator_rate, 0.0,
                    "serde filled today's nonzero default"
                );
                // Grazing lag still defaults to zero; stand in for the nonzero default
                // serde will fill once calibration ships one.
                header.params.plants.grazing_lag = 0.5;
                header.params.plants.patchiness = 2.0;
                assert_ne!(header.params.plants.patch_scale, 0.0);
            }
            restore_later_params(&mut header.params, line).unwrap();
            assert_eq!(header.params.mutation.structural.add_oscillator_rate, 0.0);
            assert_eq!(header.params.plants.grazing_lag, 0.0);
            assert_eq!(header.params.plants.patchiness, 0.0);
            if !present {
                assert_eq!(header.params.plants.patch_scale, 0.0);
            }
        }
    }
}
