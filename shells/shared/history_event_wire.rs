//! Exact species-history event encoding shared by the native and WASM shells.
//!
//! Owns decimal counters and explicit parent metadata, not archive provenance,
//! capture policy, or persistence.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sim_core::history::{EventKind, Parent};
use sim_core::ids::{BirthId, SpeciesId};

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

pub(crate) fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
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
