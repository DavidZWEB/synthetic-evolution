//! Native saved-run files: assemble, fully validate, inspect, and resume bundles.
//!
//! The container format is shared with the browser; this module owns file I/O,
//! checkpoint restore with host limits, history-segment validation, and continuing
//! restored worlds. It does not write history for resumed segments yet.

use std::fs;
use std::io::{self, Cursor, Write};
use std::path::Path;

use sim_core::checkpoint::{CHECKPOINT_FORMAT, CheckpointLimits};
use sim_core::control::BrainInheritance;
use sim_core::world::World;

use crate::Result;
use crate::cli::{ResumeArgs, SavedRunArgs};

#[path = "../../shared/saved_run.rs"]
mod format;
pub(crate) use format::{
    Cohort, CohortEntry, Decimal, Manifest, Provenance, Segment, Unavailable, Writer,
};

/// A saved run whose every component restored and validated.
pub(crate) struct Loaded {
    pub manifest: Manifest,
    /// In manifest cohort order.
    pub worlds: Vec<(Cohort, World)>,
    /// One per manifest segment; `None` where the segment is unavailable.
    pub history: Vec<Option<Vec<u8>>>,
}

pub(crate) fn writer() -> Writer {
    Writer {
        sim_version: env!("CARGO_PKG_VERSION").to_owned(),
        source_revision: env!("SYNTHETIC_EVOLUTION_REVISION").to_owned(),
    }
}

fn heredity(cohort: Cohort) -> BrainInheritance {
    match cohort {
        Cohort::Evolving => BrainInheritance::Evolving,
        Cohort::RandomControl => BrainInheritance::RandomizedAtBirth,
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Encodes worlds at one between-ticks boundary with their history segments.
pub(crate) fn assemble(
    provenance: Provenance,
    worlds: &[(Cohort, &World)],
    history: Vec<(Segment, Option<Vec<u8>>)>,
) -> Result<Vec<u8>> {
    let tick = worlds[0].1.tick_count();
    debug_assert!(worlds.iter().all(|(_, w)| w.tick_count() == tick));
    let checkpoints: Vec<Vec<u8>> = worlds.iter().map(|(_, w)| w.checkpoint()).collect();
    let cohorts = worlds
        .iter()
        .zip(&checkpoints)
        .map(|(&(cohort, world), bytes)| CohortEntry {
            cohort,
            state_hash: format!("{:016x}", world.state_hash()),
            bytes: Decimal(bytes.len() as u64),
        })
        .collect();
    let (segments, archives): (Vec<Segment>, Vec<Option<Vec<u8>>>) = history.into_iter().unzip();
    let manifest = Manifest::new(
        provenance,
        writer(),
        CHECKPOINT_FORMAT,
        tick,
        cohorts,
        segments,
    );
    let sections: Vec<&[u8]> = checkpoints
        .iter()
        .map(Vec::as_slice)
        .chain(archives.iter().flatten().map(Vec::as_slice))
        .collect();
    Ok(format::encode(&manifest, &sections).map_err(invalid)?)
}

/// Decodes and validates a bundle completely before anything uses it.
pub(crate) fn load(bytes: &[u8], max_core_bytes: u64) -> Result<Loaded> {
    let run = format::decode(bytes, bytes.len()).map_err(invalid)?;
    let manifest = run.manifest;
    if manifest.checkpoint_format != CHECKPOINT_FORMAT {
        return Err(invalid(format!(
            "saved run uses checkpoint format {}, this build reads {CHECKPOINT_FORMAT}",
            manifest.checkpoint_format
        ))
        .into());
    }
    let limits = CheckpointLimits {
        max_bytes: bytes.len(),
        max_core_bytes,
    };
    let mut worlds = Vec::with_capacity(manifest.cohorts.len());
    for (entry, checkpoint) in manifest.cohorts.iter().zip(&run.checkpoints) {
        let world = World::from_checkpoint(checkpoint, limits)
            .map_err(|error| invalid(format!("{:?} checkpoint: {error}", entry.cohort)))?;
        if world.brain_inheritance() != heredity(entry.cohort)
            || world.seed() != manifest.provenance.seed.0
            || world.tick_count() != manifest.tick.0
            || format!("{:016x}", world.state_hash()) != entry.state_hash
        {
            return Err(invalid(format!(
                "{:?} checkpoint does not match the manifest's heredity, seed, tick, or hash",
                entry.cohort
            ))
            .into());
        }
        worlds.push((entry.cohort, world));
    }
    // A paired experiment's control is only a control under identical conditions.
    if let [(_, evolving), (_, control)] = worlds.as_slice()
        && evolving.params() != control.params()
    {
        return Err(invalid("paired cohorts were saved with different params").into());
    }

    let mut history = Vec::with_capacity(manifest.history.len());
    for (index, (segment, archive)) in manifest.history.iter().zip(&run.history).enumerate() {
        let end = manifest
            .history
            .get(index + 1)
            .map_or(manifest.tick.0, Segment::starts_at);
        let Some(archive) = archive else {
            history.push(None);
            continue;
        };
        let summary = crate::history_reader::parse(Cursor::new(archive))
            .map_err(|error| invalid(format!("history segment {index}: {error}")))?;
        let cohorts: Vec<Cohort> = summary
            .cohorts
            .iter()
            .map(|c| match c.cohort {
                crate::history_wire::Cohort::Evolving => Cohort::Evolving,
                crate::history_wire::Cohort::RandomControl => Cohort::RandomControl,
            })
            .collect();
        let expected: Vec<Cohort> = manifest.cohorts.iter().map(|c| c.cohort).collect();
        // Never later events: each segment ends exactly where the next one begins.
        if summary.provenance.seed.0 != manifest.provenance.seed.0
            || summary.ticks.0 != end
            || cohorts != expected
            || segment.starts_at() != 0
        {
            return Err(invalid(format!(
                "history segment {index} does not belong to this run, its cohorts, or its boundary"
            ))
            .into());
        }
        history.push(Some(archive.to_vec()));
    }
    Ok(Loaded {
        manifest,
        worlds,
        history,
    })
}

pub(crate) fn read(path: &Path, max_core_bytes: u64) -> Result<Loaded> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not read saved run {}: {error}", path.display()),
        )
    })?;
    load(&bytes, max_core_bytes)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not write saved run {}: {error}", path.display()),
        )
        .into()
    })
}

/// `native saved-run`: validate every component and summarize.
pub fn inspect(args: SavedRunArgs) -> Result<()> {
    let loaded = read(&args.bundle, args.max_core_bytes)?;
    let mut output = io::stdout().lock();
    if args.json {
        serde_json::to_writer_pretty(&mut output, &loaded.manifest)?;
        writeln!(output)?;
        return Ok(());
    }
    let m = &loaded.manifest;
    writeln!(
        output,
        "saved run at tick {} from seed {} ({} {}); written by {} {}",
        m.tick.0,
        m.provenance.seed.0,
        m.provenance.sim_version,
        m.provenance.source_revision,
        m.writer.sim_version,
        m.writer.source_revision
    )?;
    for (cohort, world) in &loaded.worlds {
        writeln!(
            output,
            "{cohort:?}: population {}, {} species, hash {:016x}",
            world.population(),
            world.species_count(),
            world.state_hash()
        )?;
    }
    for segment in &m.history {
        match segment {
            Segment::Included { starts_at, bytes } => writeln!(
                output,
                "history from tick {}: archived ({} bytes)",
                starts_at.0, bytes.0
            )?,
            Segment::Unavailable { starts_at, reason } => writeln!(
                output,
                "history from tick {}: unavailable ({reason:?})",
                starts_at.0
            )?,
        }
    }
    Ok(())
}

/// `native resume`: continue every saved cohort together, never reseeding a control.
pub fn resume(args: ResumeArgs) -> Result<()> {
    let loaded = read(&args.bundle, args.max_core_bytes)?;
    let start = loaded.manifest.tick.0;
    let mut worlds = loaded.worlds;
    for _ in 0..args.ticks {
        for (_, world) in &mut worlds {
            world.step();
        }
    }
    for (cohort, world) in &worlds {
        eprintln!(
            "resumed {cohort:?} from tick {start} through {}: population {} ({:016x})",
            world.tick_count(),
            world.population(),
            world.state_hash()
        );
    }
    if let Some(path) = &args.save_run {
        // The restored prefix is kept as saved; the continuation is a distinct segment.
        let mut history: Vec<(Segment, Option<Vec<u8>>)> = loaded
            .manifest
            .history
            .into_iter()
            .zip(loaded.history)
            .collect();
        if args.ticks > 0 {
            history.push((
                Segment::Unavailable {
                    starts_at: Decimal(start),
                    reason: Unavailable::NotRecorded,
                },
                None,
            ));
        }
        let refs: Vec<(Cohort, &World)> = worlds.iter().map(|(c, w)| (*c, w)).collect();
        write(path, &assemble(loaded.manifest.provenance, &refs, history)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::new(
            Provenance {
                sim_version: "0.1.0".into(),
                source_revision: "test".into(),
                phase: 2,
                seed: Decimal(7),
                control: "randomized_at_birth_v3".into(),
                run_id: None,
            },
            writer(),
            CHECKPOINT_FORMAT,
            10,
            vec![CohortEntry {
                cohort: Cohort::Evolving,
                state_hash: "0123456789abcdef".into(),
                bytes: Decimal(3),
            }],
            vec![
                Segment::Included {
                    starts_at: Decimal(0),
                    bytes: Decimal(2),
                },
                Segment::Unavailable {
                    starts_at: Decimal(4),
                    reason: Unavailable::NotRecorded,
                },
            ],
        )
    }

    #[test]
    fn framing_round_trips_and_rejects_inconsistent_manifests() {
        let bytes = format::encode(&manifest(), &[b"abc", b"hi"]).unwrap();
        let run = format::decode(&bytes, bytes.len()).unwrap();
        assert_eq!(run.manifest, manifest());
        assert_eq!(run.checkpoints, [b"abc".as_slice()]);
        assert_eq!(run.history, [Some(b"hi".as_slice()), None]);
        assert!(
            format::decode(&bytes, bytes.len() - 1).is_err(),
            "size limit"
        );
        assert!(
            format::encode(&manifest(), &[b"abc"]).is_err(),
            "missing section"
        );

        type Corrupt = fn(&mut Manifest);
        let cases: [(&str, Corrupt); 6] = [
            ("controls first", |m| {
                let mut control = m.cohorts[0].clone();
                control.cohort = Cohort::RandomControl;
                m.cohorts.insert(0, control);
            }),
            ("short hash", |m| {
                m.cohorts[0].state_hash.pop();
            }),
            ("history not from zero", |m| {
                m.history.remove(0);
            }),
            ("segment after save", |m| {
                m.history[1] = Segment::Unavailable {
                    starts_at: Decimal(11),
                    reason: Unavailable::NotRecorded,
                }
            }),
            ("segments out of order", |m| m.history.swap(0, 1)),
            ("wrong version", |m| m.version += 1),
        ];
        for (name, corrupt) in cases {
            let mut m = manifest();
            corrupt(&mut m);
            let mut framed = format::MAGIC.to_vec();
            framed.extend_from_slice(&format::VERSION.to_le_bytes());
            let json = serde_json::to_vec(&m).unwrap();
            framed.extend_from_slice(&(json.len() as u32).to_le_bytes());
            framed.extend_from_slice(&json);
            framed.extend_from_slice(b"abchi");
            assert!(
                format::decode(&framed, usize::MAX).is_err(),
                "accepted {name}"
            );
        }
        let unknown = String::from_utf8(serde_json::to_vec(&manifest()).unwrap())
            .unwrap()
            .replacen('{', "{\"extra\":1,", 1);
        assert!(serde_json::from_str::<Manifest>(&unknown).is_err());
    }
}
