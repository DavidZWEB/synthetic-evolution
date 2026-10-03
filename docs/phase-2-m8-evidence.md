# Phase 2 M8 — Founder and structural-evidence experiments

First multi-seed evidence for Phase 2's success criterion (spec §8: *brains grow in
complexity and distinct species appear*). This report records what the headless runs
measured. It is not a judgment: whether anything interesting evolved is for a human
watching the sim to decide (spec §7.8 tier 3). Nothing here is ranked.

**Headline (updated with v2):** the first structural null (v1) was inconclusive
because it collapsed like the scalar control. Its human-approved replacement, v2
(donor topology, parent scalars), survives and makes the comparison informative.
**Over 400k ticks, v2 grew genomes as much as the evolving world did**, so genome growth
here cannot be credited to lineages inheriting useful structure. The one consistent
gap is population: in the dense configuration the evolving world ended 7–16% larger
than v2 on all three seeds; the sparse configuration showed no consistent gap (see
[Structural null v2 results](#structural-null-v2-results)).

## Provenance

| Item | Value |
|---|---|
| Source revision | `6909045` (main after #55; sim-core identical to `ab375db`, #54) |
| Seeds | 42, 117, 314 |
| Founders / ticks / sample interval | 2,000 / 200,000 / 5,000 |
| Controls | scalar `randomized_at_birth_v3` and structural null `structural_null_v1`, each a separate run beside the same evolving world |
| Command | `JOBS=6 scripts/experiment.sh OUT 200000 2000 5000 "42 117 314" experiments/phase-2-m8/*.json` |
| Configurations | [`dense-static`](../experiments/phase-2-m8/dense-static.json), [`dense-growth`](../experiments/phase-2-m8/dense-growth.json), [`sparse-chemo-growth`](../experiments/phase-2-m8/sparse-chemo-growth.json) |
| Full summaries | [`experiments/phase-2-m8/results/`](../experiments/phase-2-m8/results/) (text and JSON, per-seed values in the JSON) |

The evolving world's final state hashes matched between each seed's scalar and null
runs, as `summarize` requires, so each seed's three cohorts are a matched comparison.

Configurations, all otherwise at shipped defaults (the Phase 1 M12 tuning):

- **dense-static** — shipped dense founder (28 neurons, 240 connections, 16 sensor
  channels, 284 genes); all structural and sensor mutation off. Equivalent to the
  Phase 1 acceptance profile.
- **dense-growth** — the same founder with the documented opt-in rates: add connection
  0.05, add neuron 0.02, toggle 0.02, add/remove sensor 0.001 each. Neural deletion
  stays at zero pending D4 calibration, so neural gene count can only grow; toggling
  can still disable connections.
- **sparse-chemo-growth** — the minimal chemo-led founder (7 neurons, 4 connections,
  one 3-channel chemo sensor, 23 genes; no vision, energy sensor, hidden neurons, or
  oscillators) with the same rates.

## Results (final sample, mean ± sample sd [min–max] across 3 seeds)

| Metric | dense-static evolving | dense-growth evolving | sparse-chemo-growth evolving |
|---|---|---|---|
| population | 544 ± 64 [493–616] | 620 ± 55 [559–665] | 1393 ± 117 [1267–1497] |
| descendants | 538 ± 64 | 614 ± 54 | 1389 ± 119 |
| genome genes, mean | 284 (fixed) | 285.6 ± 0.5 [285.0–286.0] | 25.7 ± 0.7 [24.9–26.3] (from 23) |
| genome genes, max | 284 | 292.7 ± 1.2 [292–294] | 35 ± 2.6 [33–38] |
| neurons, mean / max | 28 / 28 | 28.4 / 31.7 | 7.6 / 11.7 (from 7) |
| connections, mean (enabled) | 240 (240) | 241.8 (241.0) | 6.4 (5.2) (from 4) |
| sensor load, mean | 16 | 13.6 ± 2.1 [12.4–16.0] | 2.0 ± 1.4 [0.3–2.9] (from 3) |
| active species | 1 | 1 | 33.7 ± 4.5 [29–38] |
| persistent species¹ | 1 | 1 | 12 ± 3 [9–15] |
| species created / extinct | 1 / 0 | 1 / 0 | 84.7 / 51 |
| structural edits applied | 0 | 1858 ± 272 | 3357 ± 360 |
| capacity refusals / peak arena use | 0 / 0.11 | 0 / 0.13 | 0 / 0.28 |
| wall seconds per 200k-tick run² | 365–541 | 426–568 | 181–192 |

¹ Alive at the end and first sampled at least 100k ticks earlier. ² Each run steps the
evolving and one control world on one core, with six runs in parallel on a 12-core
machine under other load. Sparse runs are about 2.5× faster despite about 2.2× the
population, because brains are about 20× smaller.

Controls (final sample; both protocols, all configurations):

| Configuration | scalar control population (descendants) | structural null population (descendants) |
|---|---|---|
| dense-static | 6.3 ± 2.1 (1 ± 1) | 7.7 ± 0.6 (1.3 ± 0.6) |
| dense-growth | 6 ± 2 (0.3 ± 0.6) | 7 ± 1 (0 ± 0) |
| sparse-chemo-growth | 20.7 ± 5.7 (7.7 ± 2.1) | 17.3 ± 5.5 (3.7 ± 3.2) |

In the growth configurations, every control population was down to 9–34 agents by tick
25,000 and never recovered; across all configurations controls ended at 4–27 agents,
mostly surviving generation-zero founders. No control sustained a lineage.

## Observations

- **Founder viability.** Every evolving world survived and grew after the
  generation-zero crash, in every configuration and seed. The sparse chemo-led founder
  was the most viable measured: its evolving populations ended about 2.2× larger than
  dense-growth, and its decline after the founder crash was shallower (431–1,082 at
  tick 25k vs 150–280 for dense). This is viability evidence for a simpler shipped
  founder, which spec/plan reserve for human approval; no default was changed.
- **Brain growth.** It is real but modest, and has not plateaued. In
  sparse-chemo-growth, mean genome size rose steadily on every seed (23 → 24.9–26.3)
  and the largest genome reached 33–38 genes (11–12 neurons). Connections grew more
  than neurons, and about 18% of connections are disabled. In dense-growth, the mean
  rose only about 1.5 genes over 200k ticks on a 284-gene base; growth is measurable
  but small against the existing brain.
- **Sensor loss.** Evolving sensor load fell in both growth configurations (dense
  16 → 13.6, sparse 3 → 2.0), with seed 314's sparse world at 0.34. With metabolic cost
  on sensors, removal is being retained. Some sparse lineages may be dropping their only
  chemo sensor. That is worth watching: it is either a real cost/benefit trade or
  agents surviving without perception in a rich food field.
- **Species.** The dense founders never split: one species throughout, because
  per-gene distance across 284-gene genomes barely moves at threshold 0.5. Sparse
  worlds started with 8–9 species at tick 0, purely from founder scalar variation on
  tiny genomes, and reached 29–38 active, of which 9–15 persisted for at least half the
  run. Some of this is threshold-created labelling, the confound M9 must rule out:
  small genomes make the same threshold much easier to cross. Distance/threshold
  calibration remains an open M0 gate.
- **Capacity.** There were no genome-limit, arena, or pool refusals, and peak arena use
  was at most 30%. Observed growth was not shaped by allocator bounds.

## The structural-null comparison is inconclusive

The approved protocol (spec §7.8) gives each null child a random living donor's
topology and then redraws every neural scalar, like the scalar control. Redrawing
scalars alone is already lethal at these settings: the scalar control collapses on its
own. So the null fails for the same reason, before structural inheritance can make any
difference. The data show this directly. In every configuration the two controls'
trajectories are indistinguishable from each other, while both differ from the
evolving world by two orders of magnitude.

Consequently:

- "Evolving ≫ structural null" here is fully explained by scalar inheritance and says
  nothing about whether **inherited structure** is useful.
- Gene-count growth in the evolving world cannot be credited to structural selection by
  this comparison. Spec §7.8 and the plan are explicit that a missing or inconclusive
  structural comparison is not a pass.

This is a property of the protocol at the current tuning, not an implementation fault.
The mechanism tests confirm the donor transfer, body retention, and scalar redraw.

## Decision needed

**Decided:** option 2, implemented as `structural_null_v2` (spec §7.8). The
options as they were presented follow.

Before M8 can complete, a human must choose how to proceed (spec §7.8 records design
changes after discussion). Options, without recommendation weight:

1. **Donor brain, parent body (`structural_null_v2`).** The child takes the donor's
   topology *and* its neural scalars, then receives the same scalar mutation as
   evolving offspring. Brains stay functional, but the parent-to-child neural lineage is
   broken: a body is paired with a random survivor's brain each generation. This tests
   whether lineage-coupled brains matter, not structure separately from weights.
2. **Donor topology with parent-matched scalars.** The child takes donor topology; genes
   it shares with the parent (same innovation ID) take the parent's scalars, and the
   rest take the donor's, then mutate. This isolates topology inheritance more closely,
   but shared founder innovations make child and parent nearly identical until
   structure diverges.
3. **Keep v1 and record it as uninformative** at this tuning, accepting that M9 has no
   structural evidence. Per the plan, this blocks Phase 2 acceptance rather than
   waiving it.

Whichever is chosen also needs the same three configurations rerun; a growth
configuration with nonzero deletion (after D4 calibration) would make "growth" a
balance between gain and loss rather than a ratchet.

## Structural null v2 results

Rerun of both growth configurations under `structural_null_v2` (spec §7.8) beside the
scalar control, at revision `1d120c8` (main after #58; dense founder still the default,
so `dense-growth` and `sparse-chemo-growth` are exactly the configurations above),
seeds 42/117/314, 2,000 founders, **400,000 ticks**, samples every 5,000. Summaries are
in [`experiments/phase-2-m8/results-v2/`](../experiments/phase-2-m8/results-v2/).

Per seed (42, 117, 314), final sample, evolving vs structural null v2:

| Metric | dense-growth evolving | dense-growth v2 | sparse evolving | sparse v2 |
|---|---|---|---|---|
| population | 681, 696, 676 | 583, 635, 608 | 1559, 1327, 1592 | 1464, 1464, 1956 |
| genome genes, mean | 289.4, 287.7, 284.9 | 288.0, 287.7, 290.3 | 27.9, 28.8, 26.5 | 29.8, 27.8, 28.5 |
| neurons, mean | 28.9, 28.6, 28.4 | 29.0, 28.8, 29.6 | 8.3, 7.9, 8.1 | 8.6, 8.5, 8.4 |
| connections, mean | 244.6, 244.2, 242.2 | 243.8, 243.2, 244.8 | 8.5, 9.3, 7.4 | 9.6, 7.6, 9.2 |
| sensor load, mean | 15.3, 11.9, 10.2 | 13.1, 15.1, 16.0 | 0.35, 1.79, 0.02 | 2.00, 1.95, 0.02 |
| active species | 1, 1, 1 | 1, 1, 1 | 29, 40, 57 | 33, 36, 40 |
| persistent species | 1, 1, 1 | 1, 1, 1 | 16, 23, 16 | 23, 28, 18 |

The scalar control again collapsed (4–24 agents at the end) in both configurations;
the evolving worlds' final hashes matched between each seed's two runs.

What this shows:

- **Genome growth is not evidence of useful inherited structure here.** v2 children
  take a random living agent's structure, yet v2 genomes grew as much as or more than
  the evolving world's in both configurations (sparse mean 23 → 27.8–29.8 vs
  26.5–28.8). On sparse seeds 42 and 314, v2 grew faster throughout the run. Selection
  on which structures survive, plus mutation, produces this growth without any
  lineage-specific fit between structure and weights.
- **Lineage coupling matters for population in the dense configuration.** The evolving
  world ended larger than v2 on all three dense seeds (paired gaps of 98, 61, and 68
  agents). This is consistent with dense brains whose evolved weights fit their own
  structure, but three seeds and one metric are thin, and a human should weigh it.
  The sparse configuration shows no consistent population gap (one seed each way, one
  near-equal).
- **Sensor loss is lineage-driven in dense, ambiguous in sparse.** Dense evolving
  worlds kept fewer sensors than v2 on two of three seeds. Sparse seed 314 lost nearly
  all sensors in *both* worlds (0.02): that world is mostly sensorless and still
  thriving, which a human should look at before reading any behavior into it.
- **Species are not distinguished by the null.** v2 forms as many, and as persistent,
  species as the evolving world, so species counts alone do not show heritable
  structural divergence beyond what donor mixing also produces.

Interpretation limits: three seeds; final samples only; v2 tests coupling between a
lineage's structure and its weights, not whether a particular circuit is adaptive;
neural deletion was off, so growth is a ratchet tempered only by selection and
disabling. v2 runs take about 1.7× longer than scalar-control runs because the null
world sustains a full population.

## Unranked shortlist for watching

Same-seed browser runs are reproducible from the params files above. In the browser,
the structural null is not selectable; compare against the scalar control.

- **sparse-chemo-growth, seed 42** — the most species (38 active) and steady genome
  growth. Check whether species are behaviorally distinct or threshold labels.
- **sparse-chemo-growth, seed 117** — the largest genomes (max 38 genes). Check whether
  added neurons are wired into behavior.
- **sparse-chemo-growth, seed 314** — mean sensor load 0.34 at 200k and 0.02 at 400k
  (both evolving and v2): nearly sensorless yet thriving. Check whether these agents
  forage or simply graze a rich food field.
- **dense-growth, seed 42** — the dense baseline with growth on, for comparison with the
  Phase 1 behavior already accepted.

## Limitations

- Three seeds per configuration; spreads are sample standard deviations over n = 3.
- Final-sample statistics hide transients; trajectories are in the per-run metrics,
  which are reproducible from the command above but are not committed (`*.jsonl` is
  ignored).
- 200k ticks is short for dense growth; sparse trends had not plateaued.
- Species persistence uses 5k-tick sampling, so short-lived species between samples
  are invisible to it, though they are counted in created/extinct.
