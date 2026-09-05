//! Reads and validates complete metrics JSONL streams.
//!
//! Rejects partial, reordered, duplicated, or version-incompatible files before
//! diagnostics interpret them as completed experiments.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

use crate::Result;
use crate::metrics::{MetricsRecord, RunHeader, RunSample, SCHEMA_VERSION};

pub(crate) struct MetricsData {
    pub header: RunHeader,
    pub samples: Vec<RunSample>,
}

pub(crate) fn read_metrics(path: &std::path::Path) -> Result<MetricsData> {
    let input: Box<dyn BufRead> = if path == std::path::Path::new("-") {
        Box::new(BufReader::new(io::stdin()))
    } else {
        Box::new(BufReader::new(File::open(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not open metrics {}: {error}", path.display()),
            )
        })?))
    };
    parse_metrics(input)
}

fn parse_metrics(input: impl BufRead) -> Result<MetricsData> {
    let mut header = None;
    let mut samples = Vec::new();
    for (line_number, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: MetricsRecord = serde_json::from_str(&line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("line {}: {error}", line_number + 1),
            )
        })?;
        match record {
            MetricsRecord::Header(next) if header.is_none() && samples.is_empty() => {
                if next.schema_version != SCHEMA_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported metrics schema {}", next.schema_version),
                    )
                    .into());
                }
                if next.phase != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported simulation phase {}", next.phase),
                    )
                    .into());
                }
                if next.control != "randomized_at_birth" {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported control protocol {}", next.control),
                    )
                    .into());
                }
                if next.seed.parse::<u64>().is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "metrics seed is not a decimal u64",
                    )
                    .into());
                }
                if next.sample_every == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "metrics sample interval must be non-zero",
                    )
                    .into());
                }
                next.params.validate().map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("metrics header contains {error}"),
                    )
                })?;
                header = Some(next);
            }
            MetricsRecord::Header(_) if !samples.is_empty() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "metrics header must be the first record",
                )
                .into());
            }
            MetricsRecord::Header(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "metrics file contains more than one header",
                )
                .into());
            }
            MetricsRecord::Sample(sample) => {
                let run = header.as_ref().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "metrics header must be the first record",
                    )
                })?;
                if samples
                    .last()
                    .is_some_and(|last: &RunSample| last.tick == run.ticks)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "metrics file contains records after the final sample",
                    )
                    .into());
                }
                let expected = samples.last().map_or(0, |last: &RunSample| {
                    last.tick.saturating_add(run.sample_every).min(run.ticks)
                });
                if sample.tick != expected {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("expected sample at tick {expected}, found {}", sample.tick),
                    )
                    .into());
                }
                let final_sample = sample.tick == run.ticks;
                if sample.final_state_hashes.is_some() != final_sample {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        if final_sample {
                            "final sample is missing state hashes"
                        } else {
                            "state hashes may appear only on the final sample"
                        },
                    )
                    .into());
                }
                samples.push(sample);
            }
        }
    }
    let header = header
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "metrics file has no header"))?;
    if samples.is_empty() {
        return Err(
            io::Error::new(io::ErrorKind::InvalidData, "metrics file has no samples").into(),
        );
    }
    if samples
        .last()
        .is_none_or(|sample| sample.tick != header.ticks)
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("metrics file ended before final tick {}", header.ticks),
        )
        .into());
    }
    Ok(MetricsData { header, samples })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use sim_core::params::SimParams;

    use super::*;
    use crate::metrics::{RunSample, StateHashes, WorldMetrics};

    #[test]
    fn rejects_a_truncated_metrics_stream() {
        let header = RunHeader {
            schema_version: SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test".to_owned(),
            phase: 1,
            seed: "42".to_owned(),
            ticks: 10,
            founders: 1,
            sample_every: 5,
            params: SimParams::default(),
            control: "randomized_at_birth".to_owned(),
        };
        let records = [
            MetricsRecord::Header(header),
            MetricsRecord::Sample(RunSample {
                tick: 0,
                evolving: WorldMetrics::default(),
                random_control: WorldMetrics::default(),
                final_state_hashes: None,
            }),
        ];
        let mut jsonl = String::new();
        for record in records {
            jsonl.push_str(&serde_json::to_string(&record).unwrap());
            jsonl.push('\n');
        }
        let error = parse_metrics(Cursor::new(jsonl))
            .err()
            .expect("truncated stream was accepted");
        assert!(error.to_string().contains("before final tick 10"));
    }

    #[test]
    fn rejects_records_after_the_final_sample() {
        let final_header = RunHeader {
            schema_version: SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test".to_owned(),
            phase: 1,
            seed: "42".to_owned(),
            ticks: 0,
            founders: 1,
            sample_every: 5,
            params: SimParams::default(),
            control: "randomized_at_birth".to_owned(),
        };
        let final_sample = RunSample {
            tick: 0,
            evolving: WorldMetrics::default(),
            random_control: WorldMetrics::default(),
            final_state_hashes: Some(StateHashes {
                evolving: "1".to_owned(),
                random_control: "1".to_owned(),
            }),
        };
        let records = [
            MetricsRecord::Header(final_header),
            MetricsRecord::Sample(final_sample.clone()),
            MetricsRecord::Sample(final_sample),
        ];
        let jsonl = records
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        let error = parse_metrics(Cursor::new(jsonl))
            .err()
            .expect("record after final sample was accepted");
        assert!(error.to_string().contains("after the final sample"));
    }

    #[test]
    fn rejects_invalid_header_params() {
        let mut params = SimParams::default();
        params.world.dt = 0.0;
        let records = [
            MetricsRecord::Header(RunHeader {
                schema_version: SCHEMA_VERSION,
                sim_version: "test".to_owned(),
                source_revision: "test".to_owned(),
                phase: 1,
                seed: "42".to_owned(),
                ticks: 0,
                founders: 1,
                sample_every: 5,
                params,
                control: "randomized_at_birth".to_owned(),
            }),
            MetricsRecord::Sample(RunSample {
                tick: 0,
                evolving: WorldMetrics::default(),
                random_control: WorldMetrics::default(),
                final_state_hashes: Some(StateHashes {
                    evolving: "1".to_owned(),
                    random_control: "1".to_owned(),
                }),
            }),
        ];
        let jsonl = records
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        let error = parse_metrics(Cursor::new(jsonl))
            .err()
            .expect("invalid params were accepted");
        assert!(
            error
                .to_string()
                .contains("invalid SimParams: world.dt must be positive")
        );
    }
}
