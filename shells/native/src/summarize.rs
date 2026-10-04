//! Multi-seed summaries of completed metrics files, for M8's experiment reports.
//!
//! Groups runs by configuration, pairs each seed's evolving world with the controls
//! run beside it, and reports every metric's spread across seeds per cohort. It never
//! ranks configurations or combines metrics into a score (spec section 7.9); deciding
//! what is interesting stays with the human reading the vectors.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::Serialize;
use sim_core::control::{RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL};

use crate::Result;
use crate::cli::SummarizeArgs;
use crate::diagnose::control_label;
use crate::metrics::{RunHeader, RunSample, WorldMetrics};
use crate::metrics_reader::read_metrics;

/// Final-sample metrics reported per cohort, in output order.
const METRICS: &[&str] = &[
    "extinct",
    "population",
    "descendants",
    "genome_variants",
    "agent_energy_mean",
    "speed_mean",
    "supply_captured",
    "plant_stock",
    "plant_clustering",
    "plants_reseeded",
    "brain_units_mean",
    "sensor_load_mean",
    "genome_genes_mean",
    "genome_genes_max",
    "neurons_mean",
    "neurons_max",
    "connections_mean",
    "enabled_connections_mean",
    "wired_hidden_neurons_mean",
    "wired_sensors_mean",
    "driven_effectors_mean",
    "active_species",
    "persistent_species",
    "longest_species_span",
    "species_created",
    "species_extinct",
    "unclassified_population",
    "structural_edits_applied",
    "sensor_edits_applied",
    "capacity_refusals",
    "pool_full_refusals",
    "peak_arena_use",
];

#[derive(Debug, Serialize)]
pub struct Summary {
    pub configurations: Vec<Configuration>,
    /// Definitions a reader needs to interpret the metric names.
    pub notes: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Configuration {
    /// FNV-1a of the canonical params JSON; equal digests mean identical params.
    pub params_digest: String,
    pub founders: u32,
    pub ticks: u64,
    pub sample_every: u64,
    pub source_revisions: Vec<String>,
    pub seeds: Vec<String>,
    pub cohorts: Vec<Cohort>,
    /// Seeds missing one of the controls run for other seeds in this configuration.
    pub unpaired: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Cohort {
    pub name: &'static str,
    pub protocol: Option<&'static str>,
    pub seeds: Vec<String>,
    pub metrics: Vec<MetricStats>,
}

#[derive(Debug, Serialize)]
pub struct MetricStats {
    pub name: &'static str,
    /// One value per seed, in `seeds` order; `None` where the run did not record it.
    pub values: Vec<Option<f64>>,
    pub n: usize,
    pub mean: Option<f64>,
    /// Sample standard deviation; absent with fewer than two values.
    pub sd: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

const NOTES: &[&str] = &[
    "values are each run's final sample; spread is across seeds, not time",
    "cohorts are reported side by side and are not ranked or scored",
    "persistent_species: species alive at the end that were first sampled at least half the run earlier",
    "longest_species_span: longest first-to-last sampled presence of any species, in ticks",
    "wired_*/driven_effectors: structure on an enabled path from a sensor or oscillator to an effector, per agent",
    "supply_captured: energy eaten over the second half as a fraction of the plants' nominal input (plants refuse input once full)",
    "plant_stock: plant energy as a fraction of every plant full; plant_clustering: Clark-Evans ratio, near 1 random and below 1 clustered",
    "plants_reseeded: plants that died of starvation and reseeded over the run",
    "capacity_refusals: births refused for genome or arena limits, a warning that growth met allocator bounds",
    "peak_arena_use: the fullest arena's used fraction at the final sample",
];

struct Run {
    header: RunHeader,
    samples: Vec<RunSample>,
}

#[derive(Default)]
struct Group {
    founders: u32,
    ticks: u64,
    sample_every: u64,
    revisions: Vec<String>,
    seeds: BTreeMap<u64, SeedRuns>,
}

/// Phase 2 control protocols a summary can pair, in output order.
const CONTROLS: [&str; 2] = [RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL];

#[derive(Default)]
struct SeedRuns {
    /// Evolving values and the final state hash that pairs runs of one seed.
    evolving: Option<(Vec<Option<f64>>, Option<String>)>,
    /// One slot per entry of [`CONTROLS`].
    controls: [Option<Vec<Option<f64>>>; CONTROLS.len()],
}

pub fn run(args: SummarizeArgs) -> Result<()> {
    let summary = summarize(&args.metrics)?;
    if args.json {
        serde_json::to_writer_pretty(io::stdout(), &summary)?;
        println!();
    } else {
        write_human(&mut io::stdout().lock(), &summary)?;
    }
    Ok(())
}

fn invalid(message: String) -> Box<dyn std::error::Error> {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}

pub fn summarize(paths: &[PathBuf]) -> Result<Summary> {
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for path in paths {
        let data =
            read_metrics(path).map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let run = Run {
            header: data.header,
            samples: data.samples,
        };
        add(&mut groups, run).map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    }
    let configurations = groups
        .into_iter()
        .map(|(params, group)| configuration(&params, group))
        .collect();
    Ok(Summary {
        configurations,
        notes: NOTES.to_vec(),
    })
}

fn add(groups: &mut BTreeMap<String, Group>, run: Run) -> Result<()> {
    let header = &run.header;
    let Some(last) = run.samples.last() else {
        return Err(invalid("no samples".to_owned()));
    };
    if last.tick != header.ticks {
        return Err(invalid(format!(
            "final sample is tick {}, not the run's {} ticks",
            last.tick, header.ticks
        )));
    }
    let seed: u64 = header
        .seed
        .parse()
        .map_err(|_| invalid("seed is not a decimal u64".to_owned()))?;
    let params = serde_json::to_string(&header.params)?;
    let group = groups.entry(params).or_insert_with(|| Group {
        founders: header.founders,
        ticks: header.ticks,
        sample_every: header.sample_every,
        ..Group::default()
    });
    if (group.founders, group.ticks, group.sample_every)
        != (header.founders, header.ticks, header.sample_every)
    {
        return Err(invalid(
            "same params but different founders, ticks, or sample interval; summarize them separately"
                .to_owned(),
        ));
    }
    if !group.revisions.contains(&header.source_revision) {
        group.revisions.push(header.source_revision.clone());
    }
    let runs = group.seeds.entry(seed).or_default();
    let hash = last
        .final_state_hashes
        .as_ref()
        .map(|hashes| hashes.evolving.clone());
    let evolving = cohort_values(&run.samples, header.ticks, &header.params, |sample| {
        &sample.evolving
    });
    // Every control run repeats the same evolving world; disagreement means the runs
    // are not a matched comparison (different source or params).
    match &runs.evolving {
        Some((_, previous)) if previous.is_some() && hash.is_some() && *previous != hash => {
            return Err(invalid(format!(
                "seed {seed}: evolving world differs from another run of this configuration"
            )));
        }
        Some(_) => {}
        None => runs.evolving = Some((evolving, hash)),
    }
    let control = cohort_values(&run.samples, header.ticks, &header.params, |sample| {
        &sample.random_control
    });
    let Some(index) = CONTROLS
        .iter()
        .position(|&protocol| protocol == header.control)
    else {
        return Err(invalid(format!(
            "control protocol {} predates this phase's comparisons",
            header.control
        )));
    };
    if runs.controls[index].replace(control).is_some() {
        return Err(invalid(format!(
            "seed {seed} has two runs with control {}",
            header.control
        )));
    }
    Ok(())
}

fn configuration(params: &str, group: Group) -> Configuration {
    let seeds: Vec<String> = group.seeds.keys().map(u64::to_string).collect();
    let present: Vec<bool> = (0..CONTROLS.len())
        .map(|index| {
            group
                .seeds
                .values()
                .any(|runs| runs.controls[index].is_some())
        })
        .collect();
    let unpaired = group
        .seeds
        .iter()
        .filter(|(_, runs)| {
            present
                .iter()
                .zip(&runs.controls)
                .any(|(&run_somewhere, run)| run_somewhere && run.is_none())
        })
        .map(|(seed, _)| seed.to_string())
        .collect();
    let mut cohorts = vec![cohort("evolving", None, &group, |runs| {
        runs.evolving.as_ref().map(|(values, _)| values)
    })];
    for (index, protocol) in CONTROLS.into_iter().enumerate() {
        if present[index] {
            cohorts.push(cohort(
                control_label(protocol),
                Some(protocol),
                &group,
                |runs| runs.controls[index].as_ref(),
            ));
        }
    }
    Configuration {
        params_digest: format!("{:016x}", fnv1a(params.as_bytes())),
        founders: group.founders,
        ticks: group.ticks,
        sample_every: group.sample_every,
        source_revisions: group.revisions,
        seeds,
        cohorts,
        unpaired,
    }
}

fn cohort(
    name: &'static str,
    protocol: Option<&'static str>,
    group: &Group,
    values: impl Fn(&SeedRuns) -> Option<&Vec<Option<f64>>>,
) -> Cohort {
    let present: Vec<(u64, &Vec<Option<f64>>)> = group
        .seeds
        .iter()
        .filter_map(|(seed, runs)| values(runs).map(|values| (*seed, values)))
        .collect();
    let metrics = METRICS
        .iter()
        .enumerate()
        .map(|(index, &name)| {
            stats(
                name,
                present.iter().map(|(_, values)| values[index]).collect(),
            )
        })
        .collect();
    Cohort {
        name,
        protocol,
        seeds: present.iter().map(|(seed, _)| seed.to_string()).collect(),
        metrics,
    }
}

fn stats(name: &'static str, values: Vec<Option<f64>>) -> MetricStats {
    let known: Vec<f64> = values.iter().flatten().copied().collect();
    let n = known.len();
    let mean = (n > 0).then(|| known.iter().sum::<f64>() / n as f64);
    let sd = mean.filter(|_| n > 1).map(|mean| {
        (known
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (n - 1) as f64)
            .sqrt()
    });
    MetricStats {
        name,
        values,
        n,
        mean,
        sd,
        min: known.iter().copied().reduce(f64::min),
        max: known.iter().copied().reduce(f64::max),
    }
}

/// One cohort's metric vector in `METRICS` order.
fn cohort_values(
    samples: &[RunSample],
    ticks: u64,
    params: &sim_core::SimParams,
    select: impl Fn(&RunSample) -> &WorldMetrics,
) -> Vec<Option<f64>> {
    let plants = &params.plants;
    let supply = f64::from(plants.energy_input_rate * params.world.dt);
    let capacity = f64::from(plants.max_energy) * f64::from(plants.max_plants);
    let last = select(samples.last().expect("checked non-empty"));
    let complexity = last.complexity.as_ref();
    let wiring = complexity.and_then(|c| c.wiring.as_ref());
    let species = last.species.as_ref();
    let edits = last.structural_mutations;
    let (persistent, longest) = species_persistence(samples, ticks, &select);
    let captured = supply_captured(samples, ticks, supply, &select);
    let values: Vec<Option<f64>> = vec![
        Some(f64::from(u8::from(last.population == 0))),
        Some(f64::from(last.population)),
        Some(f64::from(last.descendants)),
        Some(f64::from(last.genome_variants)),
        Some(last.agent_energy.mean),
        Some(last.speed.mean),
        captured,
        (capacity > 0.0).then(|| last.plant_energy / capacity),
        last.plants.and_then(|p| p.clustering),
        last.plants.map(|p| p.reseeded as f64),
        Some(last.brain_units.mean),
        Some(last.sensor_load.mean),
        Some(last.genome_genes.mean),
        Some(last.genome_genes.max),
        complexity.map(|c| c.neurons.mean),
        complexity.map(|c| f64::from(c.neurons.max)),
        complexity.map(|c| c.connections.mean),
        complexity.map(|c| c.enabled_connections.mean),
        wiring.map(|w| w.wired_hidden_neurons.mean),
        wiring.map(|w| w.wired_sensors.mean),
        wiring.map(|w| w.driven_effectors.mean),
        species.map(|s| s.populations.len() as f64),
        persistent,
        longest,
        species.and_then(|s| s.events).map(|e| e.created as f64),
        species.and_then(|s| s.events).map(|e| e.extinct as f64),
        species.map(|s| f64::from(s.unclassified_population)),
        edits.map(|e| {
            [
                e.remove_connection,
                e.remove_neuron,
                e.toggle_connection,
                e.add_connection,
                e.add_neuron,
            ]
            .iter()
            .chain(e.add_oscillator.as_ref())
            .map(|counts| counts.applied as f64)
            .sum()
        }),
        edits.and_then(|e| Some((e.remove_sensor?.applied + e.add_sensor?.applied) as f64)),
        last.spawn_failures.map(|f| {
            (f.genome_limit + f.arena_capacity + f.arena_fragmentation + f.arena_block_limit) as f64
        }),
        last.spawn_failures.map(|f| f.pool_full as f64),
        last.arena_usage
            .iter()
            .filter(|usage| usage.capacity > 0)
            .map(|usage| {
                f64::from(usage.capacity - usage.free_elements) / f64::from(usage.capacity)
            })
            .reduce(f64::max),
    ];
    debug_assert_eq!(values.len(), METRICS.len());
    values
}

/// Energy eaten over the run's second half as a fraction of the plants' nominal input
/// over the same ticks.
///
/// Not per agent: once a run settles, everything that enters is eaten, so intake per
/// agent is only input divided by population. What foraging changes is how much enters
/// at all, because a full plant refuses its share of the input (`Plants::grow`); a
/// population that keeps more sites grazed below capacity captures more of the supply.
/// Agents gain energy only by eating, so between samples intake is the change in
/// agent-held energy plus what was dissipated. Assumes no founders were added after
/// the first sample, which `experiment.sh` runs never do.
fn supply_captured(
    samples: &[RunSample],
    ticks: u64,
    supply: f64,
    select: &impl Fn(&RunSample) -> &WorldMetrics,
) -> Option<f64> {
    let first = samples.iter().find(|s| s.tick >= ticks / 2)?;
    let last = samples.last()?;
    let offered = supply * (last.tick - first.tick) as f64;
    if offered <= 0.0 {
        return None;
    }
    let held = |m: &WorldMetrics| m.total_energy - m.plant_energy;
    let (a, b) = (select(first), select(last));
    Some((held(b) - held(a) + (b.cumulative_dissipation - a.cumulative_dissipation)) / offered)
}

/// Species persistence from sampled presence: how many species alive at the end
/// were first sampled at least half the run earlier, and the longest sampled span.
fn species_persistence(
    samples: &[RunSample],
    ticks: u64,
    select: &impl Fn(&RunSample) -> &WorldMetrics,
) -> (Option<f64>, Option<f64>) {
    let mut first_last: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    for sample in samples {
        let Some(species) = select(sample).species.as_ref() else {
            return (None, None);
        };
        for entry in &species.populations {
            let span = first_last
                .entry(entry.species_id.raw())
                .or_insert((sample.tick, sample.tick));
            span.1 = sample.tick;
        }
    }
    let end = samples.last().map_or(0, |sample| sample.tick);
    let persistent = first_last
        .values()
        .filter(|&&(first, last)| last == end && end - first >= ticks / 2)
        .count();
    let longest = first_last
        .values()
        .map(|&(first, last)| last - first)
        .max()
        .unwrap_or(0);
    (Some(persistent as f64), Some(longest as f64))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn format_value(value: Option<f64>) -> String {
    match value {
        None => "—".to_owned(),
        Some(value) if value.fract() == 0.0 && value.abs() < 1e12 => format!("{value:.0}"),
        Some(value) => format!("{value:.3}"),
    }
}

fn cell(metric: &MetricStats) -> String {
    if metric.n == 0 {
        return "unavailable".to_owned();
    }
    format!(
        "{} ± {} [{}–{}]",
        format_value(metric.mean),
        format_value(metric.sd),
        format_value(metric.min),
        format_value(metric.max)
    )
}

pub(crate) fn write_human(output: &mut impl Write, summary: &Summary) -> io::Result<()> {
    for configuration in &summary.configurations {
        writeln!(
            output,
            "configuration params={} founders={} ticks={} sample_every={}",
            configuration.params_digest,
            configuration.founders,
            configuration.ticks,
            configuration.sample_every
        )?;
        writeln!(output, "  seeds: {}", configuration.seeds.join(", "))?;
        writeln!(
            output,
            "  source revisions: {}",
            configuration.source_revisions.join(", ")
        )?;
        if !configuration.unpaired.is_empty() {
            writeln!(
                output,
                "  unpaired seeds (a control is missing): {}",
                configuration.unpaired.join(", ")
            )?;
        }
        // Columns are sized to their widest entry, so no header or value overruns.
        // Format width counts chars, which keeps `±` and `–` aligned.
        const METRIC_HEADER: &str = "metric (mean ± sd [min–max])";
        let titles: Vec<String> = configuration
            .cohorts
            .iter()
            .map(|cohort| format!("{} (n={})", cohort.name, cohort.seeds.len()))
            .collect();
        let cells: Vec<Vec<String>> = configuration
            .cohorts
            .iter()
            .map(|cohort| cohort.metrics.iter().map(cell).collect())
            .collect();
        let first = METRICS
            .iter()
            .map(|name| name.chars().count())
            .chain([METRIC_HEADER.chars().count()])
            .max()
            .unwrap_or(0);
        let widths: Vec<usize> = titles
            .iter()
            .zip(&cells)
            .map(|(title, column)| {
                column
                    .iter()
                    .chain([title])
                    .map(|text| text.chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        write!(output, "  {METRIC_HEADER:<first$}")?;
        for (title, width) in titles.iter().zip(&widths) {
            write!(output, " | {title:<width$}")?;
        }
        writeln!(output)?;
        for (index, name) in METRICS.iter().enumerate() {
            write!(output, "  {name:<first$}")?;
            for (column, width) in cells.iter().zip(&widths) {
                write!(output, " | {:<width$}", column[index])?;
            }
            writeln!(output)?;
        }
        writeln!(output)?;
    }
    for note in &summary.notes {
        writeln!(output, "note: {note}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{SpeciesMetrics, SpeciesPopulation};
    use sim_core::ids::SpeciesId;

    fn sample(tick: u64, species: &[u32]) -> RunSample {
        let evolving = WorldMetrics {
            population: 10,
            species: Some(SpeciesMetrics {
                populations: species
                    .iter()
                    .map(|&id| SpeciesPopulation {
                        species_id: SpeciesId::new(id),
                        population: 5,
                    })
                    .collect(),
                ..SpeciesMetrics::default()
            }),
            ..WorldMetrics::default()
        };
        RunSample {
            tick,
            random_control: evolving.clone(),
            evolving,
            final_state_hashes: None,
        }
    }

    #[test]
    fn spread_uses_the_sample_standard_deviation_and_skips_missing_values() {
        let metric = stats("x", vec![Some(2.0), None, Some(4.0), Some(6.0)]);
        assert_eq!(metric.n, 3);
        assert_eq!(metric.mean, Some(4.0));
        assert_eq!(metric.sd, Some(2.0));
        assert_eq!((metric.min, metric.max), (Some(2.0), Some(6.0)));
        let single = stats("x", vec![Some(1.0)]);
        assert_eq!((single.mean, single.sd), (Some(1.0), None));
        let none = stats("x", vec![None]);
        assert_eq!((none.n, none.mean, none.min), (0, None, None));
    }

    #[test]
    fn persistence_counts_only_long_lived_species_alive_at_the_end() {
        // Species 1 spans the whole run; 2 appears late; 3 died out after a long span.
        let samples = vec![
            sample(0, &[1, 3]),
            sample(500, &[1, 3]),
            sample(800, &[1, 2, 3]),
            sample(1_000, &[1, 2]),
        ];
        let (persistent, longest) = species_persistence(&samples, 1_000, &|s| &s.evolving);
        assert_eq!(persistent, Some(1.0));
        assert_eq!(longest, Some(1_000.0));
        let mut legacy = samples.clone();
        legacy[1].evolving.species = None;
        assert_eq!(
            species_persistence(&legacy, 1_000, &|s| &s.evolving),
            (None, None)
        );
    }

    #[test]
    fn table_columns_fit_their_widest_entry() {
        let runs = vec![sample(0, &[1]), sample(1_000, &[1])];
        let mut groups = BTreeMap::new();
        let header = RunHeader {
            schema_version: crate::metrics::SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test".to_owned(),
            phase: 2,
            seed: "7".to_owned(),
            ticks: 1_000,
            founders: 1,
            sample_every: 1_000,
            params: sim_core::SimParams::default(),
            control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
        };
        add(
            &mut groups,
            Run {
                header,
                samples: runs,
            },
        )
        .unwrap();
        let summary = Summary {
            configurations: groups
                .into_iter()
                .map(|(params, group)| configuration(&params, group))
                .collect(),
            notes: Vec::new(),
        };
        let mut output = Vec::new();
        write_human(&mut output, &summary).unwrap();
        let text = String::from_utf8(output).unwrap();
        let rows: Vec<&str> = text.lines().filter(|line| line.contains(" | ")).collect();
        assert_eq!(rows.len(), METRICS.len() + 1);
        // Every separator sits in the same character column on every row.
        let separators = |line: &str| -> Vec<usize> {
            let chars: Vec<char> = line.chars().collect();
            (0..chars.len().saturating_sub(2))
                .filter(|&i| chars[i..i + 3] == [' ', '|', ' '])
                .collect()
        };
        let expected = separators(rows[0]);
        assert_eq!(expected.len(), 2, "{}", rows[0]);
        for row in &rows {
            assert_eq!(separators(row), expected, "{row}");
        }
    }

    #[test]
    fn supply_captured_is_energy_eaten_over_the_second_half_per_unit_offered() {
        let at = |tick, held: f64, dissipated: f64| {
            let mut sample = sample(tick, &[1]);
            sample.evolving.plant_energy = 1_000.0;
            sample.evolving.total_energy = 1_000.0 + held;
            sample.evolving.cumulative_dissipation = dissipated;
            sample
        };
        // The first half's feeding is excluded; the second half eats 300 (+100 held,
        // +200 dissipated) of the 5 x 100 offered.
        let samples = [at(0, 0.0, 0.0), at(100, 500.0, 0.0), at(200, 600.0, 200.0)];
        let captured = supply_captured(&samples, 200, 5.0, &|s| &s.evolving);
        assert_eq!(captured, Some(0.6));
        assert_eq!(
            supply_captured(&samples[..1], 200, 5.0, &|s| &s.evolving),
            None
        );
    }

    #[test]
    fn every_metric_has_a_value_slot() {
        let values = cohort_values(
            &[sample(0, &[1])],
            0,
            &sim_core::SimParams::default(),
            |s| &s.evolving,
        );
        assert_eq!(values.len(), METRICS.len());
        assert_eq!(values[0], Some(0.0), "a living population is not extinct");
    }
}
