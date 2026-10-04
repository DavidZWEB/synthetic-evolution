//! Human-readable rendering of diagnostic reports.
//!
//! Diagnosis and machine-readable report construction stay in `diagnose`; this module
//! only makes availability, observations, and findings unambiguous at the terminal.

use std::io::{self, Write};

use crate::diagnose::{DiagnosisReport, Finding, HistoryStatus, MetricComparison};
use crate::metrics::SizeDistribution;

pub(crate) fn print_human(report: &DiagnosisReport) -> io::Result<()> {
    let stdout = io::stdout();
    write_human(&mut stdout.lock(), report)
}

pub(crate) fn write_human(output: &mut impl Write, report: &DiagnosisReport) -> io::Result<()> {
    writeln!(output, "samples: {}", report.samples)?;
    write_findings(output, "evolving", &report.evolving)?;
    write_findings(output, report.control_label, &report.random_control)?;
    writeln!(
        output,
        "species (observational labels, not adaptive success; monoculture uses exact genomes):"
    )?;
    for (name, summary) in [
        ("evolving", &report.species.evolving),
        (report.control_label, &report.species.random_control),
    ] {
        if let Some(summary) = summary {
            writeln!(
                output,
                "  - {name} at tick {}: active_species={}/{} capacity, unclassified_population={}",
                summary.tick,
                summary.active_species,
                summary.species_capacity,
                summary.unclassified_population
            )?;
        } else {
            writeln!(output, "  - {name}: unavailable")?;
        }
    }
    writeln!(
        output,
        "complexity (live genome sizes, not fitness; min/p25/median/p75/max, mean):"
    )?;
    for (name, summary) in [
        ("evolving", &report.complexity.evolving),
        (report.control_label, &report.complexity.random_control),
    ] {
        let Some(summary) = summary else {
            writeln!(output, "  - {name}: unavailable")?;
            continue;
        };
        let sizes = &summary.distributions;
        writeln!(output, "  - {name} at tick {}:", summary.tick)?;
        for (label, distribution) in [
            ("genome genes", &sizes.genome_genes),
            ("neurons", &sizes.neurons),
            ("connections", &sizes.connections),
            ("enabled connections", &sizes.enabled_connections),
        ] {
            writeln!(output, "      {label}: {}", distribution_text(distribution))?;
        }
        match &sizes.wiring {
            Some(wiring) => {
                for (label, distribution) in [
                    ("wired hidden neurons", &wiring.wired_hidden_neurons),
                    ("wired sensors", &wiring.wired_sensors),
                    ("driven effectors", &wiring.driven_effectors),
                ] {
                    writeln!(output, "      {label}: {}", distribution_text(distribution))?;
                }
            }
            None => writeln!(output, "      wiring: not recorded")?,
        }
    }
    writeln!(output, "history capture:")?;
    for (name, status) in [
        ("evolving", &report.history.evolving),
        (report.control_label, &report.history.random_control),
    ] {
        match status {
            HistoryStatus::Unknown => writeln!(
                output,
                "  - {name}: unknown (this metrics schema did not record capture)"
            )?,
            HistoryStatus::Off => writeln!(output, "  - {name}: off")?,
            HistoryStatus::Complete { tick, availability } => writeln!(
                output,
                "  - {name}: complete through tick {tick} ({} retained events, capacity {})",
                availability.retained_events, availability.capacity
            )?,
            HistoryStatus::Incomplete { tick, availability } => writeln!(
                output,
                "  - {name}: incomplete through tick {tick}: {} dropped events in {} gaps ({} retained, capacity {})",
                availability.dropped_events,
                availability.gaps,
                availability.retained_events,
                availability.capacity
            )?,
        }
    }
    writeln!(output, "comparison:")?;
    if let Some(reason) = &report.comparison.unavailable {
        writeln!(output, "  - not run: {reason}")?;
    } else {
        if let Some(metrics) = &report.comparison.tail_means {
            write_comparison_metric(output, "population", &metrics.population)?;
            write_comparison_metric(output, "mean agent energy", &metrics.mean_agent_energy)?;
            write_comparison_metric(output, "mean speed", &metrics.mean_speed)?;
        }
        if report.comparison.findings.is_empty() {
            writeln!(output, "  - no known failure signature detected")?;
        }
        write_finding_list(output, &report.comparison.findings)?;
    }
    writeln!(output, "unavailable:")?;
    for item in &report.unavailable {
        writeln!(output, "  - {item}")?;
    }
    Ok(())
}

fn distribution_text(distribution: &SizeDistribution) -> String {
    format!(
        "{}/{}/{}/{}/{}, mean {:.2}",
        distribution.min,
        distribution.p25,
        distribution.median,
        distribution.p75,
        distribution.max,
        distribution.mean
    )
}

fn write_findings(output: &mut impl Write, name: &str, findings: &[Finding]) -> io::Result<()> {
    writeln!(output, "{name}:")?;
    if findings.is_empty() {
        writeln!(output, "  - no known failure signature detected")?;
    }
    write_finding_list(output, findings)
}

fn write_finding_list(output: &mut impl Write, findings: &[Finding]) -> io::Result<()> {
    for finding in findings {
        writeln!(output, "  - {}: {}", finding.code, finding.signal)?;
        writeln!(output, "    likely: {}", finding.likely_causes.join("; "))?;
    }
    Ok(())
}

fn write_comparison_metric(
    output: &mut impl Write,
    name: &str,
    metric: &MetricComparison,
) -> io::Result<()> {
    writeln!(
        output,
        "  - {name}: evolving={:.6}, random_control={:.6}, relative_gap={:.3}",
        metric.evolving, metric.random_control, metric.relative_gap
    )
}
