//! Reads and validates complete metrics JSONL streams.
//!
//! Rejects partial, reordered, duplicated, or version-incompatible files before
//! diagnostics interpret them as completed experiments.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

use sim_core::LayoutEra;
use sim_core::control::{
    RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL, STRUCTURAL_NULL_V2_PROTOCOL,
};

use crate::Result;
use crate::metrics::{MetricsRecord, RunHeader, RunSample, SCHEMA_VERSION, WorldMetrics};

pub(crate) struct MetricsData {
    pub header: RunHeader,
    pub samples: Vec<RunSample>,
}

fn layout_era(schema: u32) -> Option<LayoutEra> {
    match schema {
        3..=5 => Some(LayoutEra::BeforeSpecies),
        6 => Some(LayoutEra::Species),
        7..=8 => Some(LayoutEra::BirthIdentities),
        _ => None,
    }
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
                let era = layout_era(next.schema_version).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported metrics schema {}", next.schema_version),
                    )
                })?;
                let (expected_phase, controls): (_, &[&str]) = match next.schema_version {
                    3 => (1, &["randomized_at_birth"]),
                    4 => (2, &["randomized_at_birth_v2"]),
                    // The structural nulls first shipped with this schema.
                    SCHEMA_VERSION => (
                        2,
                        &[
                            RANDOMIZED_AT_BIRTH_PROTOCOL,
                            STRUCTURAL_NULL_PROTOCOL,
                            STRUCTURAL_NULL_V2_PROTOCOL,
                        ],
                    ),
                    _ => (2, &[RANDOMIZED_AT_BIRTH_PROTOCOL]),
                };
                if next.phase != expected_phase || !controls.contains(&next.control.as_str()) {
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
                // These are Phase 1's historical counts, not today's defaults.
                // Changing serde's founder defaults will also require legacy decoding
                // to restore these values for omitted fields; leave this guard fixed.
                if next.schema_version < 5
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
                next.params.validate_for_layout(era).map_err(|error| {
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
                validate_history(run, samples.last(), &sample)?;
                for metrics in [&sample.evolving, &sample.random_control] {
                    validate_species(run, metrics)?;
                    validate_complexity(run, metrics)?;
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

fn decode_record(line: &str, schema: Option<u32>) -> Result<MetricsRecord> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    // Schema 8 writes `null` to mean capture was off, so an omitted key would be
    // indistinguishable from a positive "off" claim once serde fills the default.
    if value["kind"] == "sample"
        && schema.is_some_and(|version| version >= 8)
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
    if value["kind"] == "header" {
        let schema = value["data"]["schema_version"].as_u64();
        if let Some(params) = value
            .get_mut("data")
            .and_then(|data| data.get_mut("params"))
            .and_then(serde_json::Value::as_object_mut)
        {
            if matches!(schema, Some(3..=5)) {
                if params.contains_key("species") {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "legacy metrics cannot claim species configuration",
                    )
                    .into());
                }
                // No historical classifier allocation existed. Inserting a disabled
                // policy preserves the metadata's meaning, not its buffer selection:
                // LayoutEra owns accounting independently of this normalization.
                params.insert(
                    "species".to_owned(),
                    serde_json::json!({"capacity": 0, "threshold": 0.5}),
                );
            } else if schema
                .is_some_and(|version| (6..=u64::from(SCHEMA_VERSION)).contains(&version))
            {
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
        }
    }
    Ok(serde_json::from_value(value)?)
}

fn validate_complexity(header: &RunHeader, metrics: &WorldMetrics) -> Result<()> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    if header.schema_version < 8 {
        return if metrics.complexity.is_some() {
            Err(invalid(
                "legacy metrics claim complexity distributions unavailable in their schema",
            )
            .into())
        } else {
            Ok(())
        };
    }
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
fn validate_history(
    header: &RunHeader,
    previous: Option<&RunSample>,
    sample: &RunSample,
) -> Result<()> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    let pair = [sample.evolving.history, sample.random_control.history];
    if header.schema_version < 8 {
        return if pair.iter().any(Option::is_some) {
            Err(
                invalid("legacy metrics claim history availability unavailable in their schema")
                    .into(),
            )
        } else {
            Ok(())
        };
    }
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
    if header.schema_version < 6 {
        if metrics.species.is_some() {
            return Err(invalid(
                "legacy metrics claim species observations unavailable in their schema",
            )
            .into());
        }
        return Ok(());
    }
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

    #[test]
    fn wire_schemas_map_explicitly_to_their_immutable_layout_eras() {
        for schema in [3, 4, 5] {
            assert_eq!(layout_era(schema), Some(LayoutEra::BeforeSpecies));
        }
        assert_eq!(layout_era(6), Some(LayoutEra::Species));
        for schema in [7, 8] {
            assert_eq!(layout_era(schema), Some(LayoutEra::BirthIdentities));
        }
        assert_eq!(layout_era(SCHEMA_VERSION), Some(LayoutEra::CURRENT));
        for unsupported in [0, 1, 2, 9, u32::MAX] {
            assert_eq!(layout_era(unsupported), None);
        }
    }

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

    fn set_legacy_header(records: &mut [serde_json::Value], schema: u32) {
        records[0]["data"]["schema_version"] = schema.into();
        records[0]["data"]["phase"] = if schema == 3 { 1 } else { 2 }.into();
        records[0]["data"]["control"] = match schema {
            3 => "randomized_at_birth",
            4 => "randomized_at_birth_v2",
            5 => RANDOMIZED_AT_BIRTH_PROTOCOL,
            _ => panic!("not a legacy schema"),
        }
        .into();
        records[0]["data"]["params"]
            .as_object_mut()
            .unwrap()
            .remove("species");
        for record in &mut records[1..] {
            for cohort in ["evolving", "random_control"] {
                let metrics = record["data"][cohort].as_object_mut().unwrap();
                for field in ["species", "complexity", "history"] {
                    metrics.remove(field);
                }
            }
        }
    }

    fn strip_schema_eight_observations(records: &mut [serde_json::Value]) {
        for record in &mut records[1..] {
            for cohort in ["evolving", "random_control"] {
                let metrics = record["data"][cohort].as_object_mut().unwrap();
                metrics.remove("complexity");
                metrics.remove("history");
            }
        }
    }

    #[test]
    fn schema_seven_reads_without_inventing_complexity_or_history() {
        let mut records = final_records();
        records[0]["data"]["schema_version"] = 7.into();
        strip_schema_eight_observations(&mut records);
        let data = parse_values(&records).expect("schema 7 remains readable");
        for metrics in [&data.samples[0].evolving, &data.samples[0].random_control] {
            assert_eq!(metrics.complexity, None);
            assert_eq!(metrics.history, None);
        }
        for (field, value) in [
            (
                "complexity",
                serde_json::to_value(ComplexityMetrics::default()).unwrap(),
            ),
            (
                "history",
                serde_json::json!({"capacity": 1, "retained_events": 0, "dropped_events": 0, "gaps": 0}),
            ),
        ] {
            let mut claimed = records.clone();
            for cohort in ["evolving", "random_control"] {
                claimed[1]["data"][cohort][field] = value.clone();
            }
            assert!(
                parse_values(&claimed).is_err(),
                "schema 7 cannot claim {field}"
            );
        }
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
    fn reads_current_and_explicit_legacy_protocols_without_inventing_observations() {
        let current = parse_values(&final_records()).unwrap();
        assert_eq!(current.header.control, RANDOMIZED_AT_BIRTH_PROTOCOL);
        let mut records = final_records();
        set_legacy_header(&mut records, 3);
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
        let mut expected_params = SimParams::default();
        expected_params.species.capacity = 0;
        assert_eq!(legacy.header.params, expected_params);
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
            (6, 1, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (6, 2, "randomized_at_birth_v2"),
            (6, 2, "randomized_at_birth"),
            (6, 3, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (7, 1, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (7, 2, "randomized_at_birth_v2"),
            (7, 3, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (8, 1, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (8, 3, RANDOMIZED_AT_BIRTH_PROTOCOL),
            (8, 1, STRUCTURAL_NULL_PROTOCOL),
            (8, 2, "structural_null"),
            (7, 2, STRUCTURAL_NULL_PROTOCOL),
            (5, 2, STRUCTURAL_NULL_PROTOCOL),
            (7, 2, STRUCTURAL_NULL_V2_PROTOCOL),
            (8, 2, "structural_null_v3"),
            (9, 2, RANDOMIZED_AT_BIRTH_PROTOCOL),
        ] {
            let mut records = final_records();
            if matches!(schema, 3..=5) {
                set_legacy_header(&mut records, schema);
            }
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
    fn all_historical_schemas_keep_their_tight_preclassification_budget() {
        for schema in [3, 4, 5] {
            let mut params = SimParams::default();
            params.world.max_agents = 2;
            params.plants.max_plants = 8;
            params.species.capacity = 0;
            params.storage.max_memory_bytes = params.estimated_construction_bytes().unwrap()
                - 3 * 8 * u64::from(params.world.max_agents);
            let historical_budget = params.storage.max_memory_bytes;
            let mut records = final_records();
            records[0]["data"]["params"] = serde_json::to_value(&params).unwrap();
            set_legacy_header(&mut records, schema);
            let legacy = parse_values(&records).expect("valid historical construction budget");
            assert_eq!(legacy.header.params.species.capacity, 0);
            assert_eq!(
                legacy.header.params.storage.max_memory_bytes,
                historical_budget
            );
            assert_eq!(legacy.samples[0].evolving.species, None);
            assert_eq!(legacy.samples[0].random_control.species, None);

            params.species.capacity = 256;
            assert!(
                params.validate().is_err(),
                "fixture must reject the false default classifier allocation"
            );
        }
    }

    #[test]
    fn schema_five_retains_sparse_founders_and_measured_organ_observations() {
        let mut records = final_records();
        records[0]["data"]["params"]["sensing"]["vision_rays"] = 0.into();
        records[0]["data"]["params"]["sensing"]["chemo_sensors"] = 0.into();
        records[0]["data"]["params"]["sensing"]["energy_sensors"] = 2.into();
        records[0]["data"]["params"]["brain"]["connections_per_target"] = 1.into();
        records[0]["data"]["params"]["mutation"]["organs"]["add_sensor_rate"] = 0.1.into();
        for cohort in ["evolving", "random_control"] {
            records[1]["data"][cohort]["structural_mutations"] =
                serde_json::to_value(sim_core::mutate::StructuralMutationCounts::default())
                    .unwrap();
            records[1]["data"][cohort]["structural_mutations"]["add_sensor"]["attempted"] =
                2.into();
            records[1]["data"][cohort]["structural_mutations"]["add_sensor"]["applied"] = 2.into();
        }
        set_legacy_header(&mut records, 5);
        let legacy = parse_values(&records).unwrap();
        assert_eq!(legacy.header.params.sensing.chemo_sensors, 0);
        assert_eq!(legacy.header.params.sensing.energy_sensors, 2);
        assert_eq!(legacy.header.params.brain.connections_per_target, Some(1));
        assert_eq!(legacy.header.params.species.capacity, 0);
        for metrics in [
            &legacy.samples[0].evolving,
            &legacy.samples[0].random_control,
        ] {
            assert_eq!(
                metrics
                    .structural_mutations
                    .unwrap()
                    .add_sensor
                    .unwrap()
                    .applied,
                2
            );
            assert_eq!(metrics.species, None);
        }
    }

    #[test]
    fn schema_six_preserves_species_but_does_not_pay_for_later_birth_identity_arrays() {
        let mut records = final_records();
        records[0]["data"]["schema_version"] = 6.into();
        strip_schema_eight_observations(&mut records);
        let mut params = SimParams::default();
        params.world.max_agents = 2;
        params.plants.max_plants = 8;
        params.storage.max_memory_bytes = params.estimated_construction_bytes().unwrap()
            - 3 * 8 * u64::from(params.world.max_agents);
        assert!(params.validate().is_err());
        params.validate_for_layout(LayoutEra::Species).unwrap();
        records[0]["data"]["params"] = serde_json::to_value(&params).unwrap();
        let data = parse_values(&records).unwrap();
        assert_eq!(
            data.header.params, params,
            "do not rewrite the historical budget or classifier"
        );
        assert!(data.samples[0].evolving.species.is_some());
        records[0]["data"]["schema_version"] = SCHEMA_VERSION.into();
        for cohort in ["evolving", "random_control"] {
            records[1]["data"][cohort]["complexity"] =
                serde_json::to_value(ComplexityMetrics::default()).unwrap();
            records[1]["data"][cohort]["history"] = serde_json::Value::Null;
        }
        assert!(
            parse_values(&records).is_err(),
            "current runtime must pay for its identity arrays"
        );
    }

    #[test]
    fn historical_schemas_reject_species_claims_instead_of_silently_disabling_them() {
        for schema in [3, 4, 5] {
            for policy in [
                serde_json::Value::Null,
                serde_json::json!({}),
                serde_json::json!({"capacity": 0, "threshold": 0.5}),
                serde_json::json!({"capacity": 256, "threshold": 0.5}),
            ] {
                let mut records = final_records();
                set_legacy_header(&mut records, schema);
                records[0]["data"]["params"]["species"] = policy;
                let error = parse_values(&records)
                    .err()
                    .expect("accepted legacy classification");
                assert!(
                    error
                        .to_string()
                        .contains("legacy metrics cannot claim species configuration")
                );
            }
            for cohort in ["evolving", "random_control"] {
                let mut records = final_records();
                set_legacy_header(&mut records, schema);
                records[1]["data"][cohort]["species"] = serde_json::Value::Null;
                parse_values(&records).expect("null means historically unavailable");
                records[1]["data"][cohort]["species"] =
                    serde_json::to_value(SpeciesMetrics::default()).unwrap();
                let error = parse_values(&records)
                    .err()
                    .expect("accepted invented measured zeroes");
                assert!(
                    error
                        .to_string()
                        .contains("species observations unavailable")
                );
            }
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
            set_legacy_header(&mut records, 5);
            parse_values(&records).expect("schema 5 supports nondefault M3 configuration");
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
}
