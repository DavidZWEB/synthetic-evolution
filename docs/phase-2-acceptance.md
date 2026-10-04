# Phase 2 — Acceptance (M10)

The record of Phase 2's acceptance decision: what was run, what it measured, what was
watched, and the call that was made. The earlier experiments it builds on are in
[`phase-2-m8-evidence.md`](phase-2-m8-evidence.md); the plan is
[`phase-2-implementation-plan.md`](phase-2-implementation-plan.md).

## Who decided, and how

Spec §8 makes every phase's success criterion a human judgment, and spec §7.8 says
that judgment cannot be certified by tests. **For this phase the judgment was
delegated.** On 2026-10-04, David Webster, the project's owner, asked the coding agent
(Claude, working in Claude Code) to do M10's watching and to make the final call
itself. So the decision below is the agent's, not a human's. The agent watched an
evolved world in the browser and read the multi-seed metrics. No human watched these
runs for this decision.

`AGENTS.md` requires a human to watch and judge a phase's success. It records this
delegation as its single exception, so the rule stands for every later phase.

The record is written so that a human reviewer can check the reasoning against the
evidence and overturn the call. What would change it is listed under
[Decision](#decision).

## Criterion

> *Success: brains grow in complexity, distinct species appear.* (spec §8)

The plan adds two conditions:

- Useful inherited complexity and persistent species must be distinguished from
  transient mutations, deletion-driven marker turnover, threshold-created labels, and
  misleading control comparisons.
- A missing or inconclusive structural comparison blocks acceptance.

Spec §7.8 states what that comparison can show. If evolved structure is useful
because it fits its lineage's weights, the evolving world should outperform the
structural null. If structural change is neutral drift, the two should match.

## Provenance

| Item | Value |
|---|---|
| Source revision | `dd35687` (M9's calibrated plant ecology shipped) for the acceptance and threshold runs; `e59a6f9` (the retune tooling, merged as #82) for the knockout; `8e6bdbc` for the watched world |
| Seeds | 7, 42, 117, 314, 2026 |
| Founders / ticks / sample interval | 2,000 / 1,000,000 / 10,000 (knockout: 400,000 / 5,000) |
| Controls | scalar `randomized_at_birth_v3` and structural null `structural_null_v2`, each a separate world beside the same evolving world (spec §7.8) |
| Configurations | shipped defaults; plant input halved to 12,000/s (`input-12k`); species thresholds 0.3 and 0.8; a food-scent knockout |
| Configs and summaries | [`experiments/phase-2-acceptance/`](../experiments/phase-2-acceptance/) |

The commands, from a checkout at the stated revision:

```bash
JOBS=8 scripts/experiment.sh OUT 1000000 2000 10000 "42 117 314 7 2026" \
  experiments/phase-2-acceptance/shipped.json \
  experiments/phase-2-acceptance/input-12k.json
# thresholds: evolving world and structural null only
native --seed SEED --ticks 1000000 --founders 2000 --sample-every 10000 \
  --params experiments/phase-2-acceptance/threshold-0.3.json \
  --control structural-null --metrics OUT/threshold-0.3-SEED.jsonl
# knockout (revision e59a6f9): two runs per seed, identical until tick 300,000
native --seed SEED --ticks 400000 --founders 2000 --sample-every 5000 \
  --control structural-null --metrics OUT/SEED-intact.jsonl
native --seed SEED --ticks 400000 --founders 2000 --sample-every 5000 \
  --control structural-null --metrics OUT/SEED-knockout.jsonl \
  --retune experiments/phase-2-acceptance/knockout.json --retune-at 300000
```

For each seed, the evolving world's final state hash matched between its scalar run
and its null run, so each seed's three cohorts are a matched comparison. Wall-clock
time was 939–2,940 seconds per 1M-tick run with eight running at once.

## Mechanical checks

The M10 checks are the CI suite (`.github/workflows/ci.yml`), which passes on `main`.
They exercise structural births and deaths, not founder-only scenarios:

- **Energy conservation.** `a_population_that_eats_and_breeds_still_conserves` runs
  10,000 ticks on shipped defaults, with structural mutation and plant ecology on,
  asserting relative drift below 1e-4 at every tick.
- **No hot-loop allocation.** Neural structural births, sensor edits,
  structural-null donor births, plant turnover, and corpse churn each have a test
  (`sim-core/tests/no_alloc.rs`).
- **Deterministic hashing.** Golden references cover structural mutation, organ
  mutation, variable storage, and the structural null in both heredity modes
  (`sim-core/tests/golden.rs`).
- **Native/WASM agreement.** The same cases reach the same hashes under
  `wasm-pack test` (`shells/wasm/tests/cross_target.rs`), and checkpoints transfer
  between the two in every mode.

## Brains grow in complexity

Final values at 1,000,000 ticks, per seed in the order 7, 42, 117, 314, 2026. The
founder has 25 genes, 4 enabled connections, 0 hidden neurons, and 1 wired sensor.

| Shipped input (24,000/s) | evolving | structural null | scalar control |
|---|---|---|---|
| population | 993, 935, 858, 997, 975 | 489, 575, 419, 423, 617 | 84, 134, 45, 127, 144 |
| supply captured | 0.24, 0.25, 0.22, 0.23, 0.24 | 0.17, 0.21, 0.17, 0.14, 0.19 | 0.03–0.09 |
| enabled connections | 7.4, 7.5, 9.7, 7.0, 12.9 | 4.5, 5.5, 6.0, 6.2, 7.2 | 3.9–4.6 |
| genome genes | 31.6, 32.4, 33.4, 29.6, 43.8 | 26.4, 28.2, 29.4, 30.0, 30.2 | 25.2–26.5 |
| largest genome | 45, 47, 50, 47, 60 | 34, 35, 36, 35, 37 | 28–32 |
| wired hidden neurons | 1.11, 0.67, 0.97, 0.58, 2.26 | 0.07, 0.78, 0.97, 0.76, 1.09 | 0.03–0.36 |
| wired sensors | 0.00, 0.67, 0.24, 0.01, 0.16 | 1.00, 0.99, 0.99, 1.00, 0.20 | 0.99–1.00 |

| Half input (12,000/s) | evolving | structural null | scalar control |
|---|---|---|---|
| population | 498, 429, 430, 441, 460 | 245, 112, 253, 64, 173 | 2–55 |
| supply captured | 0.23, 0.24, 0.22, 0.23, 0.24 | 0.16, 0.10, 0.17, 0.04, 0.13 | 0.00–0.07 |
| enabled connections | 9.5, 10.6, 11.7, 8.1, 8.9 | 4.1, 3.5, 6.8, 6.3, 5.9 | 3.9–4.5 |
| genome genes | 35.5, 36.2, 38.3, 33.6, 36.6 | 26.0, 25.3, 30.4, 29.7, 29.6 | 24.9–26.2 |
| wired hidden neurons | 1.30, 1.71, 1.44, 0.57, 2.45 | 0.09, 0.03, 1.40, 0.94, 0.78 | 0.00–0.33 |
| wired sensors | 0.30, 0.43, 0.05, 0.00, 0.17 | 1.00, 1.00, 0.49, 1.00, 0.98 | 0.98–1.00 |

**The structural comparison is conclusive.** On all ten seed-and-input pairs, the
evolving world outnumbers the structural null and captures more of the food supply.
At shipped input it holds 1.6–2.4 times the null's population; at half input,
1.7–6.9 times. That is the outcome spec §7.8 names for structure that is useful
because it fits its lineage's weights. The scalar control, which keeps topology but
redraws its weights, falls to 2–144 agents.

**Much of the gap is breeding, not finding food.** The controls' agents are rich.
Mean agent energy at 1M is 16,000–35,000 in the null and 63,000–170,000 in the
scalar control, against 157–1,039 in evolving worlds; a newborn starts with 100 and
may breed above 200. Control agents graze, but their reproduction drive seldom
crosses the gate, so food piles up in tanks rather than offspring, and plants stay
fuller (stock 0.69–0.77 of capacity in the null against 0.51–0.59). Evolving
lineages breed as soon as they can. The null keeps every lineage's weights on the
founder genes everyone shares, so what it disrupts is the fit between a lineage's
own added structure and those weights. Its deficit is therefore evidence that the
added structure matters, though not that it matters for foraging.

**Complexity keeps growing, and only under selection.** Window means over the 50,000
ticks before each mark (enabled connections; evolving / structural null / scalar):

| Seed (shipped) | 100k | 250k | 500k | 750k | 1M |
|---|---|---|---|---|---|
| 7 | 4.3 / 4.1 / 4.1 | 4.5 / 4.5 / 4.1 | 4.6 / 4.5 / 4.2 | 5.7 / 4.5 / 4.2 | 7.1 / 4.5 / 4.2 |
| 42 | 5.1 / 4.2 / 4.1 | 5.1 / 4.2 / 4.1 | 6.1 / 4.4 / 4.0 | 6.8 / 5.5 / 3.9 | 7.4 / 5.5 / 3.9 |
| 117 | 4.4 / 5.8 / 4.2 | 5.3 / 6.0 / 4.7 | 6.3 / 6.0 / 4.5 | 8.0 / 6.0 / 4.6 | 9.9 / 6.0 / 4.6 |
| 314 | 4.8 / 5.5 / 4.3 | 6.2 / 6.2 / 4.4 | 7.0 / 6.1 / 4.3 | 6.4 / 6.1 / 4.3 | 6.9 / 6.2 / 4.3 |
| 2026 | 5.2 / 5.4 / 4.2 | 7.9 / 6.9 / 4.3 | 10.2 / 7.1 / 4.3 | 11.0 / 7.2 / 4.4 | 12.7 / 7.2 / 4.5 |

Genome size follows the same pattern. Evolving worlds rise from 25.9–27.3 genes at
100k to 29.6–43.5 at 1M. The scalar control holds 25.1–26.8 from 250k onward, and
the null holds steady from 250k on four of five seeds (seed 42 steps up once,
between 500k and 750k). At half input, genes rise through the run on every seed,
and connections through 750k. The null's structure largely stops changing once its
population settles, because each child copies the topology of a random survivor.
Evolving lineages keep adding structure that their own weights fit.

**What grew is motor circuitry, not perception.** Wired sensors fall over the run in
evolving worlds, from about 1.0 at 100k (0.64 on seed 314) to 0.00–0.67 at 1M. Both
controls keep theirs, except the null on seed 2026, which drops to 0.20 by 250k. With
few or no sensors wired, what drives the effectors is the founders' oscillators. The
added connections and hidden neurons therefore most likely shape those rhythms into
movement, rather than process sensory input. The knockout below tests the perception
half of that directly.

### Do evolved brains use their senses?

Food scent is the shipped founders' only sense of food; eyes appear only by mutation.
At tick 300,000 the knockout run sets `plants.scent_rate` to 0, so no new scent
enters the field. The intact run of the same seed continues unchanged. The two are
identical until the retune. Supply captured over ticks 350,000–400,000:

| Seed | evolving intact / knockout | null intact / knockout | evolving wired sensors |
|---|---|---|---|
| 42 | 0.244 / 0.247 | 0.240 / 0.242 | 0.95 |
| 117 | 0.223 / 0.235 | 0.172 / 0.145 | 1.00 |
| 314 | 0.235 / 0.240 | 0.155 / 0.136 | 0.26 |
| 7 | 0.242 / 0.238 | 0.171 / 0.148 | 0.08 |
| 2026 | 0.243 / 0.250 | 0.214 / 0.211 | 0.48 |

Losing scent costs the evolving worlds nothing. Capture moves by at most 0.012, and
upward on four seeds, even where every agent still has a sensor wired (42, 117). The
structural null loses 12–16% of its capture on the three seeds where it relies on
scent (117, 314, 7). Smell is useful in this world, since random-structure brains do
measurably worse without it, but evolved lineages forage better without using it.

### Harshness

Halving plant input halves the evolving population (858–997 to 429–498), while the
share of supply captured holds at 0.22–0.25. The added structure is a little larger
when food is scarce, on four of five seeds each for connections, genes, and wired
hidden neurons. The evolving world's lead over the null widens (1.7–6.9 times its
population, against 1.6–2.4). This is a correlation across two settings, not a
controlled test of competition: a smaller population also changes drift. Seed 314
gains the least structure at both inputs.

## Distinct species appear

Species at the provisional 0.5 threshold, final sample (seeds 7, 42, 117, 314, 2026).
Persistent means alive at the end and first sampled at least half the run earlier.

| Shipped input | evolving | structural null | scalar control |
|---|---|---|---|
| active species | 49, 53, 48, 40, 36 | 21, 29, 16, 24, 20 | 8–16 |
| persistent species | 9, 21, 24, 5, 15 | 21, 26, 16, 23, 20 | 7–14 |
| species founded over the run | 382, 209, 228, 304, 205 | 89, 136, 58, 96, 109 | 61–335 |
| species extinct over the run | 333, 156, 180, 264, 169 | 68, 107, 42, 72, 89 | 51–327 |

Evolving worlds hold 1.7–3 times as many species at any moment as the null. They
found and lose species 1.5–4.3 times as often, and still keep 5–24 lineages alive
for more than half the run. The null's species are mostly its founding
clusters, frozen in place: children copy random survivors' topology, so its genomes
stop diverging. A higher persistent count in the null therefore means stasis, not
richer speciation.

**Species at three thresholds.** The threshold changes only the labels: evolving and
null populations were identical at 0.3, 0.5, and 0.8. Shipped input, seeds 7, 42, 117,
314, 2026:

| Threshold | active, evolving | active, null | persistent, evolving | persistent, null |
|---|---|---|---|---|
| 0.3 | 84, 95, 101, 78, 71 | 59, 72, 33, 47, 63 | 7, 29, 40, 7, 15 | 59, 58, 33, 44, 60 |
| 0.5 | 49, 53, 48, 40, 36 | 21, 29, 16, 24, 20 | 9, 21, 24, 5, 15 | 21, 26, 16, 23, 20 |
| 0.8 | 24, 18, 19, 17, 18 | 6, 7, 5, 7, 5 | 9, 10, 9, 7, 8 | 6, 7, 5, 7, 5 |

- **More species at any moment.** At every threshold, evolving worlds hold more active
  species than the null on every seed.
- **The null's extra persistence is fine-threshold splitting.** Its higher count of
  persistent species appears only at 0.3 and 0.5, where small differences among its
  frozen founding clusters earn separate labels.
- **At the coarse threshold the evolving world keeps more.** At 0.8, where a label
  takes substantial genetic divergence, evolving worlds keep more long-lived species on
  four seeds and tie on the fifth (314).

## Watched

The agent loaded seed 42's evolving world at tick 1,000,000 into the browser. That is
a native `--save-run` bundle at revision `8e6bdbc`. It held 935 agents, 53 species,
and a mean energy of 1,039, all matching the native run.

The agent watched it coloured by species, at 1× zoomed into one patch and at 100×
across the whole world:

- **Life follows the soil.** Agents live only on the four fertile patches; the ground
  between them holds neither plants nor agents.
- **Species hold territories.** One species dominates the north-central patch, others
  the southern patch and the western edge. Offspring spawn near parents, so lineage
  maps onto space.
- **Grazing is local and steady.** Plants under each cluster are grazed dark, while
  full plants survive in the gaps between clusters.
- **Agents do not steer to food.** They drift slowly, at about 2 units per second, and
  do not turn toward the fuller plants nearby. This matches the knockout: evolved
  lineages forage without smell.
- **The population turns over steadily.** It moved between 923 and 947 over a few
  seconds, with no idle agents and no collapse.

Not watched: a structural null at a million ticks. The browser can only watch a null
from tick 0, and saved runs carry the scalar control, so the null comparison rests on
the metrics above.

## Decision

**Accepted.** Phase 2's criterion is met, with the limits below stated rather than
waived.

- **Brains grow in complexity, and the growth is selected.** Evolving worlds add
  connections, neurons, and genes beyond both controls, and keep adding them through
  a million ticks while both controls level off. The structural comparison the plan
  requires is not inconclusive. On every seed at both inputs, the evolving world
  beats the null that breaks the fit between a lineage's structure and its weights,
  which spec §7.8 names as the sign that structure is useful.
- **Distinct species appear.** At every threshold tested, evolving worlds hold more
  species than the null at any moment, and they keep founding and losing them. At the
  coarse threshold they also carry more long-lived species. Threshold-created labels
  would not produce that pattern against a static null, and the threshold dependence
  is reported above rather than assumed away.

What the agent would want a human reviewer to weigh, and what would overturn this:

- **The complexity is motor, not sensory.** Evolved brains shed perception and
  forage without smell. If "brains grow in complexity" should mean brains that
  process more of the world, this phase has not shown it; the evidence shows richer
  internal dynamics that outbreed the controls. The agent reads the criterion as
  written, but this is the strongest case against acceptance.
- **The advantage is largely in breeding.** Control agents hoard energy rather than
  reproduce, so the population gap measures turning food into offspring at least as
  much as finding it.
- **Persistence at the default threshold favours the null.** At 0.5 the null's frozen
  founding clusters keep more labels alive for half a run. A reviewer who weights
  persistence at the default threshold over turnover and coarse-threshold persistence
  would read the species evidence as weaker than the agent does.

Carried into Phase 3: whether perception pays when there are predators to avoid and
prey to find. The knockout tooling is ready to rerun once bites are on.

## Limitations

- Five seeds, two plant inputs, one founder. Every claim above is a per-seed vector
  with both controls alongside, not a ranking.
- Final samples for the main tables. Trajectories use 50,000-tick window means from
  10,000-tick samples. Per-run JSONL is not committed; the commands above regenerate
  it at the stated revisions.
- `supply_captured` grows with population, which is itself an outcome of foraging.
  The knockout compares a world with itself, so it does not share that confound.
- The knockout silences scent only. A retune cannot blind an eye whose range lives in
  its gene, and the few evolved eyes were not tested.
- Species persistence uses 10,000-tick sampling, so short-lived species appear only
  in the founded and extinct totals.
- The harshness comparison has two input levels and does not separate competition
  from population size.
