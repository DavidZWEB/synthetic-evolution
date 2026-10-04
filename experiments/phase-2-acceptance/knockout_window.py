"""Supply captured over one window of the knockout runs, intact against knockout.

Usage: python3 knockout_window.py RUN_DIR [START END], where RUN_DIR holds the
`SEED-intact.jsonl` and `SEED-knockout.jsonl` metrics files from the knockout
commands in docs/phase-2-m8-evidence.md, and the window defaults to ticks
350,000–400,000: the last 50,000 ticks, well after the retune at 300,000.

`native summarize` reports `supply_captured` over the second half of a run, which
here mixes 100,000 ticks before the knockout with 100,000 after it. This measures the
window alone, the same way: energy that moved into agents (the change in what they
hold plus what they dissipated) divided by the plant input offered over the window.
"""
import json
import sys

SEEDS = (42, 117, 314, 7, 2026)
COHORTS = (("evolving", "evolving"), ("random_control", "null"))


def load(path):
    header, samples = None, {}
    for line in open(path):
        record = json.loads(line)
        if record["kind"] == "header":
            header = record["data"]
        else:
            samples[record["data"]["tick"]] = record["data"]
    return header, samples


def captured(header, samples, cohort, start, end):
    params = header["params"]
    offered = params["plants"]["energy_input_rate"] * params["world"]["dt"] * (end - start)
    a, b = samples[start][cohort], samples[end][cohort]
    held = lambda m: m["total_energy"] - m["plant_energy"]
    eaten = held(b) - held(a) + b["cumulative_dissipation"] - a["cumulative_dissipation"]
    return eaten / offered


def main():
    run_dir = sys.argv[1]
    start, end = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (350_000, 400_000)
    print(f"supply captured, ticks {start}-{end}: intact / knockout; wired sensors at {end}")
    for seed in SEEDS:
        intact = load(f"{run_dir}/{seed}-intact.jsonl")
        knockout = load(f"{run_dir}/{seed}-knockout.jsonl")
        for cohort, label in COHORTS:
            wired = intact[1][end][cohort]["complexity"]["wiring"]["wired_sensors"]["mean"]
            print(
                f"{seed:>5} {label:8} {captured(*intact, cohort, start, end):.3f} / "
                f"{captured(*knockout, cohort, start, end):.3f}  wired sensors {wired:.2f}"
            )


if __name__ == "__main__":
    main()
