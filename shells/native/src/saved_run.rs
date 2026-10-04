//! Native saved-run files: assemble, fully validate, inspect, and resume bundles.
//!
//! The container format is shared with the browser; this module owns file I/O,
//! checkpoint restore with host limits, history-segment validation, and continuing
//! restored worlds with an optional resumed history segment.

use std::fs;
use std::io::{self, Cursor, Write};
use std::path::Path;

use sim_core::checkpoint::CHECKPOINT_FORMAT;
use sim_core::world::World;

use crate::Result;
use crate::cli::{ResumeArgs, SavedRunArgs};
use crate::history::{self, ArchiveWriter, Capture};
use crate::metrics::{RunHeader, StateHashes};

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
    let worlds = format::restore_cohorts(&run, bytes.len(), max_core_bytes).map_err(invalid)?;
    let manifest = run.manifest;

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
        let (header, summary) = crate::history_reader::parse_archive(Cursor::new(archive))
            .map_err(|error| invalid(format!("history segment {index}: {error}")))?;
        let cohorts: Vec<Cohort> = summary
            .cohorts
            .iter()
            .map(|c| match c.cohort {
                crate::history_wire::Cohort::Evolving => Cohort::Evolving,
                crate::history_wire::Cohort::RandomControl => Cohort::RandomControl,
            })
            .collect();
        // A paired archive may accompany one cohort that a browser loaded from it.
        let covered = manifest.cohorts.iter().all(|c| cohorts.contains(&c.cohort));
        // Never later events: each segment ends exactly where the next one begins.
        if summary.provenance.seed.0 != manifest.provenance.seed.0
            || summary.ticks.0 != end
            || !covered
            // The first segment starts the run; later ones resume from their boundary.
            || header.resumed_from_tick.map(|tick| tick.0)
                != (segment.starts_at() != 0).then_some(segment.starts_at())
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
///
/// With `--history`, the continuation is captured as its own schema four segment
/// beginning at the resume tick, never appended to the restored archive.
pub fn resume(args: ResumeArgs) -> Result<()> {
    crate::output::validate_resume(
        &args.bundle,
        args.history.as_deref(),
        args.save_run.as_deref(),
    )?;
    let loaded = read(&args.bundle, args.max_core_bytes)?;
    let start = loaded.manifest.tick.0;
    let mut worlds = loaded.worlds;
    let representative_genes = args.representatives.then_some(args.representative_genes);
    let mut archive = None;
    if let Some(path) = &args.history {
        if worlds.len() != 2 {
            return Err(invalid("native history capture needs a paired saved run").into());
        }
        let provenance = &loaded.manifest.provenance;
        let run = RunHeader {
            schema_version: crate::metrics::SCHEMA_VERSION,
            sim_version: env!("CARGO_PKG_VERSION").to_owned(),
            source_revision: env!("SYNTHETIC_EVOLUTION_REVISION").to_owned(),
            phase: u32::from(provenance.phase),
            seed: provenance.seed.0.to_string(),
            ticks: start + args.ticks,
            founders: provenance.founders,
            sample_every: args.drain_every,
            params: worlds[0].1.params().clone(),
            control: provenance.control.clone(),
            retune: None,
        };
        history::validate_export(&run, args.history_capacity, representative_genes)?;
        let capture = Capture::new(args.history_capacity, representative_genes)?;
        let output = crate::output::open_output(path, "history")?;
        let writer = ArchiveWriter::resumed(
            output,
            &run,
            args.history_capacity,
            representative_genes,
            start,
        )?;
        archive = Some((writer, capture));
    }
    for _ in 0..args.ticks {
        match &mut archive {
            Some((writer, capture)) => {
                for ((_, world), cohort) in worlds.iter_mut().zip(&mut capture.cohorts) {
                    world.step_with_history_observer(
                        |_| {},
                        |_| {},
                        |_| {},
                        |event, representative| cohort.record(event, representative),
                    );
                }
                capture.check()?;
                let tick = worlds[0].1.tick_count();
                if tick % args.drain_every == 0 || tick == start + args.ticks {
                    writer.drain(capture)?;
                }
            }
            None => {
                for (_, world) in &mut worlds {
                    world.step();
                }
            }
        }
    }
    if let Some((writer, mut capture)) = archive {
        let hashes = StateHashes {
            evolving: format!("{:016x}", worlds[0].1.state_hash()),
            random_control: format!("{:016x}", worlds[1].1.state_hash()),
        };
        writer.finish(&mut capture, &hashes)?;
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
            history.push(continuation(args.history.as_deref(), start)?);
        }
        let refs: Vec<(Cohort, &World)> = worlds.iter().map(|(c, w)| (*c, w)).collect();
        write(path, &assemble(loaded.manifest.provenance, &refs, history)?)?;
    }
    Ok(())
}

/// A history segment for a new bundle: the archive if it can be re-read, or why not.
pub(crate) fn continuation(
    history: Option<&Path>,
    start: u64,
) -> Result<(Segment, Option<Vec<u8>>)> {
    let starts_at = Decimal(start);
    let unavailable = |reason| Segment::Unavailable { starts_at, reason };
    Ok(match history {
        None => (unavailable(Unavailable::NotRecorded), None),
        // Only a regular file can be re-read faithfully; stdout or a pipe cannot.
        Some(path) if path == Path::new("-") || !fs::metadata(path).is_ok_and(|m| m.is_file()) => {
            (unavailable(Unavailable::NotRetained), None)
        }
        Some(path) => {
            let archive = fs::read(path)?;
            (
                Segment::Included {
                    starts_at,
                    bytes: Decimal(archive.len() as u64),
                },
                Some(archive),
            )
        }
    })
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
                founders: 8,
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
