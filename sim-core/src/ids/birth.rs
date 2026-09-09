//! Exact world-local birth identities and their boundary representation.
//!
//! Reserves checked IDs and encodes JSON without JavaScript precision loss.
//! World owns the counter, ancestry arrays, and admission ordering (spec section 3.4).

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Persistent identity of one admitted organism, never a reusable agent slot.
///
/// JSON uses a canonical decimal string or null for unavailable identity. Binary
/// serde preserves the raw u64 sentinel; this is not a world checkpoint format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../web/src/generated/"))]
pub struct BirthId(#[cfg_attr(test, ts(type = "string | null"))] u64);

impl BirthId {
    pub const NULL: Self = Self(u64::MAX);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn is_null(self) -> bool {
        self.0 == u64::MAX
    }
}

impl Default for BirthId {
    fn default() -> Self {
        Self::NULL
    }
}

/// Exhaustion reports unavailable identity without wrapping or consuming another ID.
pub(crate) fn issue_birth(next: &mut u64) -> Option<BirthId> {
    let following = next.checked_add(1)?;
    let id = BirthId::new(*next);
    *next = following;
    Some(id)
}

impl Serialize for BirthId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !serializer.is_human_readable() {
            return serializer.serialize_u64(self.0);
        }
        if self.is_null() {
            serializer.serialize_none()
        } else {
            serializer.collect_str(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for BirthId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            deserializer.deserialize_option(OptionalId)
        } else {
            u64::deserialize(deserializer).map(Self::new)
        }
    }
}

struct OptionalId;

impl<'de> de::Visitor<'de> for OptionalId {
    type Value = BirthId;

    fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("a decimal birth ID string or null")
    }

    fn visit_none<E: de::Error>(self) -> Result<BirthId, E> {
        Ok(BirthId::NULL)
    }

    fn visit_unit<E: de::Error>(self) -> Result<BirthId, E> {
        Ok(BirthId::NULL)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<BirthId, D::Error> {
        deserializer.deserialize_str(DecimalId)
    }
}

struct DecimalId;

impl<'de> de::Visitor<'de> for DecimalId {
    type Value = BirthId;

    fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("a canonical decimal u64 birth ID below the unavailable sentinel")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<BirthId, E> {
        if value.is_empty()
            || value.len() > 20
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(E::invalid_value(de::Unexpected::Str(value), &self));
        }
        let raw = value.parse::<u64>().map_err(E::custom)?;
        if raw == u64::MAX {
            return Err(E::custom("unavailable birth ID must be null"));
        }
        Ok(BirthId::new(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representation_is_one_u64_and_typescript_is_exact() {
        assert_eq!(size_of::<BirthId>(), size_of::<u64>());
        assert_eq!(
            <BirthId as ts_rs::TS>::inline(&ts_rs::Config::default()),
            "string | null"
        );
    }

    #[test]
    fn allocation_reaches_the_last_id_then_stays_exhausted() {
        let mut next = 0;
        assert_eq!(issue_birth(&mut next), Some(BirthId::new(0)));
        next = u64::MAX - 1;
        assert_eq!(issue_birth(&mut next), Some(BirthId::new(u64::MAX - 1)));
        assert_eq!(next, u64::MAX);
        assert_eq!(issue_birth(&mut next), None);
        assert_eq!(issue_birth(&mut next), None);
        assert_eq!(next, u64::MAX);
    }

    #[test]
    fn json_and_binary_round_trip_full_width_ids_without_numbers_in_json() {
        for raw in [0, 1, (1u64 << 53) + 1, u64::MAX - 1, u64::MAX] {
            let id = BirthId::new(raw);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(
                json,
                if id.is_null() {
                    "null".to_owned()
                } else {
                    format!("\"{raw}\"")
                }
            );
            assert_eq!(serde_json::from_str::<BirthId>(&json).unwrap(), id);
            let bytes = postcard::to_allocvec(&id).unwrap();
            assert_eq!(postcard::from_bytes::<BirthId>(&bytes).unwrap(), id);
            assert_eq!(postcard::from_bytes::<u64>(&bytes).unwrap(), raw);
        }
    }

    #[test]
    fn malformed_or_lossy_json_ids_are_rejected() {
        for json in [
            "0",
            "9007199254740993",
            "1.5",
            "true",
            "[]",
            "{}",
            "\"\"",
            "\"01\"",
            "\"+1\"",
            "\"-1\"",
            "\" 1\"",
            "\"1 \"",
            "\"1e3\"",
            "\"18446744073709551615\"",
            "\"18446744073709551616\"",
        ] {
            assert!(serde_json::from_str::<BirthId>(json).is_err(), "{json}");
        }
    }
}
