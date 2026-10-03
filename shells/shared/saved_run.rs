//! The portable saved-run bundle shared by the native and browser shells.
//!
//! Frames one or two cohorts' core checkpoints and an ordered list of history
//! segments behind a strict JSON manifest (spec §7.10). It owns framing and manifest
//! validation only, not I/O, checkpoint restore, or history-archive validation.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Leading bytes that identify a saved run before any decoding.
pub const MAGIC: [u8; 8] = *b"SEVRUN\0\0";
/// Container version. Bump with any framing or manifest change; no migration.
pub const VERSION: u32 = 1;
/// The manifest is metadata, never bulk data; larger is malformed.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const FORMAT: &str = "synthetic-evolution-saved-run";
const HEADER_BYTES: usize = MAGIC.len() + 2 * size_of::<u32>();

/// An exact u64 carried as a canonical decimal string, as elsewhere in the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Decimal(pub u64);

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
            || !text.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(de::Error::custom("expected a canonical decimal u64 string"));
        }
        text.parse().map(Self).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cohort {
    Evolving,
    RandomControl,
}

/// The run a bundle descends from, unchanged by later resumes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub sim_version: String,
    pub source_revision: String,
    pub phase: u8,
    pub seed: Decimal,
    /// Generation-zero population, which later history segments still describe.
    pub founders: u32,
    pub control: String,
    /// Browser runs name themselves; native runs are identified by their files.
    pub run_id: Option<String>,
}

/// The build that wrote this bundle, which differs from provenance after a resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Writer {
    pub sim_version: String,
    pub source_revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortEntry {
    pub cohort: Cohort,
    /// The checkpoint's own `state_hash`, as 16 lowercase hex digits.
    pub state_hash: String,
    pub bytes: Decimal,
}

/// Why a history segment has no archive. Absence is explicit, never reconstructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    /// Capture was off for this segment.
    NotRecorded,
    /// Capture streamed somewhere the shell could not read back, such as stdout.
    NotRetained,
}

/// One contiguous stretch of a run's history, starting at `starts_at` and ending
/// where the next segment starts, or at the checkpoint tick for the last one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Segment {
    Included {
        starts_at: Decimal,
        bytes: Decimal,
    },
    Unavailable {
        starts_at: Decimal,
        reason: Unavailable,
    },
}

impl Segment {
    pub fn starts_at(&self) -> u64 {
        match self {
            Self::Included { starts_at, .. } | Self::Unavailable { starts_at, .. } => starts_at.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// Must equal the reader's core checkpoint format; there is no migration.
    pub checkpoint_format: u32,
    pub provenance: Provenance,
    pub writer: Writer,
    /// The between-ticks boundary every checkpoint and the history end at.
    pub tick: Decimal,
    pub cohorts: Vec<CohortEntry>,
    pub history: Vec<Segment>,
}

/// A decoded bundle borrowing its sections from the input bytes.
pub struct SavedRun<'a> {
    pub manifest: Manifest,
    /// In manifest cohort order.
    pub checkpoints: Vec<&'a [u8]>,
    /// One per history segment; `None` for unavailable segments.
    pub history: Vec<Option<&'a [u8]>>,
}

impl Manifest {
    pub fn new(
        provenance: Provenance,
        writer: Writer,
        checkpoint_format: u32,
        tick: u64,
        cohorts: Vec<CohortEntry>,
        history: Vec<Segment>,
    ) -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            checkpoint_format,
            provenance,
            writer,
            tick: Decimal(tick),
            cohorts,
            history,
        }
    }

    /// Structural rules shared by both shells' readers.
    fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != VERSION {
            return Err(format!(
                "unsupported saved-run format {} v{}",
                self.format, self.version
            ));
        }
        if self.provenance.founders == 0 {
            return Err("saved-run provenance needs a nonzero founder count".into());
        }
        let cohorts: Vec<Cohort> = self.cohorts.iter().map(|c| c.cohort).collect();
        if !matches!(
            cohorts.as_slice(),
            [Cohort::Evolving]
                | [Cohort::RandomControl]
                | [Cohort::Evolving, Cohort::RandomControl]
        ) {
            return Err("saved-run cohorts must be one cohort or both in canonical order".into());
        }
        if self.cohorts.iter().any(|c| {
            c.bytes.0 == 0
                || c.state_hash.len() != 16
                || !c
                    .state_hash
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }) {
            return Err("saved-run cohort needs a nonempty checkpoint and a 16-digit hash".into());
        }
        if self.history.first().is_none_or(|s| s.starts_at() != 0) {
            return Err("saved-run history must start with a segment at tick 0".into());
        }
        let mut previous = 0;
        for (index, segment) in self.history.iter().enumerate() {
            let start = segment.starts_at();
            if (index > 0 && start <= previous) || start > self.tick.0 {
                return Err(
                    "history segments must start in increasing order by the save tick".into(),
                );
            }
            if matches!(segment, Segment::Included { bytes, .. } if bytes.0 == 0) {
                return Err("an included history segment must be nonempty".into());
            }
            previous = start;
        }
        Ok(())
    }

    fn section_lengths(&self) -> Vec<u64> {
        self.cohorts
            .iter()
            .map(|c| c.bytes.0)
            .chain(self.history.iter().filter_map(|s| match s {
                Segment::Included { bytes, .. } => Some(bytes.0),
                Segment::Unavailable { .. } => None,
            }))
            .collect()
    }
}

/// Frames a bundle. Section lengths in `manifest` must match the slices, in order:
/// each cohort's checkpoint, then each included history segment.
pub fn encode(manifest: &Manifest, sections: &[&[u8]]) -> Result<Vec<u8>, String> {
    manifest.validate()?;
    let lengths = manifest.section_lengths();
    if lengths.len() != sections.len()
        || lengths
            .iter()
            .zip(sections)
            .any(|(&len, s)| len != s.len() as u64)
    {
        return Err("saved-run manifest lengths do not match its sections".into());
    }
    let manifest_json = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
    if manifest_json.len() > MAX_MANIFEST_BYTES {
        return Err("saved-run manifest exceeds 1 MiB".into());
    }
    let total: usize = sections.iter().map(|s| s.len()).sum();
    let mut bytes = Vec::with_capacity(HEADER_BYTES + manifest_json.len() + total);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(manifest_json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&manifest_json);
    for section in sections {
        bytes.extend_from_slice(section);
    }
    Ok(bytes)
}

/// Decodes an untrusted bundle of at most `max_bytes`, borrowing its sections.
///
/// Validates framing and the manifest only; callers still restore each checkpoint
/// and validate each history segment before replacing anything live.
pub fn decode(bytes: &[u8], max_bytes: usize) -> Result<SavedRun<'_>, String> {
    if bytes.len() > max_bytes {
        return Err("saved run exceeds the size limit".into());
    }
    if bytes.len() < HEADER_BYTES || bytes[..MAGIC.len()] != MAGIC {
        return Err("not a saved run".into());
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    let version = word(MAGIC.len());
    if version != VERSION {
        return Err(format!(
            "unsupported saved-run version {version} (expected {VERSION})"
        ));
    }
    let manifest_len = word(MAGIC.len() + 4) as usize;
    if manifest_len > MAX_MANIFEST_BYTES || manifest_len > bytes.len() - HEADER_BYTES {
        return Err("saved-run manifest length is invalid".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes[HEADER_BYTES..HEADER_BYTES + manifest_len])
            .map_err(|e| format!("invalid saved-run manifest: {e}"))?;
    manifest.validate()?;
    let mut rest = &bytes[HEADER_BYTES + manifest_len..];
    let mut take = |len: u64| -> Result<&[u8], String> {
        let len = usize::try_from(len).map_err(|_| "saved-run section is too large")?;
        if len > rest.len() {
            return Err("saved run is truncated".into());
        }
        let (section, tail) = rest.split_at(len);
        rest = tail;
        Ok(section)
    };
    let checkpoints = manifest
        .cohorts
        .iter()
        .map(|c| take(c.bytes.0))
        .collect::<Result<Vec<_>, _>>()?;
    let history = manifest
        .history
        .iter()
        .map(|s| match s {
            Segment::Included { bytes, .. } => take(bytes.0).map(Some),
            Segment::Unavailable { .. } => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !rest.is_empty() {
        return Err("saved run has trailing bytes".into());
    }
    Ok(SavedRun {
        manifest,
        checkpoints,
        history,
    })
}
