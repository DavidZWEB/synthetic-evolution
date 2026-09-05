//! Headless experiment orchestration and JSONL output.

use std::fs::File;
use std::io::{self, BufWriter, Write};

use sim_core::control::BrainInheritance;
use sim_core::params::SimParams;
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
    seed(&mut evolving, args.founders)?;
    seed(&mut random_control, args.founders)?;

    let output_path = args
        .metrics
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("-"));
    let mut output = metrics_writer(output_path)?;
    write_record(&mut output, &MetricsRecord::Header(header))?;

    let mut final_sample = sample_pair(&evolving, &random_control, args.ticks == 0)?;
    write_record(&mut output, &MetricsRecord::Sample(final_sample.clone()))?;

    for _ in 0..args.ticks {
        evolving.step();
        random_control.step();
        let tick = evolving.tick_count();
        if tick % args.sample_every == 0 || tick == args.ticks {
            final_sample = sample_pair(&evolving, &random_control, tick == args.ticks)?;
            write_record(&mut output, &MetricsRecord::Sample(final_sample.clone()))?;
        }
    }

    output.flush()?;
    if args.metrics.is_some() {
        eprintln!(
            "completed {} ticks: evolving={} control={}",
            args.ticks, final_sample.evolving.population, final_sample.random_control.population
        );
    }
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

fn seed(world: &mut World, founders: u32) -> Result<()> {
    let placed = world.seed_founders(founders);
    if placed != founders {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("requested {founders} founders but world capacity allowed only {placed}"),
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
