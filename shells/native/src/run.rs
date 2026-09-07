//! Headless experiment orchestration and JSONL output.

use std::fs::File;
use std::io::{self, BufWriter, Write};

use sim_core::control::BrainInheritance;
use sim_core::params::SimParams;
use sim_core::spawn::SpawnFailureCounts;
use sim_core::world::World;

use crate::Result;
use crate::cli::RunArgs;
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

    let params = load_params(args.params.as_deref())?;
    let header = RunHeader {
        schema_version: SCHEMA_VERSION,
        sim_version: env!("CARGO_PKG_VERSION").to_owned(),
        source_revision: env!("SYNTHETIC_EVOLUTION_REVISION").to_owned(),
        phase: 1,
        seed: args.seed.to_string(),
        ticks: args.ticks,
        founders: args.founders,
        sample_every: args.sample_every,
        params: params.clone(),
        control: "randomized_at_birth".to_owned(),
    };
    let mut evolving = World::new(args.seed, params.clone())?;
    let mut random_control =
        World::new_with_brain_inheritance(args.seed, params, BrainInheritance::RandomizedAtBirth)?;
    let mut spawn_failures = args
        .metrics
        .as_ref()
        .map(|_| [SpawnFailureCounts::default(); 2]);
    seed(
        &mut evolving,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[0]),
    )?;
    seed(
        &mut random_control,
        args.founders,
        spawn_failures.as_mut().map(|counts| &mut counts[1]),
    )?;

    let mut output = args.metrics.as_deref().map(metrics_writer).transpose()?;
    let mut final_sample = None;
    if let Some(output) = output.as_mut() {
        write_record(output, &MetricsRecord::Header(header))?;
        let sample = sample_pair(&evolving, &random_control, spawn_failures, args.ticks == 0)?;
        write_record(output, &MetricsRecord::Sample(sample.clone()))?;
        final_sample = Some(sample);
    }

    for _ in 0..args.ticks {
        if let Some([evolving_counts, control_counts]) = spawn_failures.as_mut() {
            evolving.step_with_spawn_observer(|error| evolving_counts.record(error));
            random_control.step_with_spawn_observer(|error| control_counts.record(error));
        } else {
            evolving.step();
            random_control.step();
        }
        let tick = evolving.tick_count();
        if (tick % args.sample_every == 0 || tick == args.ticks)
            && let Some(output) = output.as_mut()
        {
            let sample = sample_pair(
                &evolving,
                &random_control,
                spawn_failures,
                tick == args.ticks,
            )?;
            write_record(output, &MetricsRecord::Sample(sample.clone()))?;
            final_sample = Some(sample);
        }
    }

    if let Some(output) = output.as_mut() {
        output.flush()?;
    }
    let final_sample = final_sample.map_or_else(
        || sample_pair(&evolving, &random_control, spawn_failures, true),
        Ok,
    )?;
    let hashes = final_sample
        .final_state_hashes
        .as_ref()
        .expect("final sample always includes hashes");
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

fn seed(world: &mut World, founders: u32, counts: Option<&mut SpawnFailureCounts>) -> Result<()> {
    let mut refusal = None;
    let placed = if let Some(counts) = counts {
        world.seed_founders_with_observer(founders, |error| {
            counts.record(error);
            refusal = Some(error);
        })
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

fn metrics_writer(path: &std::path::Path) -> Result<BufWriter<Box<dyn Write>>> {
    let target: Box<dyn Write> = if path == std::path::Path::new("-") {
        Box::new(io::stdout())
    } else {
        Box::new(File::create(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not create metrics {}: {error}", path.display()),
            )
        })?)
    };
    Ok(BufWriter::new(target))
}

fn write_record(writer: &mut impl Write, record: &MetricsRecord) -> Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}
