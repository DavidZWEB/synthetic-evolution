//! Reads and validates complete metrics JSONL streams.
//!
//! Rejects partial, reordered, duplicated, or version-incompatible files before
//! diagnostics interpret them as completed experiments.

use std::fs::File;
use std::io::{self, BufRead, BufReader};

use sim_core::control::{RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL};

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
                let controls = [RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL];
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
                validate_retune(&next)?;
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
        && let Some(retune) = value
            .get_mut("data")
            .and_then(|data| data.get_mut("retune"))
            .and_then(|retune| retune.get_mut("params"))
            .and_then(serde_json::Value::as_object_mut)
    {
        // Written by the same build as its run, so it omits the same fields.
        backfill_later_params(retune)
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
    }
    if value["kind"] == "header"
        && let Some(params) = value
            .get_mut("data")
            .and_then(|data| data.get_mut("params"))
            .and_then(serde_json::Value::as_object_mut)
    {
        backfill_later_params(params)
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
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

/// Fills the params fields a file written before them omits with the values its run
/// actually had, which today's serde defaults would not. Refuses a run whose founders
/// bite but which records no `combat`, as the history readers do: today's defaults
/// could describe attack rules the run never had.
fn backfill_later_params(
    params: &mut serde_json::Map<String, serde_json::Value>,
) -> std::result::Result<(), &'static str> {
    let biting = params
        .get("founder")
        .and_then(|founder| founder.get("bite"))
        .and_then(serde_json::Value::as_bool);
    if biting == Some(true) && !params.contains_key("combat") {
        return Err("a run whose founders bite must record its combat parameters");
    }
    // Mutation rates shipped at zero until Phase 2 M8, oscillator addition did
    // not exist before it, and M9's plant ecology fields came later still; a
    // schema-8 file written earlier omits some of them. It ran without them, so
    // today's defaults must not rewrite it.
    for (path, fields) in [
        (
            &["mutation", "structural"][..],
            &[
                "remove_connection_rate",
                "remove_neuron_rate",
                "toggle_connection_rate",
                "add_connection_rate",
                "add_neuron_rate",
                "add_oscillator_rate",
            ][..],
        ),
        (
            &["mutation", "organs"][..],
            &["remove_sensor_rate", "add_sensor_rate"][..],
        ),
        (
            &["plants"][..],
            &[
                "grazing_lag",
                "patchiness",
                "patch_scale",
                "death_stock",
                "death_seconds",
                "local_dispersal",
                "dispersal_radius",
            ][..],
        ),
        // Pre-Phase-3 runs left no corpses, and had no pool for its ceiling to pay for.
        (&["corpses"][..], &["energy_fraction", "max_corpses"][..]),
        // Nor did their bodies pay upkeep for muscle or mouth, or evolve.
        (&["metabolism"][..], &["k_muscle", "k_mouth"][..]),
        (
            &["mutation"][..],
            &["body_trait_rate", "body_trait_sigma"][..],
        ),
    ] {
        let mut object = Some(&mut *params);
        for key in path {
            object = object.and_then(|parent| {
                parent
                    .entry(*key)
                    .or_insert_with(|| serde_json::json!({}))
                    .as_object_mut()
            });
        }
        if let Some(object) = object {
            for field in fields {
                // An integer zero, which reads as either a rate or a count.
                object.entry(*field).or_insert_with(|| serde_json::json!(0));
            }
        }
    }
    // Bodies did not evolve before Phase 3 either, so a one-point range at the
    // founders' traits is exactly such a run, whatever body.size it used.
    if let Some(body) = params
        .entry("body")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    {
        let size = body
            .get("size")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(sim_core::params::BodyParams::default().size));
        body.entry("size_range")
            .or_insert_with(|| serde_json::json!([size.clone(), size]));
        for field in ["muscle_range", "mouth_range"] {
            body.entry(field)
                .or_insert_with(|| serde_json::json!([1.0, 1.0]));
        }
    }
    // Nor could their founders bite, whatever a later default says. Without a bite no
    // agent swings, so an absent `combat` is inert and keeps today's values.
    if let Some(founder) = params
        .entry("founder")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    {
        founder
            .entry("bite")
            .or_insert_with(|| serde_json::json!(false));
    }
    Ok(())
}

/// A recorded retune must be one the run could have applied: within the run, and a
/// legal live retune of the world its params build.
fn validate_retune(header: &RunHeader) -> Result<()> {
    let Some(retune) = &header.retune else {
        return Ok(());
    };
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    if retune.at_tick > header.ticks {
        return Err(invalid("metrics retune comes after the run's last tick".to_owned()).into());
    }
    let params = &header.params;
    // The grid a world built from these params would have, which bounds a retune.
    let cell = sim_core::spatial::SpatialHash::cell_size_for(
        params.world.size,
        params.sensing.max_sense_radius(),
    );
    params.check_retune(&retune.params, cell).map_err(|error| {
        invalid(format!(
            "metrics retune is not a legal live retune: {error}"
        ))
    })?;
    Ok(())
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
            wiring: None,
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
                retune: None,
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
    fn reads_the_structural_null_control_and_refuses_the_retired_one() {
        let mut records = final_records();
        records[0]["data"]["control"] = STRUCTURAL_NULL_PROTOCOL.into();
        assert_eq!(
            parse_values(&records).unwrap().header.control,
            STRUCTURAL_NULL_PROTOCOL
        );
        records[0]["data"]["control"] = "structural_null_v1".into();
        assert!(parse_values(&records).is_err());
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
            retune: None,
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
            retune: None,
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
                retune: None,
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
                retune: None,
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
        let mut records = final_records();
        records[0]["data"]["control"] = STRUCTURAL_NULL_PROTOCOL.into();
        parse_values(&records).unwrap();
    }

    #[test]
    fn a_run_from_before_corpses_is_not_charged_for_a_corpse_pool() {
        // The tightest ceiling a pre-Phase-3 run could have recorded and still built.
        let mut records = final_records();
        let mut params: SimParams =
            serde_json::from_value(records[0]["data"]["params"].clone()).unwrap();
        params.corpses.max_corpses = 0;
        let (mut low, mut high) = (0, params.storage.max_memory_bytes);
        while low < high {
            let middle = low + (high - low) / 2;
            params.storage.max_memory_bytes = middle;
            if params.validate().is_ok() {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        records[0]["data"]["params"]["storage"]["max_memory_bytes"] = serde_json::json!(high);
        records[0]["data"]["params"]
            .as_object_mut()
            .unwrap()
            .remove("corpses");
        let data = parse_values(&records).unwrap();
        assert_eq!(data.header.params.corpses.max_corpses, 0);
        assert_eq!(data.header.params.storage.max_memory_bytes, high);
    }

    #[test]
    fn a_recorded_retune_is_read_as_its_run_was() {
        // Written by the same build as its run, a retune omits the same later fields.
        // Read with today's defaults, it would appear to change frozen ones.
        let mut records = final_records();
        let mut knockout = SimParams::default();
        knockout.plants.scent_rate = 0.0;
        records[0]["data"]["retune"] = serde_json::json!({
            "at_tick": 0,
            "params": serde_json::to_value(&knockout).unwrap(),
        });
        for path in ["/data/params", "/data/retune/params"] {
            let params = records[0].pointer_mut(path).unwrap();
            params.as_object_mut().unwrap().remove("corpses");
            params["body"].as_object_mut().unwrap().remove("size_range");
        }
        let data = parse_values(&records).unwrap();
        let retune = data.header.retune.unwrap().params;
        assert_eq!(retune.corpses.max_corpses, 0);
        assert_eq!(retune.body.size_range, data.header.params.body.size_range);
        assert_eq!(retune.plants.scent_rate, 0.0);
    }

    #[test]
    fn founders_written_before_the_bite_read_as_biteless() {
        // Explicitly, so a later default cannot give an older run a bite it never had,
        // while a value the file wrote stays its own.
        let mut params = serde_json::Map::new();
        backfill_later_params(&mut params).unwrap();
        assert_eq!(params["founder"]["bite"], false);
        let mut written = serde_json::json!({"founder": {"bite": true}, "combat": {}});
        backfill_later_params(written.as_object_mut().unwrap()).unwrap();
        assert_eq!(written["founder"]["bite"], true);
        // Biting with no record of how is refused, not filled from today's defaults.
        let mut unrecorded = serde_json::json!({"founder": {"bite": true}});
        assert!(backfill_later_params(unrecorded.as_object_mut().unwrap()).is_err());
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
        for section in ["corpses", "combat", "founder"] {
            records[0]["data"]["params"]
                .as_object_mut()
                .unwrap()
                .remove(section);
        }
        records[0]["data"]["params"]["plants"]
            .as_object_mut()
            .unwrap()
            .remove("grazing_lag");
        for field in ["k_muscle", "k_mouth"] {
            records[0]["data"]["params"]["metabolism"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        for field in ["body_trait_rate", "body_trait_sigma"] {
            records[0]["data"]["params"]["mutation"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        for field in ["size_range", "muscle_range", "mouth_range"] {
            records[0]["data"]["params"]["body"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        for field in [
            "patchiness",
            "patch_scale",
            "death_stock",
            "death_seconds",
            "local_dispersal",
            "dispersal_radius",
        ] {
            records[0]["data"]["params"]["plants"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
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
        assert_eq!(data.header.params.plants.grazing_lag, 0.0);
        assert_eq!(data.header.params.plants.patchiness, 0.0);
        assert_eq!(data.header.params.plants.patch_scale, 0.0);
        assert_eq!(data.header.params.plants.death_seconds, 0.0);
        assert_eq!(data.header.params.plants.local_dispersal, 0.0);
        assert_eq!(data.header.params.plants.dispersal_radius, 0.0);
        assert_eq!(data.header.params.corpses.energy_fraction, 0.0);
        assert_eq!(data.header.params.corpses.max_corpses, 0);
        assert_eq!(data.header.params.metabolism.k_muscle, 0.0);
        assert_eq!(data.header.params.metabolism.k_mouth, 0.0);
        assert_eq!(data.header.params.mutation.body_trait_rate, 0.0);
        assert_eq!(data.header.params.mutation.body_trait_sigma, 0.0);
        let size = data.header.params.body.size;
        assert_eq!(data.header.params.body.size_range, [size, size]);
        assert_eq!(data.header.params.body.muscle_range, [1.0, 1.0]);
        assert_eq!(data.header.params.body.mouth_range, [1.0, 1.0]);
        assert!(!data.header.params.founder.bite);
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
