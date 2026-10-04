# Phase 2 — Founder, structural-evidence, and plant-ecology experiments

Multi-seed evidence for Phase 2's success criterion (spec §8: *brains grow in
complexity and distinct species appear*). This report records what headless runs
measured. It is not a judgment: whether anything interesting evolved is for a human
watching the sim to decide (spec §7.8 tier 3). Nothing here is ranked.

**Headline (M9).** With M9's plant ecology shipped — regrowth that depends on what a
plant has left, plants that cluster on fertile ground, and starved plants that die and
reseed near their parents — **the structural null is distinguished for the first
time**: on every seed, evolving worlds sustain more agents than worlds whose children
take a random survivor's structure (824–964 against 424–792), and capture as much of
the food supply or more. Added structure exceeds the null's on two of three seeds.
Perception is kept on two seeds and still shed on the third. See
[Plant ecology calibration (M9)](#plant-ecology-calibration-m9); everything above it
measured the Phase 1 plants.

**Headline (M8).** Every evolving world was viable, and the minimal chemo-led founder was
the most viable configuration, so it now ships. Brains grow, and on the shipped
configuration that growth is still **not distinguished from the structural null**
(spec §7.8): lineages that inherit their own structure grow no more than lineages
given a random survivor's. The clearest consistent signal points elsewhere:
**evolving lineages shed perception**. They keep fewer sensors than the null on
every seed, and late species increasingly have no sensor wired to any effector,
running clock-driven, open-loop behavior instead. A follow-up sweep of food
patchiness and scarcity found **no plant setting where perception pays**: evolving
worlds wired fewer sensors than the null in 10 of 12 seed pairs. Selection is
visible, but in movement rather than structure: evolving worlds converge across
seeds on how fast they move and how much of the food supply they capture. Species
counts are set mostly by the threshold; deletion's marker churn is a minor effect.

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

## Perception sweep

Does a different food environment make perception pay? Plant parameters only
(tuning, no mechanism change), everything else at shipped defaults. Seeds
42/117/314, 2,000 founders, 300,000 ticks, both controls. Runs used revision
`ac36fea`; summaries were produced by `48c79a7`'s `native summarize`.
Summaries, configs, and wall-clock times:
[`experiments/perception-sweep/`](../experiments/perception-sweep/).

- **baseline**: shipped plants, 4,000 sites × 60 energy, input 12,000/s.
- **sites-1000**: the same total capacity on 1,000 sites of 240, so food is 4×
  patchier.
- **sites-250**: 250 sites of 960, 16× patchier.
- **scarce**: baseline sites with input 8,000/s, two-thirds of the supply.

`supply_captured` is the share of the plants' nominal input that the population
eats over the run's second half. A full plant refuses its share, so a population
that keeps more sites grazed captures more. The wired counts are per agent at the
final sample. For reference, the founder has 1 wired sensor, 4 driven effectors, 0
hidden neurons, and a sensor load of 3.

| Per seed (42, 117, 314) | baseline | sites-1000 | sites-250 | scarce |
|---|---|---|---|---|
| population, evolving | 1439, 1716, 1612 | 995, 1239, 1116 | 172, 446, 296 | 816, 863, 826 |
| population, null | 1462, 1432, 1934 | 1245, 1370, 1092 | 307, 109, 414 | 559, 697, 1009 |
| population, scalar control | 19–32 | 22–196 | 9–148 | 10–13 |
| supply captured, evolving | 0.69, 0.69, 0.69 | 0.59, 0.58, 0.58 | 0.07, 0.23, 0.27 | 0.63, 0.62, 0.64 |
| supply captured, null | 0.63, 0.64, 0.70 | 0.65, 0.77, 0.48 | 0.09, 0.08, 0.28 | 0.46, 0.56, 0.61 |
| supply captured, scalar control | 0.02–0.03 | 0.02–0.18 | 0.02–0.24 | 0.01–0.02 |
| sensor load, evolving | 2.05, 0.20, 1.05 | 2.87, 2.82, 2.90 | 2.13, 3.04, 2.99 | 2.51, 1.72, 2.79 |
| sensor load, null | 2.54, 1.55, 3.03 | 3.00, 2.77, 2.91 | 2.98, 3.88, 2.56 | 2.99, 3.01, 2.59 |
| **wired sensors, evolving** | **0.68, 0.06, 0.35** | **0.89, 0.79, 0.36** | **0.71, 0.89, 1.00** | **0.80, 0.37, 0.85** |
| **wired sensors, null** | **0.83, 0.50, 1.00** | **0.97, 0.78, 0.89** | **0.99, 0.99, 0.84** | **1.00, 1.00, 0.87** |
| driven effectors, evolving | 2.92, 3.24, 2.60 | 3.47, 2.83, 2.83 | 2.70, 2.40, 3.77 | 2.14, 3.52, 2.91 |
| driven effectors, null | 3.41, 3.39, 3.84 | 3.57, 2.54, 3.20 | 2.02, 2.05, 3.10 | 3.83, 3.86, 3.45 |
| wired hidden neurons, evolving | 2.33, 0.56, 0.19 | 0.35, 0.77, 1.11 | 0.81, 1.17, 0.24 | 0.25, 0.80, 0.63 |
| wired hidden neurons, null | 1.10, 0.99, 1.96 | 0.34, 1.04, 1.92 | 1.94, 0.36, 3.06 | 0.83, 0.68, 0.18 |
| mean speed, evolving | 2.09, 1.43, 1.68 | 0.66, 0.63, 0.63 | 0.52, 0.49, 0.54 | 2.34, 2.27, 2.38 |
| mean speed, null | 1.43, 1.74, 0.71 | 0.43, 0.35, 0.18 | 0.29, 0.40, 0.34 | 1.88, 2.23, 1.37 |

- **No plant setting made perception pay.** Evolving worlds wired fewer sensors than
  the null in 10 of 12 seed pairs, at least two in every configuration. The other two
  are a tie (sites-1000 seed 117) and one exception (sites-250 seed 314). Patchier
  food kept sensor *genes* (sensor load about 2.85 at sites-1000 against 0.20–2.05 at
  baseline) but did not keep them wired. The null kept as many sensors and wired more
  of them.
- **Perception does not earn its upkeep.** A chemo sensor adds 0.0019 per tick to an
  idle founder's 0.064 (about 3%), and lineages that drop or disconnect it do not
  lose. Movement is far dearer (`k_move` × thrust², up to 0.5 per tick), yet evolving
  agents moved faster than the null in 11 of 12 pairs.
- **Selection is visible, in movement rather than structure.** Evolving worlds
  converge across seeds. Supply captured spans at most 0.02 between seeds in
  baseline, sites-1000, and scarce, against 0.08–0.28 for the null, and speed spans
  at most 0.11 outside baseline. Under scarcity, evolving worlds captured more of the
  supply than the null on every seed (0.62–0.64 vs 0.46–0.61) while wiring fewer
  sensors: the advantage comes from how they move, not from what they sense.
- **Added structure is still not distinguished from the null.** Wired hidden
  neurons go each way across seeds in every configuration.
- **sites-250 is too sparse to read.** Populations shrank to 109–446 and one
  scalar-control world held 148 agents, so seed-to-seed variation swamps the
  comparison.
- Species-founding genomes agree. Running [`wiring.py`](../experiments/phase-2-m8/wiring.py)
  on history archives of the same worlds (revision `e515d3f`, whose 24 final state
  hashes match `ac36fea`'s) found last-quarter founders had 0.07–0.59 wired sensors
  at baseline and 0.47–0.93 at sites-1000. Output is in
  [`experiments/perception-sweep/wiring/`](../experiments/perception-sweep/wiring/).

Untested: how informative the chemo signal is (`chemo_radius`, `scent_rate`), and
sensor cost (`k_sensor`). The mechanism that makes clock-driven grazing competitive is
that plants are fixed sites regrowing in place (spec §5.1 as read in
`sim-core/src/plants.rs`), dense enough that covering ground at a tuned speed finds
food. Changing that, for example by relocating depleted sites or adding spec §5.3's
nutrient heterogeneity, is a design change for discussion, not tuning.

## Plant ecology calibration (M9)

Spec §5.1 and §5.3's plant ecology, calibrated before shipping. Seeds 42/117/314,
2,000 founders, 300,000 ticks, both controls, revision `257114b` (the mechanisms in
place, defaults still off). Every configuration file states all eight plant-ecology
fields explicitly, so it reproduces on later revisions whatever the defaults are.
Configs, summaries, and wall-clock times:
[`experiments/plant-ecology/`](../experiments/plant-ecology/).

| Config | Plants |
|---|---|
| baseline | Phase 1's plants: input 12,000/s, everything else off |
| patchy-only | patchiness 4, patch scale 150 |
| lag-only | grazing lag 0.8 |
| turnover-only | death below 0.1 of capacity for 30 s, dispersal 0.9 within 40 |
| eco | all three together, input 12,000/s |
| eco-input-18k | eco at 18,000/s |
| **eco-input-24k** | **eco at 24,000/s — now the shipped defaults** |
| eco-strong | grazing lag 0.9, patchiness 8, death after 15 s |
| eco-sparse | eco on 1,000 sites of 240 |

The shipped values were chosen by rules fixed before the input-scaling round was
read: viable on every seed; mechanisms visibly operating (clustering below 0.85,
sustained reseeding, unsaturated stock); ecology values from spec §5.5's
relationships; and the lowest tested input that keeps the evolving population near
half the baseline. Wired sensors and the evolving-versus-null gaps were not criteria:
they are what this section reports.

| Per seed (42, 117, 314) | evolving population | null population | scalar control | evolving supply captured | null supply captured | plant clustering | plants reseeded (evolving) |
|---|---|---|---|---|---|---|---|
| baseline | 1439, 1716, 1612 | 1462, 1432, 1934 | 19–32 | 0.69, 0.69, 0.69 | 0.63, 0.64, 0.70 | 1.00–1.01 | 0 |
| patchy-only | 1890, 1869, 1801 | 1899, 1678, 2152 | 123–537 | 0.85, 0.78, 0.79 | 0.84, 0.76, 0.80 | 0.78–0.86 | 0 |
| lag-only | 359, 338, 315 | 107, 175, 217 | 2–8 | 0.21, 0.19, 0.17 | 0.11, 0.15, 0.18 | 1.00–1.01 | 0 |
| turnover-only | 1483, 1415, 1408 | 982, 960, 1394 | 12–16 | 0.68, 0.66, 0.70 | 0.52, 0.59, 0.63 | 0.87–0.94 | 1051–3721 |
| eco | 468, 416, 431 | 175, 362, 68 | 2–54 | 0.25, 0.22, 0.18 | 0.16, 0.20, 0.05 | 0.70–0.76 | 4682–9211 |
| eco-input-18k | 676, 669, 736 | 441, 536, 408 | 5–99 | 0.25, 0.23, 0.24 | 0.23, 0.19, 0.18 | 0.68–0.76 | 3151–6431 |
| **eco-input-24k** | **964, 824, 936** | **792, 424, 478** | 50–152 | **0.25, 0.23, 0.24** | **0.23, 0.17, 0.18** | 0.68–0.76 | 2949–7575 |
| eco-strong | 0, 0, 0 | 0, 0, 0 | 0–1 | 0 | 0 | 0.42–0.53 (empty worlds) | 1703–10217 |
| eco-sparse | 493, 485, 3 | 45, 43, 24 | 4–11 | 0.18, 0.26, 0.00 | 0.05, 0.07, 0.02 | 0.72–0.80 | 1118–6699 |

Structure and perception, evolving against the structural null (founder: 1 wired
sensor, 0 hidden neurons, 25 genes):

| Per seed (42, 117, 314) | wired sensors, evolving | wired sensors, null | wired hidden, evolving | wired hidden, null | genes, evolving | genes, null | persistent species, evolving / null |
|---|---|---|---|---|---|---|---|
| baseline | 0.68, 0.06, 0.35 | 0.83, 0.50, 1.00 | 2.33, 0.56, 0.19 | 1.10, 0.99, 1.96 | 35.5, 26.4, 26.7 | 31.5, 28.9, 34.5 | 8–27 / 19–36 |
| lag-only | 0.99, 0.01, 0.99 | 0.91, 0.99, 1.00 | 1.29, 0.84, 1.86 | 0.10, 0.82, 0.11 | 30.5, 27.8, 32.4 | 25.5, 27.8, 26.6 | 7–8 / 5–10 |
| turnover-only | 0.20, 0.13, 0.77 | 0.85, 0.98, 0.02 | 1.94, 0.86, 0.53 | 0.51, 0.18, 0.22 | 34.7, 27.2, 29.1 | 27.4, 26.7, 26.3 | 11–30 / 20–41 |
| **eco-input-24k** | **0.93, 0.99, 0.34** | **0.98, 0.99, 1.00** | **0.81, 0.47, 1.44** | **0.41, 0.97, 0.78** | **29.3, 28.1, 31.6** | **26.5, 29.4, 30.0** | 6–10 / 17–28 |

- **Inherited structure now pays ecologically.** With grazing lag or turnover on,
  evolving worlds outnumber the structural null on every seed of every viable
  configuration (eco-sparse seed 314, where both collapsed, is the exception), and
  capture as much of the supply or more on every other seed pair but one (lag-only
  seed 314, 0.17 against 0.18). Under Phase 1's plants and under patchiness alone the null
  matched them. A lineage's own wiring now matters for how many of it the world can
  feed.
- **Added structure exceeds the null more often.** In the shipped configuration,
  evolving worlds have more wired hidden neurons and larger genomes than the null on
  two of three seeds; turnover alone gives more of both on all three. Under Phase 1's
  plants, neither was distinguished.
- **Perception is no longer reliably shed, nor reliably kept.** Shipped evolving
  worlds keep sensors wired on seeds 42 and 117 (0.93, 0.99) and shed them on 314
  (0.34); at baseline they shed on all three. The null keeps its sensors throughout,
  so wired sensors alone still do not separate useful perception from inherited
  scenery.
- **Patchiness alone makes foraging easier, not harder.** Clustered food let random
  brains survive in the hundreds (123–537) and left evolving and null equal. It ships
  only together with grazing lag and turnover, which make a patch something to find
  and leave.
- **Limits.** Grazing lag 0.9 with patchiness 8 and 15-second starvation killed
  every world, controls included; fewer, larger sites with the ecology on nearly
  killed seed 314. The shipped values sit inside that boundary.
- **Species.** Evolving worlds hold fewer persistent species than the null under the
  ecology (6–10 against 17–28 shipped); at the 0.5 threshold, species counts still
  do not favour the evolving world.

## Unranked shortlist for watching

In the browser against the scalar control, at shipped defaults unless a params
override is given. The M9 entries come first; the rest describe Phase 1's plants and
need the baseline config (`experiments/plant-ecology/baseline.json`) to reproduce:

- **M9 seed 42**: sensors kept wired (0.93) and the largest population (964). Do
  agents move between patches, turn toward fat plants, and leave stripped ones?
- **M9 seed 117**: the widest gap over the null (824 against 424). What does the
  evolving world do that a world of reshuffled brains cannot?
- **M9 seed 314**: sensors shed (0.34 wired) yet more agents than the null. What
  replaces perception here?
- **Patch drift, any seed**: do patches visibly creep as overgrazed plants die and
  survivors reseed nearby?

- **seed 42** — the largest genomes in the shipped rerun (max 51 genes). Are the added
  neurons and clocks visible in behavior?
- **seed 117** — nearly sensorless (mean sensor load 0.17) yet the largest population.
  Do agents forage, or graze on a timer?
- **seed 314** — the most species. Are they behaviorally distinct or threshold labels?
- **scarce, any seed** (`{"plants":{"energy_input_rate":8000.0}}`): evolving worlds
  out-captured the null on every seed with fewer wired sensors. Is the difference
  visible as movement, such as speed, turning, and dispersal?
- **sites-1000 seed 42** (`{"plants":{"max_plants":1000,"max_energy":240.0}}`):
  sensors kept and mostly wired (0.89 per agent). Does anything look like
  chemotaxis, or do agents pass plants they could have turned to?

## Limitations

- Three seeds per configuration; final samples only. Trajectories are reproducible
  from the command above but are not committed (`*.jsonl` is ignored).
- The M8 functional-wiring table covers one seed and species-founding genomes; the
  perception sweep's population-wide `wired_*` metrics cover three seeds, at the final
  sample only.
- `supply_captured` is a population measure: it rises with per-agent foraging and with
  population size, which is itself an outcome of foraging. Compare cohorts at similar
  populations.
- The M9 calibration ran 300,000 ticks, final samples only; the shipped configuration
  has not yet been run longer. Its random-brain control survives at 50–152 agents,
  more than under Phase 1's plants, because clustered food is easier to stumble on.
- Species persistence uses 5k-tick sampling, so species living between samples are
  counted only in created/extinct totals.
