//! Headless experiment orchestration and JSONL output.

use std::io::{self, Write};

use sim_core::control::{BrainInheritance, RANDOMIZED_AT_BIRTH_PROTOCOL};
use sim_core::history::Recorder;
use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use sim_core::spawn::SpawnFailureCounts;
use sim_core::species::SpeciesEventCounts;
use sim_core::world::World;

use crate::Result;
use crate::cli::RunArgs;
use crate::history::{self, ArchiveWriter, Capture};
use crate::metrics::{MetricsRecord, RunHeader, SCHEMA_VERSION, sample_pair};

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
        control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
    };
    if args.history.is_some() {
        history::validate_export(&header, args.history_capacity)?;
    }
    let mut evolving = World::new(args.seed, params.clone())?;
    let mut random_control =
        World::new_with_brain_inheritance(args.seed, params, BrainInheritance::RandomizedAtBirth)?;
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
        .map(|_| Capture::new(args.history_capacity))
        .transpose()?;
    seed(
        &mut evolving,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[0]),
        species_events.as_mut().map(|counts| &mut counts[0]),
        capture.as_mut().map(|capture| &mut capture.recorders[0]),
    )?;
    seed(
        &mut random_control,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[1]),
        species_events.as_mut().map(|counts| &mut counts[1]),
        capture.as_mut().map(|capture| &mut capture.recorders[1]),
    )?;

    if let Some(capture) = &capture {
        capture.check()?;
    }
    let (mut output, history_output) = crate::output::open(&args)?;
    let mut archive = history_output
        .map(|output| ArchiveWriter::new(output, &header, args.history_capacity))
        .transpose()?;
    if let (Some(archive), Some(capture)) = (&mut archive, &mut capture) {
        archive.drain(capture)?;
    }
    let mut final_sample = None;
    if let Some(output) = output.as_mut() {
        write_record(output, &MetricsRecord::Header(Box::new(header)))?;
        let sample = sample_pair(
            &evolving,
            &random_control,
            spawn_failures,
            structural_mutations,
            species_events,
            args.ticks == 0,
        )?;
        write_record(output, &MetricsRecord::Sample(Box::new(sample.clone())))?;
        final_sample = Some(sample);
    }

    for _ in 0..args.ticks {
        step(
            &mut evolving,
            spawn_failures.as_mut().map(|counts| &mut counts[0]),
            structural_mutations.as_mut().map(|counts| &mut counts[0]),
            species_events.as_mut().map(|counts| &mut counts[0]),
            capture.as_mut().map(|capture| &mut capture.recorders[0]),
        );
        step(
            &mut random_control,
            spawn_failures.as_mut().map(|counts| &mut counts[1]),
            structural_mutations.as_mut().map(|counts| &mut counts[1]),
            species_events.as_mut().map(|counts| &mut counts[1]),
            capture.as_mut().map(|capture| &mut capture.recorders[1]),
        );
        if let Some(capture) = &capture {
            capture.check()?;
        }
        let tick = evolving.tick_count();
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
    Ok(())
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
    recorder: Option<&mut Recorder>,
) -> Result<()> {
    let mut refusal = None;
    let placed = if let Some(recorder) = recorder {
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
            |event| history::record(recorder, event),
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
    recorder: Option<&mut Recorder>,
) {
    if let Some(recorder) = recorder {
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
            |event| history::record(recorder, event),
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
