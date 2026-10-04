//! Reads and validates complete metrics JSONL streams.
//!
//! Rejects partial, reordered, duplicated, or version-incompatible files before
//! diagnostics interpret them as completed experiments.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

use sim_core::control::{
    RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL, STRUCTURAL_NULL_V2_PROTOCOL,
};

use crate::Result;
use crate::metrics::{MetricsRecord, RunHeader, RunSample, SCHEMA_VERSION, WorldMetrics};

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
        let schema = header.as_ref().map(|run: &RunHeader| run.schema_version);
        let record = decode_record(&line, schema).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("line {}: {error}", line_number + 1),
            )
        })?;
        match record {
            MetricsRecord::Header(next) if header.is_none() && samples.is_empty() => {
                // Only the current schema is read; older metrics files are not supported.
                if next.schema_version != SCHEMA_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported metrics schema {} (this build reads schema {SCHEMA_VERSION})",
                            next.schema_version
                        ),
                    )
                    .into());
                }
                let controls = [
                    RANDOMIZED_AT_BIRTH_PROTOCOL,
                    STRUCTURAL_NULL_PROTOCOL,
                    STRUCTURAL_NULL_V2_PROTOCOL,
                ];
                if next.phase != 2 || !controls.contains(&next.control.as_str()) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported metrics protocol combination: phase {}, control {}",
                            next.phase, next.control
                        ),
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
                validate_history(samples.last(), &sample)?;
                for metrics in [&sample.evolving, &sample.random_control] {
                    validate_species(run, metrics)?;
                    validate_complexity(metrics)?;
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

fn decode_record(line: &str, schema: Option<u32>) -> Result<MetricsRecord> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    // `null` means capture was off, so an omitted key would be indistinguishable
    // from a positive "off" claim once serde fills the default.
    if value["kind"] == "sample"
        && schema.is_some()
        && ["evolving", "random_control"].iter().any(|cohort| {
            value["data"][cohort]
                .as_object()
                .is_some_and(|metrics| !metrics.contains_key("history"))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current metrics require explicit history availability, even when null",
        )
        .into());
    }
    if value["kind"] == "header"
        && let Some(params) = value
            .get_mut("data")
            .and_then(|data| data.get_mut("params"))
            .and_then(serde_json::Value::as_object_mut)
    {
        // Mutation rates shipped at zero until Phase 2 M8, and oscillator addition
        // did not exist before it; a schema-8 file written earlier omits some rates.
        // It ran without those mutations, so today's defaults must not rewrite it.
        for (section, fields) in [
            (
                "structural",
                &[
                    "remove_connection_rate",
                    "remove_neuron_rate",
                    "toggle_connection_rate",
                    "add_connection_rate",
                    "add_neuron_rate",
                    "add_oscillator_rate",
                ][..],
            ),
            ("organs", &["remove_sensor_rate", "add_sensor_rate"][..]),
        ] {
            if let Some(object) = params
                .entry("mutation")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .and_then(|mutation| {
                    mutation
                        .entry(section)
                        .or_insert_with(|| serde_json::json!({}))
                        .as_object_mut()
                })
            {
                for field in fields {
                    object
                        .entry(*field)
                        .or_insert_with(|| serde_json::json!(0.0));
                }
            }
        }
        for (section, fields) in [
            ("species", &["capacity", "threshold"][..]),
            (
                "distance",
                &[
                    "disjoint_coefficient",
                    "excess_coefficient",
                    "weight_coefficient",
                ][..],
            ),
        ] {
            if fields.iter().any(|field| {
                params
                    .get(section)
                    .and_then(serde_json::Value::as_object)
                    .is_none_or(|object| !object.contains_key(*field))
            }) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("current metrics require explicit {section} configuration"),
                )
                .into());
            }
        }
    }
    Ok(serde_json::from_value(value)?)
}

fn validate_complexity(metrics: &WorldMetrics) -> Result<()> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    let complexity = metrics
        .complexity
        .as_ref()
        .ok_or_else(|| invalid("current metrics require complexity distributions"))?;
    if !complexity.is_consistent() || (metrics.population == 0) != complexity.is_empty() {
        return Err(invalid(
            "complexity distributions must be ordered, bounded, and empty exactly when no agent lives",
        )
        .into());
    }
    Ok(())
}

/// History capture is fixed for a run: both cohorts share it, and its counts only grow.
fn validate_history(previous: Option<&RunSample>, sample: &RunSample) -> Result<()> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    let pair = [sample.evolving.history, sample.random_control.history];
    match pair {
        [None, None] => {}
        [Some(evolving), Some(control)] => {
            if evolving.capacity != control.capacity
                || pair.iter().flatten().any(|history| {
                    history.capacity == 0
                        || history.gaps > history.dropped_events
                        || (history.gaps == 0) != (history.dropped_events == 0)
                })
            {
                return Err(invalid(
                    "history availability requires one nonzero capacity and a gap for every drop",
                )
                .into());
            }
        }
        _ => return Err(invalid("history availability must cover both cohorts or neither").into()),
    }
    if let Some(previous) = previous {
        let before = [previous.evolving.history, previous.random_control.history];
        let consistent = before
            .iter()
            .zip(&pair)
            .all(|(before, after)| match (before, after) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    before.capacity == after.capacity
                        && before.retained_events <= after.retained_events
                        && before.dropped_events <= after.dropped_events
                        && before.gaps <= after.gaps
                }
                _ => false,
            });
        if !consistent {
            return Err(invalid(
                "history availability must stay enabled or disabled and never decrease",
            )
            .into());
        }
    }
    Ok(())
}

fn validate_species(header: &RunHeader, metrics: &WorldMetrics) -> Result<()> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let species = metrics
        .species
        .as_ref()
        .ok_or_else(|| invalid("current metrics require authoritative species populations"))?;
    if metrics.population > header.params.world.max_agents
        || species.populations.len() as u64 > u64::from(header.params.species.capacity)
    {
        return Err(
            invalid("species metrics exceed configured population or species capacity").into(),
        );
    }
    let mut previous = None;
    let mut population = u64::from(species.unclassified_population);
    for row in &species.populations {
        if row.species_id.is_null()
            || previous.is_some_and(|id| row.species_id <= id)
            || row.population == 0
        {
            return Err(invalid(
                "species populations require ascending unique non-NULL IDs and positive counts",
            )
            .into());
        }
        previous = Some(row.species_id);
        population += u64::from(row.population);
    }
    if population != u64::from(metrics.population) {
        return Err(invalid(
            "species populations plus unclassified population must equal population",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use sim_core::params::SimParams;

    use super::*;
    use crate::metrics::{
        ComplexityMetrics, RunSample, SizeDistribution, SpeciesMetrics, StateHashes, WorldMetrics,
    };

    fn empty_world_metrics() -> WorldMetrics {
        WorldMetrics {
            species: Some(SpeciesMetrics::default()),
            complexity: Some(ComplexityMetrics::default()),
            ..Default::default()
        }
    }

    fn living_complexity() -> serde_json::Value {
        let genes = SizeDistribution {
            min: 3,
            p25: 3,
            median: 4,
            p75: 5,
            max: 6,
            mean: 4.25,
        };
        serde_json::to_value(ComplexityMetrics {
            genome_genes: genes.clone(),
            neurons: SizeDistribution {
                min: 1,
                p25: 1,
                median: 1,
                p75: 2,
                max: 2,
                mean: 1.5,
            },
            connections: SizeDistribution {
                min: 1,
                p25: 1,
                median: 2,
                p75: 2,
                max: 3,
                mean: 2.0,
            },
            enabled_connections: SizeDistribution {
                min: 0,
                p25: 1,
                median: 1,
                p75: 2,
                max: 3,
                mean: 1.5,
            },
        })
        .unwrap()
    }

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
                evolving: empty_world_metrics(),
                random_control: empty_world_metrics(),
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

    #[test]
    fn current_schema_requires_consistent_complexity() {
        for cohort in ["evolving", "random_control"] {
            let mut records = final_records();
            records[1]["data"][cohort]
                .as_object_mut()
                .unwrap()
                .remove("complexity");
            assert!(parse_values(&records).is_err(), "missing complexity");

            let mut records = final_records();
            records[1]["data"][cohort]["complexity"] = living_complexity();
            assert!(
                parse_values(&records).is_err(),
                "an empty population cannot have genome sizes"
            );
            records[1]["data"][cohort]["population"] = 1.into();
            records[1]["data"][cohort]["species"]["unclassified_population"] = 1.into();
            parse_values(&records).expect("living population with sizes");
            for (path, value) in [
                ("/genome_genes/p25", serde_json::json!(5)),
                ("/enabled_connections/max", serde_json::json!(4)),
                ("/neurons/mean", serde_json::json!(9.0)),
            ] {
                let mut invalid = records.clone();
                *invalid[1]["data"][cohort]["complexity"]
                    .pointer_mut(path)
                    .unwrap() = value;
                assert!(parse_values(&invalid).is_err(), "accepted {path}");
            }
        }
    }

    fn history_records(history: [serde_json::Value; 2]) -> Vec<serde_json::Value> {
        let [mut header, mut second] = final_records();
        header["data"]["ticks"] = 5.into();
        let mut first = second.clone();
        first["data"]["final_state_hashes"] = serde_json::Value::Null;
        second["data"]["tick"] = 5.into();
        for (record, value) in [(&mut first, &history[0]), (&mut second, &history[1])] {
            for cohort in ["evolving", "random_control"] {
                record["data"][cohort]["history"] = value.clone();
            }
        }
        vec![header, first, second]
    }

    #[test]
    fn history_availability_is_fixed_per_run_paired_and_monotonic() {
        let complete = serde_json::json!({"capacity": 4, "retained_events": 2, "dropped_events": 0, "gaps": 0});
        let later = serde_json::json!({"capacity": 4, "retained_events": 3, "dropped_events": 2, "gaps": 1});
        parse_values(&history_records([complete.clone(), later.clone()])).expect("growing counts");
        parse_values(&history_records([
            serde_json::Value::Null,
            serde_json::Value::Null,
        ]))
        .expect("capture off");
        for history in [
            [later.clone(), complete.clone()],
            [complete.clone(), serde_json::Value::Null],
            [serde_json::Value::Null, complete.clone()],
            [
                complete.clone(),
                serde_json::json!({"capacity": 8, "retained_events": 2, "dropped_events": 0, "gaps": 0}),
            ],
            [
                complete.clone(),
                serde_json::json!({"capacity": 4, "retained_events": 2, "dropped_events": 1, "gaps": 0}),
            ],
            [
                serde_json::json!({"capacity": 0, "retained_events": 0, "dropped_events": 0, "gaps": 0}),
                serde_json::json!({"capacity": 0, "retained_events": 0, "dropped_events": 0, "gaps": 0}),
            ],
        ] {
            assert!(
                parse_values(&history_records(history.clone())).is_err(),
                "accepted {history:?}"
            );
        }
        let mut unpaired = history_records([complete.clone(), complete]);
        unpaired[1]["data"]["random_control"]["history"] = serde_json::Value::Null;
        assert!(
            parse_values(&unpaired).is_err(),
            "one cohort without capture"
        );
        for cohort in ["evolving", "random_control"] {
            let mut omitted = history_records([serde_json::Value::Null, serde_json::Value::Null]);
            omitted[2]["data"][cohort]
                .as_object_mut()
                .unwrap()
                .remove("history");
            let error = parse_values(&omitted)
                .err()
                .expect("omitted history accepted");
            assert!(error.to_string().contains("explicit history availability"));
        }
    }

    #[test]
    fn reads_current_structural_null_controls() {
        for protocol in [STRUCTURAL_NULL_PROTOCOL, STRUCTURAL_NULL_V2_PROTOCOL] {
            let mut records = final_records();
            records[0]["data"]["control"] = protocol.into();
            assert_eq!(parse_values(&records).unwrap().header.control, protocol);
        }
    }

    #[test]
    fn current_schema_requires_explicit_classification_metadata() {
        for section in ["species", "distance"] {
            let records = final_records();
            let fields: Vec<_> = records[0]["data"]["params"][section]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for field in fields {
                let mut records = records.clone();
                records[0]["data"]["params"][section]
                    .as_object_mut()
                    .unwrap()
                    .remove(&field);
                let error = parse_values(&records)
                    .err()
                    .expect("accepted defaulted classifier metadata");
                assert!(
                    error
                        .to_string()
                        .contains(&format!("explicit {section} configuration"))
                );
            }
        }
    }

    #[test]
    fn malformed_headers_return_errors_instead_of_panicking_during_version_decoding() {
        for data in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!("not header data"),
            serde_json::json!([]),
            serde_json::json!({"schema_version": 3, "params": null}),
            serde_json::json!({"schema_version": 6, "params": []}),
        ] {
            let record = serde_json::json!({"kind": "header", "data": data});
            assert!(parse_values(&[record]).is_err());
        }
    }

    #[test]
    fn current_schema_requires_species_state_but_not_event_observations() {
        for cohort in ["evolving", "random_control"] {
            let mut records = final_records();
            records[1]["data"][cohort]["species"]
                .as_object_mut()
                .unwrap()
                .remove("events");
            let data = parse_values(&records).expect("event observation is optional");
            assert_eq!(
                data.samples[0].evolving.species.as_ref().unwrap().events,
                None
            );
            assert_eq!(
                data.samples[0]
                    .random_control
                    .species
                    .as_ref()
                    .unwrap()
                    .events,
                None
            );
            records[1]["data"][cohort]["species"] = serde_json::Value::Null;
            assert!(parse_values(&records).is_err());
            records[1]["data"][cohort]
                .as_object_mut()
                .unwrap()
                .remove("species");
            assert!(parse_values(&records).is_err());
        }
    }

    #[test]
    fn validates_species_totals_sorted_historical_ids_and_capacity_per_cohort() {
        for cohort in ["evolving", "random_control"] {
            let mut valid = final_records();
            valid[0]["data"]["params"]["species"]["capacity"] = 2.into();
            valid[1]["data"][cohort]["population"] = 4.into();
            valid[1]["data"][cohort]["complexity"] = living_complexity();
            valid[1]["data"][cohort]["species"] = serde_json::json!({
                "populations": [
                    {"species_id": 5, "population": 1},
                    {"species_id": 19, "population": 2}
                ],
                "unclassified_population": 1,
                "events": null
            });
            parse_values(&valid).expect("historical IDs are not bounded by active capacity");
            for (path, value) in [
                ("/populations/0/population", serde_json::json!(0)),
                ("/populations/0/population", serde_json::json!(2)),
                ("/populations/0/population", serde_json::json!(u32::MAX)),
                ("/populations/1/species_id", serde_json::json!(5)),
                ("/populations/1/species_id", serde_json::json!(4)),
                ("/populations/1/species_id", serde_json::json!(u32::MAX)),
                ("/unclassified_population", serde_json::json!(0)),
                ("/unclassified_population", serde_json::json!(u32::MAX)),
            ] {
                let mut records = valid.clone();
                *records[1]["data"][cohort]["species"]
                    .pointer_mut(path)
                    .unwrap() = value;
                assert!(
                    parse_values(&records).is_err(),
                    "accepted invalid {cohort}{path}"
                );
            }
            for capacity in [0, 1] {
                let mut records = valid.clone();
                records[0]["data"]["params"]["species"]["capacity"] = capacity.into();
                assert!(
                    parse_values(&records).is_err(),
                    "accepted excess active species"
                );
            }
            valid[0]["data"]["params"]["world"]["max_agents"] = 3.into();
            assert!(
                parse_values(&valid).is_err(),
                "accepted population above world capacity"
            );
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
                evolving: empty_world_metrics(),
                random_control: empty_world_metrics(),
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
            evolving: empty_world_metrics(),
            random_control: empty_world_metrics(),
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
                evolving: empty_world_metrics(),
                random_control: empty_world_metrics(),
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

    #[test]
    fn only_the_current_schema_and_protocols_are_read() {
        for schema in [7, 9] {
            let mut records = final_records();
            records[0]["data"]["schema_version"] = schema.into();
            let error = parse_values(&records).err().expect("other schemas refused");
            assert!(
                error.to_string().contains("unsupported metrics schema"),
                "{error}"
            );
        }
        for (field, value) in [
            ("phase", serde_json::json!(1)),
            ("control", "unknown".into()),
        ] {
            let mut records = final_records();
            records[0]["data"][field] = value;
            assert!(parse_values(&records).is_err(), "{field}");
        }
        for control in [STRUCTURAL_NULL_PROTOCOL, STRUCTURAL_NULL_V2_PROTOCOL] {
            let mut records = final_records();
            records[0]["data"]["control"] = control.into();
            parse_values(&records).unwrap();
        }
    }

    #[test]
    fn rates_and_counts_older_schema_eight_files_omit_are_zero_and_unknown() {
        let mut records = final_records();
        records[0]["data"]["params"]["mutation"]["structural"]
            .as_object_mut()
            .unwrap()
            .remove("add_oscillator_rate");
        records[0]["data"]["params"]["mutation"]["organs"]
            .as_object_mut()
            .unwrap()
            .remove("add_sensor_rate");
        for cohort in ["evolving", "random_control"] {
            let mut counts =
                serde_json::to_value(sim_core::mutate::StructuralMutationCounts::default())
                    .unwrap();
            counts.as_object_mut().unwrap().remove("add_oscillator");
            records[1]["data"][cohort]["structural_mutations"] = counts;
        }
        let data = parse_values(&records).unwrap();
        let mutation = &data.header.params.mutation;
        assert_eq!(mutation.structural.add_oscillator_rate, 0.0);
        assert_eq!(mutation.organs.add_sensor_rate, 0.0);
        assert_eq!(
            data.samples[0]
                .evolving
                .structural_mutations
                .unwrap()
                .add_oscillator,
            None
        );
    }
}
