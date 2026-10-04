# Phase 2 M8 — Founder and structural-evidence experiments

Multi-seed evidence for Phase 2's success criterion (spec §8: *brains grow in
complexity and distinct species appear*). This report records what headless runs
measured. It is not a judgment: whether anything interesting evolved is for a human
watching the sim to decide (spec §7.8 tier 3). Nothing here is ranked.

**Headline.** Every evolving world was viable, and the minimal chemo-led founder was
the most viable configuration, so it now ships. Brains grow, and on the shipped
configuration that growth is still **not distinguished from the structural null**
(spec §7.8): lineages that inherit their own structure grow no more than lineages
given a random survivor's. The clearest consistent signal points elsewhere:
**evolving lineages shed perception**. They keep fewer sensors than the null on
every seed, and late species increasingly have no sensor wired to any effector,
running clock-driven, open-loop behavior instead. The likely reason is that the
environment barely rewards perception; that, not the genetic machinery, is the next
thing to investigate. Species counts are set mostly by the threshold; deletion's
marker churn is a minor effect.

## Provenance

| Item | Value |
|---|---|
| Source revision | `1d120c8` (main after #58) |
| Seeds | 42, 117, 314 |
| Founders / ticks / sample interval | 2,000 / 400,000 / 5,000 |
| Controls | scalar `randomized_at_birth_v3` and structural null `structural_null_v2`, each a separate run beside the same evolving world |
| Command | `JOBS=6 scripts/experiment.sh OUT 400000 2000 5000 "42 117 314" dense-growth.json sparse-chemo-growth.json` |
| Summaries | [`experiments/phase-2-m8/results/`](../experiments/phase-2-m8/results/) (text and JSON, per-seed values in the JSON) |

The evolving world's final state hashes matched between each seed's scalar and null
runs, as `summarize` requires, so each seed's three cohorts are a matched comparison.

Both configurations used the then-shipped Phase 1 tuning with the growth rates add
connection 0.05, add neuron 0.02, toggle 0.02, add/remove sensor 0.001, and no neural
deletion:

- **dense-growth** — Phase 1's dense founder: 28 neurons, 240 connections, 16 sensor
  channels, 284 genes.
- **sparse-chemo-growth** — the minimal chemo-led founder: 7 neurons, 4 connections,
  one 3-channel chemo sensor, 23 genes; no vision, energy sensor, hidden neurons, or
  oscillators.

## Founder viability

| Final population (seeds 42, 117, 314) | Evolving | Scalar control |
|---|---|---|
| dense-growth | 681, 696, 676 | 4–8 |
| sparse-chemo-growth | 1559, 1327, 1592 | 12–24 |

The sparse founder ended about 2.2× larger. On a human's approval it became the
shipped founder, later with two oscillators restored; the scalar control collapsed
in every run, as in Phase 1. Two follow-up checks on the shipped founder (seeds
42/117/314, 2,000 founders, 200k ticks) stayed viable: with two oscillators,
1,514–1,590 agents; with every structural and sensor mutation enabled (the shipped
rates, including deletion), 1,315–1,645 agents with mean genomes of 26–34 genes.

## Structural null comparison

Per seed (42, 117, 314), final sample, evolving vs structural null:

| Metric | dense evolving | dense null | sparse evolving | sparse null |
|---|---|---|---|---|
| population | 681, 696, 676 | 583, 635, 608 | 1559, 1327, 1592 | 1464, 1464, 1956 |
| genome genes, mean | 289.4, 287.7, 284.9 | 288.0, 287.7, 290.3 | 27.9, 28.8, 26.5 | 29.8, 27.8, 28.5 |
| neurons, mean | 28.9, 28.6, 28.4 | 29.0, 28.8, 29.6 | 8.3, 7.9, 8.1 | 8.6, 8.5, 8.4 |
| connections, mean | 244.6, 244.2, 242.2 | 243.8, 243.2, 244.8 | 8.5, 9.3, 7.4 | 9.6, 7.6, 9.2 |
| sensor load, mean | 15.3, 11.9, 10.2 | 13.1, 15.1, 16.0 | 0.35, 1.79, 0.02 | 2.00, 1.95, 0.02 |
| active species | 1, 1, 1 | 1, 1, 1 | 29, 40, 57 | 33, 36, 40 |
| persistent species | 1, 1, 1 | 1, 1, 1 | 16, 23, 16 | 23, 28, 18 |

- **Genome growth is not evidence of useful inherited structure.** Null children take
  a random survivor's structure, yet null genomes grew as much as or more than the
  evolving world's (sparse mean 23 → 27.8–29.8 vs 26.5–28.8); on sparse seeds 42 and
  314 the null grew faster throughout. Selection on which structures survive, plus
  mutation, produces this growth without lineage-specific fit.
- **Lineage coupling matters for population in the dense configuration:** paired gaps
  of 98, 61, and 68 agents in the evolving world's favor. Consistent with dense
  brains whose weights fit their own structure, but three seeds and one metric are
  thin. Sparse shows no consistent gap.
- **Sensors:** dense evolving worlds kept fewer sensors than the null on two of three
  seeds. Sparse seed 314 is nearly sensorless in *both* worlds yet thriving.
- **Species are not distinguished by the null.** It forms as many, and as persistent,
  species as the evolving world. Dense founders never speciated at threshold 0.5;
  sparse worlds' counts are partly threshold labelling on small genomes.
- No capacity refusals; peak arena use at most about 40%.

## Shipped configuration rerun

Today's shipped defaults (minimal founder with two oscillators, every structural and
sensor mutation enabled, including deletion), revision `9ae8c0a`, seeds 42/117/314,
2,000 founders, 400,000 ticks, both controls. Summaries:
[`experiments/phase-2-m8/shipped/`](../experiments/phase-2-m8/shipped/).

| Per seed (42, 117, 314) | evolving | structural null | scalar control |
|---|---|---|---|
| population | 1450, 1809, 1665 | 1412, 1477, 1971 | 20, 18, 30 |
| genome genes, mean | 36.9, 26.6, 27.0 | 32.0, 28.9, 35.1 | 25 |
| genome genes, max | 51, 36, 40 | 40, 41, 43 | 25–26 |
| connections, mean | 12.6, 6.2, 6.2 | 9.1, 7.2, 10.5 | 4 |
| **sensor load, mean** | **1.78, 0.17, 0.43** | **2.61, 1.46, 2.96** | 3 |
| mean agent energy | 127, 137, 211 | 1056, 1796, 151 | ~20,000 |
| active / persistent species | 37/8, 66/34, 68/24 | 33/27, 40/39, 23/14 | 5–7 |

- **Growth is still not distinguished from the null**: genome size and population go
  each way across seeds.
- **Evolving lineages shed sensors on every seed**, keeping 12–68% of the null's sensor
  load. Sensors cost metabolism; dropping them is selected when perception does not
  pay its way.
- Evolving agents hold far less energy each and turn over faster (1.2–2.4× the
  null's structural edits, which track births): they spend energy on reproduction.
- No capacity refusals; peak arena use at most 39%.

## Functional wiring

What the added structure does, measured on species-founding genomes recorded in one
shipped run (seed 42, 400k ticks, `--history --representatives`, 80 evolving
species) with [`wiring.py`](../experiments/phase-2-m8/wiring.py); output in
[`wiring-seed42.txt`](../experiments/phase-2-m8/wiring-seed42.txt). A neuron is
*functional* when it lies on an enabled path from an input (sensor target or
oscillator) to an output (effector source).

| Species founded at | genes | hidden neurons | functional hidden | functional connections | sensors wired to an output | effectors driven |
|---|---|---|---|---|---|---|
| tick 0 (founders) | 25 | 0 | 0 | 4 | 1.00 | 4.0 |
| 100k–200k | 35.7 | 3.2 | 2.6 | 7.1 | 1.00 | 3.5 |
| 200k–300k | 37.7 | 4.5 | 2.8 | 7.2 | 0.67 | 2.9 |
| 300k–400k | 36.5 | 4.1 | 1.8 | 5.4 | 0.62 | 2.3 |

Early growth is mostly functional (about 80% of added hidden neurons sit on a working
path through 200k). Later, functional structure shrinks while genomes stay large:
about 38% of the last period's species have no sensor connected to any effector, and only
2.3 of 4 effectors are driven by anything (the rest output a constant). Oscillators
increasingly carry the wiring. One seed, so this is a lead, not a measurement across
seeds; it agrees with the sensor loss above.

## Species calibration

Shipped defaults, 200,000 ticks, seeds 42/117/314. Classification does not affect
dynamics, so the threshold variants are identical worlds labelled differently.
Summaries: [`experiments/phase-2-m8/calibration/`](../experiments/phase-2-m8/calibration/).

| Evolving, per seed | active species | persistent | created |
|---|---|---|---|
| shipped (threshold 0.5) | 18, 46, 48 | 6, 18, 17 | 40, 81, 84 |
| deletion off | 21, 38, 32 | 8, 15, 13 | 54, 70, 68 |
| threshold 0.3 | 41, 146, 119 | 10, 43, 31 | 191, 525, 480 |
| threshold 0.8 | 4, 10, 9 | 4, 5, 4 | 5, 12, 9 |

- **The threshold sets the species count**: moving it from 0.5 to 0.3 roughly triples
  active species, and 0.8 leaves a handful. Species counts are labels relative to the
  threshold, not a measure of divergence on their own.
- **Deletion's marker churn is a minor effect**: with deletion off, species counts move
  within seed-to-seed variation (lower on two seeds, higher on one). Shipping deletion
  before calibration did not materially inflate labels.
- Species claims should report persistent species at more than one threshold rather
  than a single count.

## Unranked shortlist for watching

With today's shipped defaults, in the browser against the scalar control:

- **seed 42** — the largest genomes in the shipped rerun (max 51 genes). Are the added
  neurons and clocks visible in behavior?
- **seed 117** — nearly sensorless (mean sensor load 0.17) yet the largest population.
  Do agents forage, or graze on a timer?
- **seed 314** — the most species. Are they behaviorally distinct or threshold labels?

## Limitations

- Three seeds per configuration; final samples only. Trajectories are reproducible
  from the command above but are not committed (`*.jsonl` is ignored).
- The functional-wiring analysis covers one seed and species-founding genomes, not
  whole populations.
- Species persistence uses 5k-tick sampling, so species living between samples are
  counted only in created/extinct totals.
