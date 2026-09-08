//! Reads and validates complete metrics JSONL streams.
//!
//! Rejects partial, reordered, duplicated, or version-incompatible files before
//! diagnostics interpret them as completed experiments.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;

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
                if !matches!(next.schema_version, 3 | 4 | SCHEMA_VERSION) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported metrics schema {}", next.schema_version),
                    )
                    .into());
                }
                let (expected_phase, expected_control) = match next.schema_version {
                    3 => (1, "randomized_at_birth"),
                    4 => (2, "randomized_at_birth_v2"),
                    _ => (2, RANDOMIZED_AT_BIRTH_PROTOCOL),
                };
                if next.phase != expected_phase || next.control != expected_control {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported metrics protocol combination: schema {}, phase {}, control {}",
                            next.schema_version, next.phase, next.control
                        ),
                    )
                    .into());
                }
                let structural = &next.params.mutation.structural;
                if next.schema_version == 3
                    && [
                        structural.remove_connection_rate,
                        structural.remove_neuron_rate,
                        structural.toggle_connection_rate,
                        structural.add_connection_rate,
                        structural.add_neuron_rate,
                    ]
                    .iter()
                    .any(|&rate| rate != 0.0)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "legacy randomized_at_birth protocol cannot have nonzero structural mutation rates",
                    ).into());
                }
                if next.schema_version < SCHEMA_VERSION
                    && (next.params.mutation.organs.remove_sensor_rate != 0.0
                        || next.params.mutation.organs.add_sensor_rate != 0.0
                        || next.params.sensing.chemo_sensors != 1
                        || next.params.sensing.energy_sensors != 1
                        || next.params.brain.connections_per_target.is_some())
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "legacy metrics cannot have nonzero organ mutation rates or nondefault M3 founder composition",
                    ).into());
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
                header = Some(*next);
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
                for metrics in [&sample.evolving, &sample.random_control] {
                    if let Some(counts) = metrics.structural_mutations
                        && (run.schema_version == 3
                            || (run.schema_version == 4
                                && (counts.remove_sensor.is_some() || counts.add_sensor.is_some())))
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "legacy metrics claim mutation observations unavailable in their schema",
                        ).into());
                    }
                }
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
                samples.push(*sample);
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

    fn final_records() -> [serde_json::Value; 2] {
        [
            serde_json::to_value(MetricsRecord::Header(Box::new(RunHeader {
                schema_version: SCHEMA_VERSION,
                sim_version: "test".to_owned(),
                source_revision: "test".to_owned(),
                phase: 2,
                seed: "42".to_owned(),
                ticks: 0,
                founders: 1,
                sample_every: 5,
                params: SimParams::default(),
                control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
            })))
            .unwrap(),
            serde_json::to_value(MetricsRecord::Sample(Box::new(RunSample {
                tick: 0,
                evolving: WorldMetrics::default(),
                random_control: WorldMetrics::default(),
                final_state_hashes: Some(StateHashes {
                    evolving: "1".to_owned(),
                    random_control: "1".to_owned(),
                }),
            })))
            .unwrap(),
        ]
    }

    fn parse_values(records: &[serde_json::Value]) -> Result<MetricsData> {
        let jsonl = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        parse_metrics(Cursor::new(jsonl))
    }

    fn set_legacy_header(records: &mut [serde_json::Value], schema: u32) {
        records[0]["data"]["schema_version"] = schema.into();
        records[0]["data"]["phase"] = if schema == 3 { 1 } else { 2 }.into();
        records[0]["data"]["control"] = if schema == 3 {
            "randomized_at_birth"
        } else {
            "randomized_at_birth_v2"
        }
        .into();
    }

    #[test]
    fn reads_current_and_explicit_legacy_protocols_without_inventing_observations() {
        let current = parse_values(&final_records()).unwrap();
        assert_eq!(current.header.control, RANDOMIZED_AT_BIRTH_PROTOCOL);
        let mut records = final_records();
        records[0]["data"]["schema_version"] = 3.into();
        records[0]["data"]["phase"] = 1.into();
        records[0]["data"]["control"] = "randomized_at_birth".into();
        records[0]["data"]["params"]["mutation"]
            .as_object_mut()
            .unwrap()
            .remove("structural");
        for cohort in ["evolving", "random_control"] {
            records[1]["data"][cohort]
                .as_object_mut()
                .unwrap()
                .remove("structural_mutations");
        }
        let legacy = parse_values(&records).unwrap();
        assert_eq!(legacy.header.control, "randomized_at_birth");
        assert_eq!(
            legacy.header.params.mutation.structural,
            SimParams::default().mutation.structural,
        );
        assert_eq!(legacy.samples[0].evolving.structural_mutations, None);
        assert_eq!(legacy.samples[0].random_control.structural_mutations, None);

        let mut records = final_records();
        set_legacy_header(&mut records, 4);
        records[0]["data"]["params"]["mutation"]
            .as_object_mut()
            .unwrap()
            .remove("organs");
        for field in ["chemo_sensors", "energy_sensors"] {
            records[0]["data"]["params"]["sensing"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        records[0]["data"]["params"]["brain"]
            .as_object_mut()
            .unwrap()
            .remove("connections_per_target");
        for cohort in ["evolving", "random_control"] {
            let mut counts =
                serde_json::to_value(sim_core::mutate::StructuralMutationCounts::default())
                    .unwrap();
            counts.as_object_mut().unwrap().remove("remove_sensor");
            counts.as_object_mut().unwrap().remove("add_sensor");
            counts["add_neuron"]["attempted"] = 2.into();
            counts["add_neuron"]["applied"] = 2.into();
            records[1]["data"][cohort]["structural_mutations"] = counts;
        }
        let legacy = parse_values(&records).unwrap();
        assert_eq!(legacy.header.control, "randomized_at_birth_v2");
        assert_eq!(legacy.header.params, SimParams::default());
        for cohort in [
            &legacy.samples[0].evolving,
            &legacy.samples[0].random_control,
        ] {
            let counts = cohort.structural_mutations.unwrap();
            assert_eq!(counts.add_neuron.applied, 2);
            assert_eq!(counts.remove_sensor, None);
            assert_eq!(counts.add_sensor, None);
        }
    }

    #[test]
    fn rejects_unknown_or_mixed_protocol_versions() {
        for (schema, phase, control) in [
            (3, 2, "randomized_at_birth"),
            (3, 1, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (4, 1, "randomized_at_birth_v2"),
            (4, 2, "randomized_at_birth"),
            (4, 2, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (4, 2, "unknown"),
            (4, 3, "randomized_at_birth_v2"),
            (5, 1, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (5, 2, "randomized_at_birth_v2"),
            (5, 2, "randomized_at_birth"),
            (5, 3, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (6, 2, RANDOMIZED_AT_BIRTH_PROTOCOL),
        ] {
            let mut records = final_records();
            records[0]["data"]["schema_version"] = schema.into();
            records[0]["data"]["phase"] = phase.into();
            records[0]["data"]["control"] = control.into();
            assert!(
                parse_values(&records).is_err(),
                "accepted {schema}/{phase}/{control}"
            );
        }
    }

    #[test]
    fn legacy_control_rejects_each_nonzero_structural_rate_but_current_accepts_it() {
        for rate in [
            "remove_connection_rate",
            "remove_neuron_rate",
            "toggle_connection_rate",
            "add_connection_rate",
            "add_neuron_rate",
        ] {
            let mut records = final_records();
            records[0]["data"]["params"]["mutation"]["structural"][rate] = 0.1.into();
            parse_values(&records).expect("current structural configuration");
            set_legacy_header(&mut records, 4);
            parse_values(&records).expect("schema 4 supports neural structural mutation");
            records[0]["data"]["schema_version"] = 3.into();
            records[0]["data"]["phase"] = 1.into();
            records[0]["data"]["control"] = "randomized_at_birth".into();
            let error = parse_values(&records)
                .err()
                .expect("legacy structural rate accepted");
            assert!(
                error
                    .to_string()
                    .contains("nonzero structural mutation rates"),
                "{error}"
            );
        }
    }

    #[test]
    fn legacy_protocols_reject_m3_behavior_but_current_accepts_it() {
        for (path, value) in [
            (
                "/mutation/organs/remove_sensor_rate",
                serde_json::json!(0.1),
            ),
            ("/mutation/organs/add_sensor_rate", serde_json::json!(0.1)),
            ("/sensing/chemo_sensors", serde_json::json!(0)),
            ("/sensing/energy_sensors", serde_json::json!(2)),
            ("/brain/connections_per_target", serde_json::json!(0)),
            ("/brain/connections_per_target", serde_json::json!(1)),
        ] {
            let mut records = final_records();
            *records[0]["data"]["params"].pointer_mut(path).unwrap() = value;
            parse_values(&records).expect("current M3 configuration");
            for schema in [3, 4] {
                set_legacy_header(&mut records, schema);
                let error = parse_values(&records)
                    .err()
                    .expect("accepted legacy M3 behavior");
                assert!(
                    error.to_string().contains("M3 founder composition"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn legacy_sensor_observations_are_unknown_never_measured_zero() {
        for schema in [3, 4] {
            for cohort in ["evolving", "random_control"] {
                for operator in ["remove_sensor", "add_sensor"] {
                    let mut records = final_records();
                    set_legacy_header(&mut records, schema);
                    let mut counts =
                        serde_json::to_value(sim_core::mutate::StructuralMutationCounts::default())
                            .unwrap();
                    counts["remove_sensor"] = serde_json::Value::Null;
                    counts["add_sensor"] = serde_json::Value::Null;
                    records[1]["data"][cohort]["structural_mutations"] = counts;
                    if schema == 4 {
                        parse_values(&records).expect("explicit null remains unavailable");
                    }
                    records[1]["data"][cohort]["structural_mutations"][operator] =
                        serde_json::to_value(sim_core::mutate::OperatorCounts::default()).unwrap();
                    let error = parse_values(&records)
                        .err()
                        .expect("legacy claimed measured sensor observations");
                    assert!(
                        error.to_string().contains("observations unavailable"),
                        "{error}"
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_a_truncated_metrics_stream() {
        let header = RunHeader {
            schema_version: SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test".to_owned(),
            phase: 2,
            seed: "42".to_owned(),
            ticks: 10,
            founders: 1,
            sample_every: 5,
            params: SimParams::default(),
            control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
        };
        let records = [
            MetricsRecord::Header(Box::new(header)),
            MetricsRecord::Sample(Box::new(RunSample {
                tick: 0,
                evolving: WorldMetrics::default(),
                random_control: WorldMetrics::default(),
                final_state_hashes: None,
            })),
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
            phase: 2,
            seed: "42".to_owned(),
            ticks: 0,
            founders: 1,
            sample_every: 5,
            params: SimParams::default(),
            control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
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
            MetricsRecord::Header(Box::new(final_header)),
            MetricsRecord::Sample(Box::new(final_sample.clone())),
            MetricsRecord::Sample(Box::new(final_sample)),
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
            MetricsRecord::Header(Box::new(RunHeader {
                schema_version: SCHEMA_VERSION,
                sim_version: "test".to_owned(),
                source_revision: "test".to_owned(),
                phase: 2,
                seed: "42".to_owned(),
                ticks: 0,
                founders: 1,
                sample_every: 5,
                params,
                control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
            })),
            MetricsRecord::Sample(Box::new(RunSample {
                tick: 0,
                evolving: WorldMetrics::default(),
                random_control: WorldMetrics::default(),
                final_state_hashes: Some(StateHashes {
                    evolving: "1".to_owned(),
                    random_control: "1".to_owned(),
                }),
            })),
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
                .contains("invalid SimParams: world.dt must be finite and positive")
        );
    }

    #[test]
    fn rejects_metrics_schemas_without_current_storage_observations() {
        for version in [1, 2] {
            let header = MetricsRecord::Header(Box::new(RunHeader {
                schema_version: version,
                sim_version: "test".to_owned(),
                source_revision: "test".to_owned(),
                phase: 1,
                seed: "42".to_owned(),
                ticks: 0,
                founders: 1,
                sample_every: 5,
                params: SimParams::default(),
                control: "randomized_at_birth".to_owned(),
            }));
            let jsonl = serde_json::to_string(&header).unwrap();
            let error = parse_metrics(Cursor::new(jsonl))
                .err()
                .expect("old metrics schema was accepted");
            assert!(
                error
                    .to_string()
                    .contains(&format!("unsupported metrics schema {version}"))
            );
        }
    }
}
