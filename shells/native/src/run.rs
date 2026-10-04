//! Headless experiment orchestration and JSONL output.

use std::io::{self, Write};

use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use sim_core::spawn::SpawnFailureCounts;
use sim_core::species::SpeciesEventCounts;
use sim_core::world::World;

use crate::Result;
use crate::cli::RunArgs;
use crate::history::{self, ArchiveWriter, Capture, CohortCapture};
use crate::metrics::{MetricsRecord, Retune, RunHeader, SCHEMA_VERSION, sample_pair};
use crate::saved_run::{self, Cohort, Decimal, Provenance};

pub fn run(args: RunArgs) -> Result<()> {
    if args.sample_every == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--sample-every must be non-zero",
        )
        .into());
    }
    if args.founders == 0 {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "--founders must be non-zero").into(),
        );
    }

    crate::output::validate(&args)?;
    let params = load_params(args.params.as_deref())?;
    let retune = load_retune(&args, &params)?;
    let header = RunHeader {
        schema_version: SCHEMA_VERSION,
        sim_version: env!("CARGO_PKG_VERSION").to_owned(),
        source_revision: env!("SYNTHETIC_EVOLUTION_REVISION").to_owned(),
        phase: 2,
        seed: args.seed.to_string(),
        ticks: args.ticks,
        founders: args.founders,
        sample_every: args.sample_every,
        params: params.clone(),
        control: args.control.protocol().to_owned(),
        retune: retune.clone(),
    };
    if args.history.is_some() {
        history::validate_export(&header, args.history_capacity, args.representative_genes())?;
    }
    let mut evolving = World::new(args.seed, params.clone())?;
    let mut random_control =
        World::new_with_brain_inheritance(args.seed, params, args.control.heredity())?;
    // Refused now rather than at its tick, so a long run cannot fail at the end.
    if let Some(retune) = &retune {
        for world in [&evolving, &random_control] {
            world.check_retune(&retune.params).map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidInput, format!("--retune: {error}"))
            })?;
        }
    }
    let mut spawn_failures = args
        .metrics
        .as_ref()
        .map(|_| [SpawnFailureCounts::default(); 2]);
    let mut structural_mutations = args
        .metrics
        .as_ref()
        .map(|_| [StructuralMutationCounts::default(); 2]);
    let mut species_events = args
        .metrics
        .as_ref()
        .map(|_| [SpeciesEventCounts::default(); 2]);
    let mut capture = args
        .history
        .as_ref()
        .map(|_| Capture::new(args.history_capacity, args.representative_genes()))
        .transpose()?;
    seed(
        &mut evolving,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[0]),
        species_events.as_mut().map(|counts| &mut counts[0]),
        capture.as_mut().map(|capture| &mut capture.cohorts[0]),
    )?;
    seed(
        &mut random_control,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[1]),
        species_events.as_mut().map(|counts| &mut counts[1]),
        capture.as_mut().map(|capture| &mut capture.cohorts[1]),
    )?;

    if let Some(capture) = &capture {
        capture.check()?;
    }
    let (mut output, history_output) = crate::output::open(&args)?;
    let mut archive = history_output
        .map(|output| {
            ArchiveWriter::new(
                output,
                &header,
                args.history_capacity,
                args.representative_genes(),
            )
        })
        .transpose()?;
    if let (Some(archive), Some(capture)) = (&mut archive, &mut capture) {
        archive.drain(capture)?;
    }
    let mut final_sample = None;
    if let Some(output) = output.as_mut() {
        write_record(output, &MetricsRecord::Header(Box::new(header.clone())))?;
        let sample = sample_pair(
            &evolving,
            &random_control,
            spawn_failures,
            structural_mutations,
            species_events,
            archive.as_ref().map(ArchiveWriter::availability),
            args.ticks == 0,
        )?;
        write_record(output, &MetricsRecord::Sample(Box::new(sample.clone())))?;
        final_sample = Some(sample);
    }

    apply_retune(&retune, 0, &mut evolving, &mut random_control)?;
    for _ in 0..args.ticks {
        step(
            &mut evolving,
            spawn_failures.as_mut().map(|counts| &mut counts[0]),
            structural_mutations.as_mut().map(|counts| &mut counts[0]),
            species_events.as_mut().map(|counts| &mut counts[0]),
            capture.as_mut().map(|capture| &mut capture.cohorts[0]),
        );
        step(
            &mut random_control,
            spawn_failures.as_mut().map(|counts| &mut counts[1]),
            structural_mutations.as_mut().map(|counts| &mut counts[1]),
            species_events.as_mut().map(|counts| &mut counts[1]),
            capture.as_mut().map(|capture| &mut capture.cohorts[1]),
        );
        if let Some(capture) = &capture {
            capture.check()?;
        }
        let tick = evolving.tick_count();
        apply_retune(&retune, tick, &mut evolving, &mut random_control)?;
        if tick % args.sample_every == 0 || tick == args.ticks {
            if let (Some(archive), Some(capture)) = (&mut archive, &mut capture) {
                archive.drain(capture)?;
            }
            if let Some(output) = output.as_mut() {
                let sample = sample_pair(
                    &evolving,
                    &random_control,
                    spawn_failures,
                    structural_mutations,
                    species_events,
                    archive.as_ref().map(ArchiveWriter::availability),
                    tick == args.ticks,
                )?;
                write_record(output, &MetricsRecord::Sample(Box::new(sample.clone())))?;
                final_sample = Some(sample);
            }
        }
    }

    if let Some(output) = output.as_mut() {
        output.flush()?;
    }
    let final_sample = final_sample.map_or_else(
        || {
            sample_pair(
                &evolving,
                &random_control,
                spawn_failures,
                structural_mutations,
                species_events,
                archive.as_ref().map(ArchiveWriter::availability),
                true,
            )
        },
        Ok,
    )?;
    let hashes = final_sample
        .final_state_hashes
        .as_ref()
        .expect("final sample always includes hashes");
    if let (Some(archive), Some(capture)) = (archive, &mut capture) {
        archive.finish(capture, hashes)?;
    }
    eprintln!(
        "completed {} ticks: evolving={} ({}) control={} ({})",
        args.ticks,
        final_sample.evolving.population,
        hashes.evolving,
        final_sample.random_control.population,
        hashes.random_control
    );
    if let Some(path) = &args.save_run {
        save_run(path, &args, &header, &evolving, &random_control)?;
    }
    Ok(())
}

/// Bundles both cohorts with the finished history, read back from its file. History
/// streamed to stdout cannot be re-read, so the bundle says so rather than omit it.
fn save_run(
    path: &std::path::Path,
    args: &RunArgs,
    header: &RunHeader,
    evolving: &World,
    random_control: &World,
) -> Result<()> {
    let history = saved_run::continuation(args.history.as_deref(), 0)?;
    let provenance = Provenance {
        sim_version: header.sim_version.clone(),
        source_revision: header.source_revision.clone(),
        phase: header.phase as u8,
        seed: Decimal(args.seed),
        founders: args.founders,
        control: header.control.clone(),
        run_id: None,
    };
    let bytes = saved_run::assemble(
        provenance,
        &[
            (Cohort::Evolving, evolving),
            (Cohort::RandomControl, random_control),
        ],
        vec![history],
    )?;
    saved_run::write(path, &bytes)
}

/// Applies a scheduled retune to both worlds at the boundary after `tick`.
fn apply_retune(
    retune: &Option<Retune>,
    tick: u64,
    evolving: &mut World,
    control: &mut World,
) -> Result<()> {
    if let Some(retune) = retune.as_ref().filter(|retune| retune.at_tick == tick) {
        evolving.set_params(retune.params.clone())?;
        control.set_params(retune.params.clone())?;
    }
    Ok(())
}

/// The run's params with `--retune`'s partial document laid over them, field by field.
fn load_retune(args: &RunArgs, base: &SimParams) -> Result<Option<Retune>> {
    let (Some(path), Some(at_tick)) = (args.retune.as_deref(), args.retune_at) else {
        return Ok(None);
    };
    if at_tick > args.ticks {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--retune-at must be at most --ticks",
        )
        .into());
    }
    let json = std::fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not read retune {}: {error}", path.display()),
        )
    })?;
    let mut merged = serde_json::to_value(base)?;
    overlay(&mut merged, serde_json::from_str(&json)?);
    Ok(Some(Retune {
        at_tick,
        params: serde_json::from_value(merged)?,
    }))
}

/// Replaces `base`'s fields with `top`'s, recursing into objects; arrays and scalars
/// are replaced whole.
fn overlay(base: &mut serde_json::Value, top: serde_json::Value) {
    match (base, top) {
        (serde_json::Value::Object(base), serde_json::Value::Object(top)) => {
            for (key, value) in top {
                overlay(base.entry(key).or_insert(serde_json::Value::Null), value);
            }
        }
        (base, top) => *base = top,
    }
}

fn load_params(path: Option<&std::path::Path>) -> Result<SimParams> {
    match path {
        Some(path) => {
            let json = std::fs::read_to_string(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not read params {}: {error}", path.display()),
                )
            })?;
            Ok(serde_json::from_str(&json)?)
        }
        None => Ok(SimParams::default()),
    }
}

fn seed(
    world: &mut World,
    founders: u32,
    mut counts: Option<&mut SpawnFailureCounts>,
    mut species: Option<&mut SpeciesEventCounts>,
    capture: Option<&mut CohortCapture>,
) -> Result<()> {
    let mut refusal = None;
    let placed = if let Some(capture) = capture {
        world.seed_founders_with_history_observer(
            founders,
            |error| {
                if let Some(counts) = &mut counts {
                    counts.record(error);
                }
                refusal = Some(error);
            },
            |event| {
                if let Some(species) = &mut species {
                    species.record(event);
                }
            },
            |event, representative| capture.record(event, representative),
        )
    } else if let (Some(counts), Some(species)) = (counts, species) {
        world.seed_founders_with_observers(
            founders,
            |error| {
                counts.record(error);
                refusal = Some(error);
            },
            |event| species.record(event),
        )
    } else {
        world.seed_founders(founders)
    };
    if placed != founders {
        let detail = refusal.map_or_else(String::new, |error| format!(": {error}"));
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "requested {founders} founders but world capacity allowed only {placed}{detail}"
            ),
        )
        .into());
    }
    Ok(())
}

fn step(
    world: &mut World,
    mut counts: Option<&mut SpawnFailureCounts>,
    mut edits: Option<&mut StructuralMutationCounts>,
    mut species: Option<&mut SpeciesEventCounts>,
    capture: Option<&mut CohortCapture>,
) {
    if let Some(capture) = capture {
        world.step_with_history_observer(
            |error| {
                if let Some(counts) = &mut counts {
                    counts.record(error);
                }
            },
            |event| {
                if let Some(edits) = &mut edits {
                    edits.record(event);
                }
            },
            |event| {
                if let Some(species) = &mut species {
                    species.record(event);
                }
            },
            |event, representative| capture.record(event, representative),
        );
    } else if let (Some(counts), Some(edits), Some(species)) = (counts, edits, species) {
        world.step_with_all_observers(
            |error| counts.record(error),
            |event| edits.record(event),
            |event| species.record(event),
        );
    } else {
        world.step();
    }
}

fn write_record(writer: &mut impl Write, record: &MetricsRecord) -> Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}
