//! Maps metrics-series signatures to the known failure modes in spec §10.
//!
//! Diagnostics are evidence, not a score: every detected signal is reported alongside
//! its likely causes, and systems not present in the current phase are explicitly
//! unavailable rather than inferred from unrelated numbers.

use std::io;

use serde::Serialize;

use crate::Result;
use crate::cli::DiagnoseArgs;
use crate::diagnose_output::print_human;
use crate::metrics::{RunHeader, RunSample, WorldMetrics};
use crate::metrics_reader::read_metrics;

const IDLE_SPEED_FRACTION: f64 = 0.001;
const ENERGY_NOISE_FRACTION: f64 = 0.001;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub signal: String,
    pub likely_causes: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MetricComparison {
    pub evolving: f64,
    pub random_control: f64,
    pub relative_gap: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControlMetrics {
    pub population: MetricComparison,
    pub mean_agent_energy: MetricComparison,
    pub mean_speed: MetricComparison,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComparisonReport {
    pub tail_means: Option<ControlMetrics>,
    pub findings: Vec<Finding>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiagnosisReport {
    pub samples: usize,
    pub evolving: Vec<Finding>,
    pub random_control: Vec<Finding>,
    pub comparison: ComparisonReport,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Copy)]
enum Cohort {
    Evolving,
    Control,
}

impl Cohort {
    fn name(self) -> &'static str {
        match self {
            Self::Evolving => "evolving",
            Self::Control => "scalar control",
        }
    }
}

pub fn run(args: DiagnoseArgs) -> Result<()> {
    let data = read_metrics(&args.metrics)?;
    let report = diagnose(&data.header, &data.samples);
    if args.json {
        serde_json::to_writer_pretty(io::stdout(), &report)?;
        println!();
    } else {
        print_human(&report)?;
    }
    Ok(())
}

pub fn diagnose(header: &RunHeader, samples: &[RunSample]) -> DiagnosisReport {
    let comparison = compare_control(header, samples);
    let (evolving, evolving_idle_unavailable) = diagnose_cohort(header, samples, Cohort::Evolving);
    let (random_control, control_idle_unavailable) =
        diagnose_cohort(header, samples, Cohort::Control);
    let mut unavailable = vec![
        "species-cluster diagnostics are unavailable until M4 clustering; monoculture uses exact genome variants, not species".to_owned(),
        "predator/prey diagnostics require Phase 3 trophic roles".to_owned(),
        "signal-correlation diagnostics require Phase 4 signaling".to_owned(),
    ];
    if let Some(reason) = evolving_idle_unavailable {
        unavailable.push(reason);
    }
    if let Some(reason) = control_idle_unavailable {
        unavailable.push(reason);
    }
    for cohort in [Cohort::Evolving, Cohort::Control] {
        if samples.is_empty()
            || samples
                .iter()
                .any(|sample| select(sample, cohort).spawn_failures.is_none())
        {
            unavailable.push(format!(
                "complete spawn-refusal counts for {} are unavailable: stepping was not observed for every sample",
                cohort.name()
            ));
        }
        if samples.is_empty()
            || samples
                .iter()
                .any(|sample| select(sample, cohort).structural_mutations.is_none())
        {
            unavailable.push(format!(
                "complete structural-mutation counts for {} are unavailable: schema 3 or unobserved sampling does not establish zero edits",
                cohort.name()
            ));
        }
    }
    DiagnosisReport {
        samples: samples.len(),
        evolving,
        random_control,
        comparison,
        unavailable,
    }
}

fn diagnose_cohort(
    header: &RunHeader,
    samples: &[RunSample],
    cohort: Cohort,
) -> (Vec<Finding>, Option<String>) {
    let mut findings = Vec::new();
    let metrics: Vec<(u64, &WorldMetrics)> = samples
        .iter()
        .map(|sample| (sample.tick, select(sample, cohort)))
        .collect();

    if let Some((tick, sample)) = metrics
        .iter()
        .rev()
        .find(|(_, sample)| sample.spawn_failures.is_some())
    {
        diagnose_storage(*tick, sample, &mut findings);
    }
    if let Some((tick, sample)) = metrics
        .iter()
        .rev()
        .find(|(_, sample)| sample.structural_mutations.is_some())
    {
        diagnose_structural_mutations(*tick, sample, &mut findings);
    }

    if let Some((index, (tick, _))) = metrics
        .iter()
        .enumerate()
        .find(|(_, (_, sample))| sample.population == 0)
    {
        let previous_tick = index
            .checked_sub(1)
            .and_then(|previous| metrics.get(previous))
            .map_or(0, |(tick, _)| *tick);
        let early_boundary = header.ticks / 4;
        let (code, likely_causes) = if *tick <= early_boundary {
            (
                "early_extinction",
                vec![
                    "energy input too low for metabolic demand",
                    "mutation rate past error catastrophe",
                ],
            )
        } else if previous_tick >= early_boundary {
            (
                "extinction",
                vec!["population failed to replace itself over the observed run"],
            )
        } else {
            (
                "extinction_timing_uncertain",
                vec!["sample more frequently to distinguish early from late collapse"],
            )
        };
        findings.push(Finding {
            code,
            signal: format!(
                "extinction occurred in ({previous_tick}, {tick}] with sample interval {}",
                header.sample_every,
            ),
            likely_causes,
        });
    }

    if let Some((tick, sample, ratio)) = metrics
        .iter()
        .map(|(tick, sample)| {
            let scale = sample
                .total_energy
                .abs()
                .max(sample.cumulative_energy_input.abs())
                .max(sample.cumulative_dissipation.abs())
                .max(1.0);
            (*tick, *sample, sample.energy_drift.abs() / scale)
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        && ratio > 1e-3
    {
        findings.push(Finding {
            code: "energy_drift",
            signal: format!(
                "relative ledger drift reached {ratio:.6} at tick {tick} ({:.6} joules)",
                sample.energy_drift
            ),
            likely_causes: vec!["energy was created or destroyed outside the ledger"],
        });
    }

    // Older live samples do not describe the current state of an extinct run.
    if metrics
        .last()
        .is_some_and(|(_, sample)| sample.population == 0)
    {
        return (findings, None);
    }

    let live_tail: Vec<(u64, &WorldMetrics)> = metrics
        .iter()
        .rev()
        .filter_map(|(tick, sample)| (sample.population > 0).then_some((*tick, *sample)))
        .take(5)
        .collect();
    let had_diversity = metrics
        .iter()
        .any(|(_, sample)| sample.population > 1 && sample.genome_variants > 1);
    let monoculture_span = live_tail
        .first()
        .zip(live_tail.last())
        .map_or(0, |(newest, oldest)| newest.0.saturating_sub(oldest.0));
    if had_diversity
        && live_tail.len() >= 3
        && monoculture_span >= header.params.reproduction.maturity_ticks as u64
        && live_tail
            .iter()
            .all(|(_, sample)| sample.population > 1 && sample.genome_variants == 1)
    {
        findings.push(Finding {
            code: "monoculture",
            signal: format!(
                "ticks {}..={} contain one exact genome variant",
                live_tail.last().expect("three samples").0,
                live_tail.first().expect("three samples").0
            ),
            likely_causes: vec!["world too homogeneous; add spatial or temporal niches"],
        });
    }

    let live: Vec<&WorldMetrics> = metrics
        .iter()
        .filter_map(|(_, sample)| (sample.population > 0).then_some(*sample))
        .collect();
    if let (Some(first), Some(last)) = (live.first(), live.last())
        && last.brain_units.mean > first.brain_units.mean * 1.25
    {
        findings.push(Finding {
            code: "brain_bloat",
            signal: format!(
                "mean brain units rose from {:.1} to {:.1}",
                first.brain_units.mean, last.brain_units.mean
            ),
            likely_causes: vec!["brain complexity is underpriced by metabolism"],
        });
    }

    let requirements = match idle_requirements(header, &metrics) {
        Ok(requirements) => requirements,
        Err(reason) => {
            return (
                findings,
                Some(format!(
                    "stable-idle diagnosis for {} is unavailable: {reason}",
                    cohort.name()
                )),
            );
        }
    };
    let mut idle_tail = Vec::new();
    for &(tick, sample) in metrics.iter().rev() {
        if sample.population == 0 {
            break;
        }
        idle_tail.push((tick, sample));
        let span = idle_tail[0].0.saturating_sub(tick);
        if idle_tail.len() >= 3 && span >= requirements.observation_ticks {
            break;
        }
    }
    let idle_span = idle_tail
        .first()
        .zip(idle_tail.last())
        .map_or(0, |(newest, oldest)| newest.0.saturating_sub(oldest.0));
    if idle_tail.len() < 3 || idle_span < requirements.observation_ticks {
        return (
            findings,
            Some(format!(
                "stable-idle diagnosis for {} requires {} trailing live ticks across at least three samples; measured tail covers {idle_span} ticks across {} samples",
                cohort.name(),
                requirements.observation_ticks,
                idle_tail.len()
            )),
        );
    }

    {
        let min_population = idle_tail
            .iter()
            .map(|(_, sample)| sample.population)
            .min()
            .unwrap_or(0);
        let max_population = idle_tail
            .iter()
            .map(|(_, sample)| sample.population)
            .max()
            .unwrap_or(0);
        let mean_speed = idle_tail
            .iter()
            .map(|(_, sample)| sample.speed.mean)
            .sum::<f64>()
            / idle_tail.len() as f64;
        if max_population > 0
            && min_population as f64 >= max_population as f64 * 0.9
            && mean_speed < requirements.speed_threshold
        {
            findings.push(Finding {
                code: "stable_but_idle",
                signal: format!(
                    "population stayed within {min_population}..={max_population} for {idle_span} ticks while mean speed was {mean_speed:.4} (idle threshold {:.4})",
                    requirements.speed_threshold
                ),
                likely_causes: vec!["metabolism too cheap; idling is viable"],
            });
        }
    }

    (findings, None)
}

fn diagnose_storage(tick: u64, sample: &WorldMetrics, findings: &mut Vec<Finding>) {
    let Some(counts) = sample.spawn_failures else {
        return;
    };
    for (code, count, cause) in [
        (
            "agent_pool_full",
            counts.pool_full,
            "agent slots are exhausted; review max_agents and the explicit per-world memory budget",
        ),
        (
            "storage_capacity",
            counts.arena_capacity,
            "shared arenas lack total free elements; inspect arena_usage and review pooled storage allowances within the explicit memory budget",
        ),
        (
            "storage_fragmentation",
            counts.arena_fragmentation,
            "an arena has enough free elements but no sufficiently large contiguous span; compare free_elements with largest_free_block",
        ),
        (
            "storage_block_limit",
            counts.arena_block_limit,
            "the arena's simultaneous live-block limit was reached; free elements alone cannot satisfy another allocation",
        ),
        (
            "genome_limit",
            counts.genome_limit,
            "attempted genomes exceed configured per-genome limits; larger aggregate arena allowances alone will not admit them",
        ),
        (
            "invalid_genome",
            counts.invalid_genome,
            "attempted genomes failed validation; inspect the spawning input rather than tuning the energy economy",
        ),
    ] {
        if count > 0 {
            findings.push(Finding {
                code,
                signal: format!("{count} cumulative spawn refusals through tick {tick}"),
                likely_causes: vec![cause],
            });
        }
    }
}

fn diagnose_structural_mutations(tick: u64, sample: &WorldMetrics, findings: &mut Vec<Finding>) {
    let Some(counts) = sample.structural_mutations else {
        return;
    };
    for (operator, counts) in [
        ("remove_connection", counts.remove_connection),
        ("remove_neuron", counts.remove_neuron),
        ("toggle_connection", counts.toggle_connection),
        ("add_connection", counts.add_connection),
        ("add_neuron", counts.add_neuron),
    ] {
        for (code, count, cause) in [
            (
                "structural_genome_limit",
                counts.genome_limit,
                "a structural candidate edit exceeded a per-genome cap; aggregate arena capacity does not remove this limit",
            ),
            (
                "structural_scratch_limit",
                counts.scratch_limit,
                "a structural candidate edit exceeded preallocated mutation scratch capacity",
            ),
            (
                "structural_innovation_exhausted",
                counts.innovation_exhausted,
                "a structural candidate edit could not obtain fresh innovation IDs; energy and arena allowances cannot restore IDs",
            ),
        ] {
            if count > 0 {
                findings.push(Finding {
                    code,
                    signal: format!(
                        "{operator}: {count} cumulative refused candidate edits through tick {tick}; these are not spawn refusals"
                    ),
                    likely_causes: vec![cause],
                });
            }
        }
    }
}

struct IdleRequirements {
    observation_ticks: u64,
    speed_threshold: f64,
}

fn idle_requirements(
    header: &RunHeader,
    metrics: &[(u64, &WorldMetrics)],
) -> std::result::Result<IdleRequirements, &'static str> {
    let Some(first) = metrics
        .iter()
        .map(|(_, sample)| *sample)
        .find(|sample| sample.population > 0)
    else {
        return Err("there are no live samples");
    };
    let params = &header.params;
    let cost = params.metabolism.base as f64
        + params.metabolism.k_size as f64 * params.body.size as f64 * params.body.size as f64
        + params.metabolism.k_brain as f64 * first.brain_units.mean
        + params.metabolism.k_sensor as f64 * first.sensor_load.mean;
    if !cost.is_finite() || cost <= 0.0 {
        return Err("idle metabolic cost is not positive");
    }
    let speed_threshold = params.movement.max_speed as f64 * IDLE_SPEED_FRACTION;
    if !speed_threshold.is_finite() || speed_threshold <= 0.0 {
        return Err("movement.max_speed does not define a positive idle threshold");
    }
    let idle_lifetime = (params.reproduction.start_energy as f64 / cost).ceil();
    if !idle_lifetime.is_finite() || idle_lifetime > u64::MAX as f64 {
        return Err("idle lifetime exceeds the measurable tick range");
    }
    Ok(IdleRequirements {
        observation_ticks: (idle_lifetime as u64).max(params.reproduction.maturity_ticks as u64),
        speed_threshold,
    })
}

fn compare_control(header: &RunHeader, samples: &[RunSample]) -> ComparisonReport {
    let tail: Vec<_> = samples
        .iter()
        .rev()
        .take_while(|sample| {
            sample.evolving.descendants > 0
                && sample.random_control.descendants > 0
                && sample.evolving.population >= 10
                && sample.random_control.population >= 10
        })
        .take(5)
        .collect();
    if tail.len() < 3 {
        let reason = match samples.last() {
            Some(sample)
                if sample.evolving.descendants == 0 || sample.random_control.descendants == 0 =>
            {
                "scalar-control comparison requires living descendants in both cohorts"
            }
            Some(sample)
                if sample.evolving.population < 10 || sample.random_control.population < 10 =>
            {
                "scalar-control comparison requires at least 10 living agents in both cohorts"
            }
            _ => "scalar-control comparison requires at least three consecutive eligible samples",
        };
        return ComparisonReport {
            tail_means: None,
            findings: Vec::new(),
            unavailable: Some(reason.to_owned()),
        };
    }
    let mean = |value: fn(&WorldMetrics) -> f64, cohort: Cohort| {
        tail.iter()
            .map(|sample| value(select(sample, cohort)))
            .sum::<f64>()
            / tail.len() as f64
    };
    let population = compare_metric(
        mean(|m| m.population as f64, Cohort::Evolving),
        mean(|m| m.population as f64, Cohort::Control),
        1.0,
    );
    let mean_agent_energy = compare_metric(
        mean(|m| m.agent_energy.mean, Cohort::Evolving),
        mean(|m| m.agent_energy.mean, Cohort::Control),
        header.params.reproduction.start_energy as f64 * ENERGY_NOISE_FRACTION,
    );
    let mean_speed = compare_metric(
        mean(|m| m.speed.mean, Cohort::Evolving),
        mean(|m| m.speed.mean, Cohort::Control),
        header.params.movement.max_speed as f64 * IDLE_SPEED_FRACTION,
    );
    let tail_means = ControlMetrics {
        population,
        mean_agent_energy,
        mean_speed,
    };

    let findings = if tail_means.population.relative_gap < 0.1
        && tail_means.mean_agent_energy.relative_gap < 0.1
        && tail_means.mean_speed.relative_gap < 0.1
    {
        vec![Finding {
            code: "indistinguishable_from_random_control",
            signal: format!(
                "single-seed tail relative gaps: population={:.3}, energy={:.3}, speed={:.3}",
                tail_means.population.relative_gap,
                tail_means.mean_agent_energy.relative_gap,
                tail_means.mean_speed.relative_gap,
            ),
            likely_causes: vec![
                "observed behavior may not reflect cumulative neural-scalar inheritance",
                "repeat across seeds before drawing a conclusion",
            ],
        }]
    } else {
        Vec::new()
    };
    ComparisonReport {
        tail_means: Some(tail_means),
        findings,
        unavailable: None,
    }
}

fn compare_metric(evolving: f64, random_control: f64, noise_floor: f64) -> MetricComparison {
    let scale = evolving
        .abs()
        .max(random_control.abs())
        .max(noise_floor)
        .max(f64::EPSILON);
    MetricComparison {
        evolving,
        random_control,
        relative_gap: (evolving - random_control).abs() / scale,
    }
}

fn select(sample: &RunSample, cohort: Cohort) -> &WorldMetrics {
    match cohort {
        Cohort::Evolving => &sample.evolving,
        Cohort::Control => &sample.random_control,
    }
}

#[cfg(test)]
mod tests {
    use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;
    use sim_core::params::SimParams;

    use super::*;
    use crate::metrics::{SCHEMA_VERSION, Summary};

    fn header(ticks: u64) -> RunHeader {
        RunHeader {
            schema_version: SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test-revision".to_owned(),
            phase: 2,
            seed: "42".to_owned(),
            ticks,
            founders: 100,
            sample_every: 1_000,
            params: SimParams::default(),
            control: RANDOMIZED_AT_BIRTH_PROTOCOL.to_owned(),
        }
    }

    fn world(population: u32, genome_variants: u32, speed: f64) -> WorldMetrics {
        WorldMetrics {
            population,
            descendants: 0,
            genome_variants,
            agent_energy: Summary {
                mean: if population == 0 { 0.0 } else { 50.0 },
                max: if population == 0 { 0.0 } else { 60.0 },
            },
            plant_energy: 100.0,
            total_energy: 100.0 + population as f64 * 50.0,
            speed: Summary {
                mean: speed,
                max: speed,
            },
            age: Summary::default(),
            brain_units: Summary {
                mean: 100.0,
                max: 100.0,
            },
            sensor_load: Summary {
                mean: 16.0,
                max: 16.0,
            },
            genome_genes: Summary {
                mean: 120.0,
                max: 120.0,
            },
            mean_abs_connection_weight: 0.5,
            cumulative_energy_input: 0.0,
            cumulative_dissipation: 0.0,
            energy_rounding_reserve: 0.0,
            energy_drift: 0.0,
            arena_usage: Vec::new(),
            spawn_failures: None,
            structural_mutations: None,
        }
    }

    fn sample(tick: u64, population: u32, variants: u32) -> RunSample {
        RunSample {
            tick,
            evolving: world(population, variants, 1.0),
            random_control: world(population, variants, 1.0),
            final_state_hashes: None,
        }
    }

    #[test]
    fn mutation_pressure_is_separate_from_birth_pressure_and_live_complexity() {
        use sim_core::mutate::structural::StructuralMutationCounts;
        use sim_core::spawn::SpawnFailureCounts;

        let mut samples = vec![sample(0, 1, 1), sample(1_000, 0, 0)];
        for sample in &mut samples {
            for metrics in [&mut sample.evolving, &mut sample.random_control] {
                metrics.spawn_failures = Some(SpawnFailureCounts::default());
                metrics.structural_mutations = Some(StructuralMutationCounts::default());
            }
        }
        let evolving = samples[1].evolving.structural_mutations.as_mut().unwrap();
        evolving.add_neuron.attempted = 9;
        evolving.add_neuron.applied = 4;
        evolving.add_neuron.genome_limit = 2;
        evolving.add_neuron.scratch_limit = 3;
        let control = samples[1]
            .random_control
            .structural_mutations
            .as_mut()
            .unwrap();
        control.add_connection.attempted = 7;
        control.add_connection.innovation_exhausted = 7;
        let report = diagnose(&header(1_000), &samples);
        for (code, count) in [
            ("structural_genome_limit", 2),
            ("structural_scratch_limit", 3),
        ] {
            let finding = report.evolving.iter().find(|f| f.code == code).unwrap();
            assert!(
                finding
                    .signal
                    .contains(&format!("add_neuron: {count} cumulative"))
            );
            assert!(finding.signal.contains("not spawn refusals"));
        }
        let exhausted = report
            .random_control
            .iter()
            .find(|f| f.code == "structural_innovation_exhausted")
            .unwrap();
        assert!(exhausted.signal.contains("add_connection: 7 cumulative"));
        assert!(
            !report
                .evolving
                .iter()
                .any(|f| f.code == "structural_innovation_exhausted")
        );
        assert!(!report.evolving.iter().any(|f| f.code == "genome_limit"));
        assert!(
            !report
                .unavailable
                .iter()
                .any(|reason| reason.contains("structural-mutation"))
        );
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("M4 clustering"))
        );
    }

    #[test]
    fn absent_mutation_observations_are_unavailable_not_zero() {
        use sim_core::mutate::structural::StructuralMutationCounts;

        let mut samples = [sample(0, 1, 1)];
        samples[0].evolving.structural_mutations = Some(StructuralMutationCounts::default());
        let report = diagnose(&header(0), &samples);
        assert!(
            !report
                .unavailable
                .iter()
                .any(|reason| reason.contains("structural-mutation counts for evolving"))
        );
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("structural-mutation counts for scalar control"))
        );
        assert!(
            !report
                .random_control
                .iter()
                .any(|finding| finding.code.starts_with("structural_"))
        );
    }

    #[test]
    fn storage_pressure_is_reported_per_cohort_even_after_extinction() {
        use sim_core::spawn::SpawnFailureCounts;

        let mut samples = vec![sample(0, 1, 1), sample(1_000, 0, 0)];
        for sample in &mut samples {
            sample.evolving.spawn_failures = Some(SpawnFailureCounts::default());
            sample.random_control.spawn_failures = Some(SpawnFailureCounts::default());
        }
        samples[1]
            .evolving
            .spawn_failures
            .as_mut()
            .unwrap()
            .arena_capacity = 2;
        samples[1]
            .random_control
            .spawn_failures
            .as_mut()
            .unwrap()
            .arena_fragmentation = 3;
        let report = diagnose(&header(1_000), &samples);
        let capacity = report
            .evolving
            .iter()
            .find(|f| f.code == "storage_capacity")
            .unwrap();
        assert!(capacity.signal.contains("2 cumulative"));
        assert!(capacity.likely_causes[0].contains("total free elements"));
        assert!(
            !report
                .evolving
                .iter()
                .any(|f| f.code == "storage_fragmentation")
        );
        let fragmentation = report
            .random_control
            .iter()
            .find(|f| f.code == "storage_fragmentation")
            .unwrap();
        assert!(fragmentation.signal.contains("3 cumulative"));
        assert!(fragmentation.likely_causes[0].contains("contiguous"));
        assert!(
            !report
                .unavailable
                .iter()
                .any(|reason| reason.contains("spawn-refusal"))
        );
    }

    #[test]
    fn absent_spawn_observations_are_unavailable_not_zero() {
        let samples = [sample(0, 1, 1)];
        let report = diagnose(&header(0), &samples);
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| { reason.contains("spawn-refusal counts for evolving") })
        );
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| { reason.contains("spawn-refusal counts for scalar control") })
        );
        assert!(
            !report
                .evolving
                .iter()
                .any(|finding| finding.code.starts_with("storage_"))
        );
    }

    #[test]
    fn detects_deliberately_induced_early_extinction() {
        let samples = vec![
            sample(0, 100, 1),
            sample(1_000, 40, 1),
            sample(2_000, 0, 0),
            sample(10_000, 0, 0),
        ];
        let report = diagnose(&header(10_000), &samples);
        assert!(
            report
                .evolving
                .iter()
                .any(|finding| finding.code == "early_extinction")
        );
        assert!(report.comparison.findings.is_empty());
        assert!(
            report
                .comparison
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("living descendants"))
        );
    }

    #[test]
    fn detects_deliberately_induced_monoculture() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 99, 1),
            sample(2_000, 101, 1),
            sample(3_000, 100, 1),
            sample(4_000, 100, 1),
            sample(5_000, 100, 1),
        ];
        let report = diagnose(&header(5_000), &samples);
        assert!(
            report
                .evolving
                .iter()
                .any(|finding| finding.code == "monoculture")
        );
    }

    #[test]
    fn a_live_moving_population_is_not_extinct_or_idle() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 98, 4),
            sample(2_000, 102, 4),
            sample(3_000, 100, 4),
        ];
        let report = diagnose(&header(3_000), &samples);
        assert!(
            !report.evolving.iter().any(|finding| {
                matches!(
                    finding.code,
                    "early_extinction" | "extinction" | "stable_but_idle" | "monoculture"
                )
            }),
            "{:?}",
            report.evolving
        );
    }

    #[test]
    fn detects_idle_stability_and_brain_bloat_without_false_extinction() {
        let mut samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
            sample(3_000, 100, 4),
        ];
        for sample in &mut samples {
            sample.evolving.speed.mean = 0.0;
        }
        samples.last_mut().unwrap().evolving.brain_units.mean = 130.0;

        let report = diagnose(&header(3_000), &samples);
        assert!(
            report
                .evolving
                .iter()
                .any(|finding| finding.code == "stable_but_idle")
        );
        assert!(
            report
                .evolving
                .iter()
                .any(|finding| finding.code == "brain_bloat")
        );
        assert!(
            !report
                .evolving
                .iter()
                .any(|finding| finding.code.contains("extinction"))
        );
    }

    #[test]
    fn detects_energy_drift_relative_to_world_stock() {
        let mut samples = vec![sample(0, 100, 4), sample(1_000, 100, 4)];
        samples[1].evolving.total_energy = 1_000.0;
        samples[1].evolving.energy_drift = 2.0;
        let report = diagnose(&header(1_000), &samples);
        assert!(
            report
                .evolving
                .iter()
                .any(|finding| finding.code == "energy_drift")
        );
    }

    #[test]
    fn small_drift_is_harmless_against_large_energy_throughput() {
        let mut samples = vec![sample(0, 100, 4), sample(1_000, 100, 4)];
        samples[1].evolving.total_energy = 0.1;
        samples[1].evolving.cumulative_energy_input = 1_000_000.0;
        samples[1].evolving.cumulative_dissipation = 999_999.9;
        samples[1].evolving.energy_drift = 0.01;
        let report = diagnose(&header(1_000), &samples);
        assert!(
            !report
                .evolving
                .iter()
                .any(|finding| finding.code == "energy_drift")
        );
    }

    #[test]
    fn short_runs_do_not_claim_stable_idling() {
        let samples = vec![sample(0, 100, 4), sample(1, 100, 4), sample(2, 100, 4)];
        let report = diagnose(&header(2), &samples);
        assert!(
            !report
                .evolving
                .iter()
                .any(|finding| finding.code == "stable_but_idle")
        );
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("stable-idle"))
        );
    }

    #[test]
    fn recent_idle_samples_do_not_stand_in_for_an_idle_lifetime() {
        let mut samples = vec![
            sample(0, 100, 4),
            sample(9_746, 100, 4),
            sample(9_996, 100, 4),
            sample(9_997, 100, 4),
            sample(9_998, 100, 4),
            sample(9_999, 100, 4),
            sample(10_000, 100, 4),
        ];
        for sample in &mut samples[2..] {
            sample.evolving.speed.mean = 0.0;
        }

        let report = diagnose(&header(10_000), &samples);
        assert!(
            !report
                .evolving
                .iter()
                .any(|finding| finding.code == "stable_but_idle")
        );
    }

    #[test]
    fn invalid_idle_thresholds_are_reported_unavailable() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        let mut no_cost = header(2_000);
        no_cost.params.metabolism.base = 0.0;
        no_cost.params.metabolism.k_size = 0.0;
        no_cost.params.metabolism.k_brain = 0.0;
        no_cost.params.metabolism.k_sensor = 0.0;
        let report = diagnose(&no_cost, &samples);
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("idle metabolic cost is not positive"))
        );

        let mut no_speed = header(2_000);
        no_speed.params.movement.max_speed = 0.0;
        let report = diagnose(&no_speed, &samples);
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("movement.max_speed"))
        );
    }

    #[test]
    fn control_comparison_requires_the_treatment_to_have_occurred() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        let report = diagnose(&header(2_000), &samples);
        assert!(report.comparison.findings.is_empty());
        assert!(
            report
                .comparison
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("living descendants"))
        );
    }

    #[test]
    fn control_comparison_does_not_reuse_a_stale_eligible_tail() {
        let mut samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
            sample(3_000, 100, 4),
            sample(4_000, 0, 0),
        ];
        for sample in &mut samples[..4] {
            sample.evolving.descendants = 10;
            sample.random_control.descendants = 10;
        }

        let report = diagnose(&header(4_000), &samples);
        assert!(report.comparison.tail_means.is_none());
        assert!(report.comparison.findings.is_empty());
        assert!(
            report
                .comparison
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("living descendants"))
        );
    }

    #[test]
    fn control_comparison_uses_the_latest_eligible_population_suffix() {
        let mut samples = vec![
            sample(0, 9, 4),
            sample(1_000, 10, 4),
            sample(2_000, 10, 4),
            sample(3_000, 10, 4),
        ];
        for sample in &mut samples {
            sample.evolving.descendants = 5;
            sample.random_control.descendants = 5;
        }

        let report = diagnose(&header(3_000), &samples);
        assert!(report.comparison.unavailable.is_none());
        assert!(report.comparison.tail_means.is_some());
    }

    #[test]
    fn compares_live_descendant_cohorts_after_randomization() {
        let mut samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        for sample in &mut samples {
            sample.evolving.descendants = 10;
            sample.random_control.descendants = 10;
        }
        let report = diagnose(&header(2_000), &samples);
        assert!(
            report
                .comparison
                .findings
                .iter()
                .any(|finding| finding.code == "indistinguishable_from_random_control")
        );
        assert!(report.comparison.tail_means.is_some());
    }

    #[test]
    fn near_zero_control_metrics_are_treated_as_noise() {
        let mut samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        for sample in &mut samples {
            sample.evolving.descendants = 10;
            sample.random_control.descendants = 10;
            sample.evolving.agent_energy.mean = 1e-8;
            sample.random_control.agent_energy.mean = 2e-8;
        }

        let report = diagnose(&header(2_000), &samples);
        assert!(
            report
                .comparison
                .findings
                .iter()
                .any(|finding| finding.code == "indistinguishable_from_random_control")
        );
        assert!(
            report
                .comparison
                .tail_means
                .as_ref()
                .unwrap()
                .mean_agent_energy
                .relative_gap
                < 0.1
        );
    }

    #[test]
    fn distinct_control_metrics_are_reported_without_a_failure_claim() {
        let mut samples = vec![sample(0, 32, 4), sample(1_000, 32, 4), sample(2_000, 32, 4)];
        for sample in &mut samples {
            sample.evolving.descendants = 10;
            sample.random_control.descendants = 10;
            sample.evolving.agent_energy.mean = 13_362.7;
            sample.random_control.agent_energy.mean = 15_823.3;
            sample.evolving.speed.mean = 0.988;
            sample.random_control.speed.mean = 1.458;
        }

        let report = diagnose(&header(2_000), &samples);
        assert!(report.comparison.findings.is_empty());
        let metrics = report.comparison.tail_means.as_ref().unwrap();
        assert!(metrics.mean_agent_energy.relative_gap > 0.1);
        assert!(metrics.mean_speed.relative_gap > 0.1);
        assert_eq!(metrics.population.evolving, 32.0);
        assert_eq!(metrics.population.random_control, 32.0);
    }

    #[test]
    fn human_report_says_when_control_comparison_did_not_run() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        let report = diagnose(&header(2_000), &samples);
        let mut output = Vec::new();
        crate::diagnose_output::write_human(&mut output, &report).expect("writes");
        let output = String::from_utf8(output).expect("UTF-8");
        assert!(output.contains(
            "comparison:\n  - not run: scalar-control comparison requires living descendants"
        ));
    }
}
