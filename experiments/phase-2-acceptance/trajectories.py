"""Window means of population and structure through each acceptance run, as JSON.

Usage: python3 trajectories.py RUN.jsonl... > results/trajectories.json, given the
`CONFIG-SEED-CONTROL.jsonl` metrics files from the `scripts/experiment.sh` command in
docs/phase-2-m8-evidence.md. For each run and cohort it averages the samples in the
50,000 ticks before each mark: population, wired hidden neurons, genome genes,
enabled connections, and wired sensors. An extinct or empty cohort counts as zero
population and has no structure to average.
"""
import json
import statistics
import sys

MARKS = [100_000, 250_000, 500_000, 750_000, 1_000_000]
WINDOW = 50_000


def structure(cohort):
    """Mean structure of a living cohort, or None for an empty one."""
    if not cohort.get("complexity") or not cohort["population"]:
        return None
    complexity = cohort["complexity"]
    return {
        "hidden": complexity["wiring"]["wired_hidden_neurons"]["mean"],
        "genes": complexity["genome_genes"]["mean"],
        "conns": complexity["enabled_connections"]["mean"],
        "sensors": complexity["wiring"]["wired_sensors"]["mean"],
    }


def windowed(rows, cohort, metric, mark):
    values = []
    for row in rows:
        if not mark - WINDOW < row["tick"] <= mark:
            continue
        data = row.get(cohort)
        if metric == "pop":
            values.append(data["population"] if data else 0)
        elif data and (measured := structure(data)) is not None:
            values.append(measured[metric])
    return statistics.mean(values) if values else None


def main():
    runs = {}
    for path in sys.argv[1:]:
        rows = [json.loads(line)["data"] for line in open(path)]
        rows = [row for row in rows if "tick" in row and "evolving" in row]
        cohorts = [c for c in ("evolving", "random_control") if c in rows[0]]
        name = path.rsplit("/", 1)[-1].removesuffix(".jsonl")
        runs[name] = {
            cohort: {
                metric: [windowed(rows, cohort, metric, mark) for mark in MARKS]
                for metric in ("pop", "hidden", "genes", "conns", "sensors")
            }
            for cohort in cohorts
        }
    json.dump({"marks": MARKS, "window": WINDOW, "runs": runs}, sys.stdout, indent=1)


if __name__ == "__main__":
    main()
