# Phase 2 M8 — Founder and structural-evidence experiments

Multi-seed evidence for Phase 2's success criterion (spec §8: *brains grow in
complexity and distinct species appear*). This report records what headless runs
measured. It is not a judgment: whether anything interesting evolved is for a human
watching the sim to decide (spec §7.8 tier 3). Nothing here is ranked.

**Headline.** Every evolving world was viable, and the minimal chemo-led founder was
the most viable configuration, so it now ships. Genomes grew on every seed, but the
structural null (spec §7.8) grew them as much, so **genome growth here is not
evidence that lineages inherit useful structure**. The one consistent gap is
population in the dense configuration, where the evolving world ended 7–16% larger
than the null on all three seeds.

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

## Unranked shortlist for watching

With today's shipped defaults (minimal founder with oscillators, mutation enabled),
in the browser against the scalar control:

- **seed 42** — the sparse run with the most species. Are species behaviorally distinct
  or threshold labels?
- **seed 117** — the sparse run with the largest genomes. Are added neurons wired into
  behavior?
- **seed 314** — nearly sensorless in the sparse run. Do these agents forage or graze a
  rich food field?

## Limitations

- Three seeds per configuration; final samples only. Trajectories are reproducible
  from the command above but are not committed (`*.jsonl` is ignored).
- The null comparison ran before oscillators and deletion shipped; the shipped
  configuration's structural evidence would need a rerun.
- Species persistence uses 5k-tick sampling, so species living between samples are
  counted only in created/extinct totals.
