//! Version-one species-history wire records, independent of metrics and core serde.
//!
//! Identities and counters remain exact in JSON consumers; origins describe only
//! the founding admission, not the ancestry of every member of a species.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sim_core::history::{EventKind, Parent};
use sim_core::ids::{BirthId, SpeciesId};
use sim_core::params::SimParams;

use crate::metrics::RunHeader;

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_LINE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Decimal(pub u64);

impl Serialize for Decimal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty()
            || (text.len() > 1 && text.starts_with('0'))
            || !text.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(de::Error::custom("expected a canonical decimal u64 string"));
        }
        text.parse().map(Self).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Cohort {
    Evolving,
    RandomControl,
}

impl Cohort {
    pub const ALL: [Self; 2] = [Self::Evolving, Self::RandomControl];

    pub fn index(self) -> usize {
        match self {
            Self::Evolving => 0,
            Self::RandomControl => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    pub sim_version: String,
    pub source_revision: String,
    pub phase: u32,
    pub seed: Decimal,
    pub control: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    pub schema_version: u32,
    pub provenance: Provenance,
    pub cohorts: [Cohort; 2],
    pub ticks: Decimal,
    pub founders: u32,
    pub drain_every: Decimal,
    pub capacity_per_cohort: u32,
    pub params: SimParams,
}

impl Header {
    pub fn new(run: &RunHeader, capacity: u32) -> crate::Result<Self> {
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            provenance: Provenance {
                sim_version: run.sim_version.clone(),
                source_revision: run.source_revision.clone(),
                phase: run.phase,
                seed: Decimal(run.seed.parse()?),
                control: run.control.clone(),
            },
            cohorts: Cohort::ALL,
            ticks: Decimal(run.ticks),
            founders: run.founders,
            drain_every: Decimal(run.sample_every),
            capacity_per_cohort: capacity,
            params: run.params.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ParentRecord {
    Absent {},
    Unavailable {},
    Observed {
        #[serde(deserialize_with = "required_birth_id")]
        birth_id: BirthId,
        // A required nullable field: omitted metadata must not imply unclassified.
        #[serde(deserialize_with = "required_option")]
        species_id: Option<SpeciesId>,
    },
}

fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

fn required_birth_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BirthId, D::Error> {
    BirthId::deserialize(deserializer)
}

impl From<Parent> for ParentRecord {
    fn from(parent: Parent) -> Self {
        match parent {
            Parent::Absent => Self::Absent {},
            Parent::Unavailable => Self::Unavailable {},
            Parent::Observed {
                birth_id,
                species_id,
            } => Self::Observed {
                birth_id,
                species_id,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EventRecord {
    SpeciesOrigin {
        species_id: SpeciesId,
        #[serde(deserialize_with = "required_birth_id")]
        founder_birth_id: BirthId,
        parent_a: ParentRecord,
        parent_b: ParentRecord,
    },
    SpeciesExtinct {
        species_id: SpeciesId,
    },
}

impl From<EventKind> for EventRecord {
    fn from(event: EventKind) -> Self {
        match event {
            EventKind::SpeciesOrigin {
                species_id,
                founder_birth_id,
                parent_a,
                parent_b,
            } => Self::SpeciesOrigin {
                species_id,
                founder_birth_id,
                parent_a: parent_a.into(),
                parent_b: parent_b.into(),
            },
            EventKind::SpeciesExtinct { species_id } => Self::SpeciesExtinct { species_id },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Counts {
    pub next_sequence: Decimal,
    pub events: Decimal,
    pub dropped_events: Decimal,
    pub gaps: Decimal,
    pub origins: Decimal,
    pub extinctions: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohortCompletion {
    pub cohort: Cohort,
    pub counts: Counts,
    pub history_complete: bool,
    pub final_state_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    pub schema_version: u32,
    pub provenance: Provenance,
    pub ticks: Decimal,
    pub cohorts: [CohortCompletion; 2],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum ArchiveRecord {
    Header(Box<Header>),
    Event {
        cohort: Cohort,
        sequence: Decimal,
        tick: Decimal,
        event: EventRecord,
    },
    Gap {
        cohort: Cohort,
        first_sequence: Decimal,
        last_sequence: Decimal,
    },
    Complete(Completion),
}
