//! Versioned species-history archive records, independent of metrics and core serde.
//!
//! Native export remains version one; version two also describes browser capture
//! prefixes. Shared events describe founding admissions, not complete genealogy.

use serde::{Deserialize, Deserializer, Serialize};
use sim_core::params::SimParams;

use crate::metrics::RunHeader;

#[path = "../../shared/history_event_wire.rs"]
mod history_event_wire;
use history_event_wire::required_option;
pub(crate) use history_event_wire::{Decimal, EventRecord, ParentRecord};

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const BROWSER_SCHEMA_VERSION: u32 = 2;
pub(crate) const MAX_LINE_BYTES: u64 = 1024 * 1024;

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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub run_id: Option<String>,
    pub provenance: Provenance,
    pub cohorts: Vec<Cohort>,
    #[serde(deserialize_with = "required_option")]
    pub ticks: Option<Decimal>,
    pub founders: u32,
    #[serde(deserialize_with = "required_option")]
    pub drain_every: Option<Decimal>,
    pub capacity_per_cohort: u32,
    pub params: SimParams,
}

impl Header {
    pub fn new(run: &RunHeader, capacity: u32) -> crate::Result<Self> {
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            run_id: None,
            provenance: Provenance {
                sim_version: run.sim_version.clone(),
                source_revision: run.source_revision.clone(),
                phase: run.phase,
                seed: Decimal(run.seed.parse()?),
                control: run.control.clone(),
            },
            cohorts: Cohort::ALL.to_vec(),
            ticks: Some(Decimal(run.ticks)),
            founders: run.founders,
            drain_every: Some(Decimal(run.sample_every)),
            capacity_per_cohort: capacity,
            params: run.params.clone(),
        })
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
    #[serde(deserialize_with = "required_option")]
    pub final_state_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureEnd {
    Finished,
    Snapshot,
    Stopped,
    Reseeded,
    ParamsChanged,
    Unfinalized,
    StorageLimit,
    StorageError,
    CaptureError,
}

impl CaptureEnd {
    pub fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Unfinalized | Self::StorageLimit | Self::StorageError | Self::CaptureError
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Snapshot => "snapshot",
            Self::Stopped => "stopped",
            Self::Reseeded => "reseeded",
            Self::ParamsChanged => "params_changed",
            Self::Unfinalized => "unfinalized",
            Self::StorageLimit => "storage_limit",
            Self::StorageError => "storage_error",
            Self::CaptureError => "capture_error",
        }
    }
}

// Version-specific optional fields may be absent in v1, but never explicitly null.
fn present_value<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    pub schema_version: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub run_id: Option<String>,
    pub provenance: Provenance,
    pub ticks: Decimal,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub capture_end: Option<CaptureEnd>,
    pub cohorts: Vec<CohortCompletion>,
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
