//! Maps metrics-series signatures to the known failure modes in spec §10.
//!
//! Diagnostics are evidence, not a score: every detected signal is reported alongside
//! its likely causes, and systems not present in the current phase are explicitly
//! unavailable rather than inferred from unrelated numbers.

use std::io;

use serde::Serialize;

use crate::Result;
use crate::cli::DiagnoseArgs;
use crate::metrics::{RunHeader, RunSample, WorldMetrics};
use crate::metrics_reader::read_metrics;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub signal: String,
    pub likely_causes: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiagnosisReport {
    pub samples: usize,
    pub evolving: Vec<Finding>,
    pub random_control: Vec<Finding>,
    pub comparison: Vec<Finding>,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Copy)]
enum Cohort {
    Evolving,
    Control,
}

pub fn run(args: DiagnoseArgs) -> Result<()> {
    let data = read_metrics(&args.metrics)?;
    let report = diagnose(&data.header, &data.samples);
    if args.json {
        serde_json::to_writer_pretty(io::stdout(), &report)?;
        println!();
    } else {
        print_human(&report);
    }
    Ok(())
}

pub fn diagnose(header: &RunHeader, samples: &[RunSample]) -> DiagnosisReport {
    let (comparison, comparison_unavailable) = compare_control(samples);
    let observed_ticks = samples
        .last()
        .zip(samples.first())
        .map_or(0, |(last, first)| last.tick.saturating_sub(first.tick));
    let idle_observation = required_idle_observation(header, samples);
    let mut unavailable = vec![
        "species-cluster diagnostics require Phase 2; Phase 1 monoculture uses exact genome variants".to_owned(),
        "predator/prey diagnostics require Phase 3 trophic roles".to_owned(),
        "signal-correlation diagnostics require Phase 4 signaling".to_owned(),
    ];
    if let Some(reason) = comparison_unavailable {
        unavailable.push(reason);
    }
    if observed_ticks < idle_observation {
        unavailable.push(format!(
            "stable-idle diagnosis requires {idle_observation} observed ticks; file covers {observed_ticks}"
        ));
    }
    DiagnosisReport {
        samples: samples.len(),
        evolving: diagnose_cohort(
            header,
            samples,
            Cohort::Evolving,
            observed_ticks >= idle_observation,
        ),
        random_control: diagnose_cohort(
            header,
            samples,
            Cohort::Control,
            observed_ticks >= idle_observation,
        ),
        comparison,
        unavailable,
    }
}

fn diagnose_cohort(
    header: &RunHeader,
    samples: &[RunSample],
    cohort: Cohort,
    can_diagnose_idling: bool,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let metrics: Vec<(u64, &WorldMetrics)> = samples
        .iter()
        .map(|sample| (sample.tick, select(sample, cohort)))
        .collect();

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
        return findings;
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

    if can_diagnose_idling && live_tail.len() >= 3 {
        let min_population = live_tail
            .iter()
            .map(|(_, sample)| sample.population)
            .min()
            .unwrap_or(0);
        let max_population = live_tail
            .iter()
            .map(|(_, sample)| sample.population)
            .max()
            .unwrap_or(0);
        let mean_speed = live_tail
            .iter()
            .map(|(_, sample)| sample.speed.mean)
            .sum::<f64>()
            / live_tail.len() as f64;
        let idle_speed = header.params.movement.max_speed as f64 * 0.001;
        if max_population > 0
            && min_population as f64 >= max_population as f64 * 0.9
            && mean_speed < idle_speed
        {
            findings.push(Finding {
                code: "stable_but_idle",
                signal: format!(
                    "population stayed within {min_population}..={max_population} while mean speed was {mean_speed:.4} (idle threshold {idle_speed:.4})"
                ),
                likely_causes: vec!["metabolism too cheap; idling is viable"],
            });
        }
    }

    findings
}

fn required_idle_observation(header: &RunHeader, samples: &[RunSample]) -> u64 {
    let Some(first) = samples
        .iter()
        .map(|sample| &sample.evolving)
        .find(|sample| sample.population > 0)
    else {
        return 0;
    };
    let params = &header.params;
    let cost = params.metabolism.base as f64
        + params.metabolism.k_size as f64 * params.body.size as f64 * params.body.size as f64
        + params.metabolism.k_brain as f64 * first.brain_units.mean
        + params.metabolism.k_sensor as f64 * first.sensor_load.mean;
    let idle_lifetime = if cost <= 0.0 {
        0
    } else {
        (params.reproduction.start_energy as f64 / cost).ceil() as u64
    };
    idle_lifetime.max(params.reproduction.maturity_ticks as u64)
}

fn compare_control(samples: &[RunSample]) -> (Vec<Finding>, Option<String>) {
    let tail: Vec<_> = samples
        .iter()
        .rev()
        .filter(|sample| sample.evolving.descendants > 0 && sample.random_control.descendants > 0)
        .take(5)
        .collect();
    if tail.len() < 3 {
        return (
            Vec::new(),
            Some(
                "random-control comparison requires at least three samples with living descendants in both cohorts"
                    .to_owned(),
            ),
        );
    }
    if tail
        .iter()
        .any(|sample| sample.evolving.population < 10 || sample.random_control.population < 10)
    {
        return (
            Vec::new(),
            Some(
                "random-control comparison requires at least 10 living agents in both cohorts"
                    .to_owned(),
            ),
        );
    }
    let mean = |value: fn(&WorldMetrics) -> f64, cohort: Cohort| {
        tail.iter()
            .map(|sample| value(select(sample, cohort)))
            .sum::<f64>()
            / tail.len() as f64
    };
    let population_gap = relative_gap(
        mean(|m| m.population as f64, Cohort::Evolving),
        mean(|m| m.population as f64, Cohort::Control),
    );
    let energy_gap = relative_gap(
        mean(|m| m.agent_energy.mean, Cohort::Evolving),
        mean(|m| m.agent_energy.mean, Cohort::Control),
    );
    let speed_gap = relative_gap(
        mean(|m| m.speed.mean, Cohort::Evolving),
        mean(|m| m.speed.mean, Cohort::Control),
    );

    if population_gap < 0.1 && energy_gap < 0.1 && speed_gap < 0.1 {
        (
            vec![Finding {
                code: "indistinguishable_from_random_control",
                signal: format!(
                    "single-seed tail relative gaps: population={population_gap:.3}, energy={energy_gap:.3}, speed={speed_gap:.3}"
                ),
                likely_causes: vec![
                    "observed behavior may not reflect cumulative neural evolution",
                    "repeat across seeds before drawing a conclusion",
                ],
            }],
            None,
        )
    } else {
        (Vec::new(), None)
    }
}

fn relative_gap(a: f64, b: f64) -> f64 {
    let scale = a.abs().max(b.abs()).max(1e-9);
    (a - b).abs() / scale
}

fn select(sample: &RunSample, cohort: Cohort) -> &WorldMetrics {
    match cohort {
        Cohort::Evolving => &sample.evolving,
        Cohort::Control => &sample.random_control,
    }
}

fn print_human(report: &DiagnosisReport) {
    println!("samples: {}", report.samples);
    print_cohort("evolving", &report.evolving);
    print_cohort("random control", &report.random_control);
    print_cohort("comparison", &report.comparison);
    println!("unavailable:");
    for item in &report.unavailable {
        println!("  - {item}");
    }
}

fn print_cohort(name: &str, findings: &[Finding]) {
    println!("{name}:");
    if findings.is_empty() {
        println!("  - no known Phase 1 failure signature detected");
    }
    for finding in findings {
        println!("  - {}: {}", finding.code, finding.signal);
        println!("    likely: {}", finding.likely_causes.join("; "));
    }
}

#[cfg(test)]
mod tests {
    use sim_core::params::SimParams;

    use super::*;
    use crate::metrics::{SCHEMA_VERSION, Summary};

    fn header(ticks: u64) -> RunHeader {
        RunHeader {
            schema_version: SCHEMA_VERSION,
            sim_version: "test".to_owned(),
            source_revision: "test-revision".to_owned(),
            phase: 1,
            seed: "42".to_owned(),
            ticks,
            founders: 100,
            sample_every: 1_000,
            params: SimParams::default(),
            control: "randomized_at_birth".to_owned(),
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
            energy_drift: 0.0,
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
        assert!(report.comparison.is_empty());
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("living descendants"))
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
    fn control_comparison_requires_the_treatment_to_have_occurred() {
        let samples = vec![
            sample(0, 100, 4),
            sample(1_000, 100, 4),
            sample(2_000, 100, 4),
        ];
        let report = diagnose(&header(2_000), &samples);
        assert!(report.comparison.is_empty());
        assert!(
            report
                .unavailable
                .iter()
                .any(|reason| reason.contains("living descendants"))
        );
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
                .iter()
                .any(|finding| finding.code == "indistinguishable_from_random_control")
        );
    }
}
