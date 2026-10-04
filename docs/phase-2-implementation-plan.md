# Phase 2 - Implementation Plan

Phase 2 builds the **genetic architecture** from
[`synthetic-evolution-spec.md`](synthetic-evolution-spec.md) section 8. Design
decisions live in the spec; this plan tracks status and the remaining work. Phase 1's
acceptance evidence is in [`phase-1-implementation-plan.md`](phase-1-implementation-plan.md).

**Status: complete.** Phase 2 was accepted on 2026-10-04, against its success
criterion: **brains grow in complexity and distinct species appear**, judged with
reproducible multi-seed evidence and both controls alongside (the scalar control and
the structural null, spec §7.8). The project owner delegated M10's watching and
final call to the coding agent, so that judgment is the agent's. The record, its
reasoning, and what would overturn it are in [M10](#m10---acceptance) below. Work
continues in [`phase-3-implementation-plan.md`](phase-3-implementation-plan.md).

| Milestone | State |
|---|---|
| M0 Design decisions | Approved and recorded in the spec; distance/threshold calibration open (below) |
| M1 Variable-length storage | Done: pooled arenas, transactional births, allocator-state hashing |
| M2 Neural structural mutation | Done: add/remove connection and neuron, toggle, oscillator addition; rates ship enabled |
| M3 Sensors and founders | Done: add/remove sensor, configurable founders; the minimal chemo-led founder ships |
| M4 Distance and species | Done: deterministic classification with stable IDs; provisional threshold 0.5 |
| M5 Phylogeny | Done: lifetime identities, bounded native/browser history archives |
| M6 Observation and sharing | Done: schema 8 telemetry, species browser, origin graph, representative comparison |
| M7 Manual checkpoints | Done: saved-run bundles with exact continuation in both shells |
| M8 Founder experiments | Evidence reported, including the shipped-configuration rerun, species calibration, and a perception sweep; its open questions answered by M10's evidence |
| M9 Plant ecology | Done: stock-dependent regrowth, patchy fertility, turnover; calibrated and shipped |
| M10 Acceptance | Accepted 2026-10-04 by the agent under the owner's delegation; see [M10](#m10---acceptance) |

## Delivered

- **M1** — Genomes and compiled brains live in bounded, pooled variable-length arenas
  (spec §2.2a). A birth claims every arena before an identity, so a refused birth
  leaves the parent and allocation order untouched. Budgets are runtime `SimParams`.
- **M2** — Five neural operators plus M8's oscillator addition run once per birth in a
  fixed order; zero rates draw nothing (spec §3.3). The scalar control applies the same
  structural edits, then redraws neural scalars on the resulting topology (§7.8).
- **M3** — Sensor addition/removal over the existing modalities, and founder
  composition as construction-time params. The shipped founder is one chemoreceptor,
  two oscillators, and four effectors with one input each (spec §3.1).
- **M4** — Genetic distance over typed genes and a classifier with immutable
  representatives, stable never-reused IDs, and explicit unclassified overflow
  (spec §3.4).
- **M5** — Persistent birth identities and parent links, bounded species-history
  capture, and native and browser archives (history schemas 1–4; spec §3.4, §7.10).
- **M6** — Complexity and history telemetry (metrics schema 8), the live species
  browser and coloring, representative archives, the layered species-origin graph,
  and aligned representative comparison.
- **M7** — Saved-run bundles: core checkpoints plus available history, exact
  continuation across native and WASM, `resume` with resumed history segments, and
  browser Save run / Load run (spec §7.10). Incompatible formats are refused, not
  migrated.
- **M8** — Multi-seed tooling (`scripts/experiment.sh`, `native summarize`), the
  structural null, founder-viability and structural-null evidence, the minimal
  founder, oscillator addition, and structural/sensor mutation shipped enabled.
  After M8: population-wide functional-wiring telemetry (`wired_hidden_neurons`,
  `wired_sensors`, `driven_effectors`), the `supply_captured` foraging measure in
  `native summarize`, and the perception sweep.
- **M9** — Plant ecology (spec §5.1, §5.3): stock-dependent regrowth, a fertility
  map for patchy placement, and starvation turnover with local dispersal; plant
  clustering and reseed telemetry; calibrated defaults.

## Outcome

All of Phase 2's evidence is in [`phase-2-m8-evidence.md`](phase-2-m8-evidence.md),
M10's under [Acceptance runs (M10)](phase-2-m8-evidence.md#acceptance-runs-m10). Over 1,000,000 ticks on five
seeds, evolving worlds outnumber the structural null and capture more food on every
seed, at shipped and at half plant input. Their brains keep adding connections and
genes while both controls level off. What grew is motor circuitry, most likely
shaping the founders' oscillator rhythms: a food-scent knockout costs evolved worlds
nothing, while the null loses up to 16% of its capture. At every threshold tested,
evolving worlds hold more species at any moment, with more turnover; at the coarse
threshold they also keep more long-lived species.

Carried forward rather than resolved:

1. **Perception is unused.** Evolved lineages shed their sensors and forage without
   them. Phase 3's predators and prey are the next test of whether sensing pays;
   rerun the knockout once bites are on.
2. **Species thresholds stay provisional.** Report persistent species at more than
   one threshold whenever species are a claim.

## M9 - Plant ecology (done)

Spec §5.1 and §5.3 record the design: regrowth that depends on what is left, plants
that die and reseed near parents, and patchy fertility. Seasons, disturbance, and
terrain stay in Phase 6. Each mechanism landed disabled, with a zero value that
reproduces Phase 1's plants exactly, and calibration enabled them together; the
calibration is in the evidence report.

1. **Stock-dependent regrowth** (`grazing_lag`).
2. **Fertility map and patchy placement** (`patchiness`, `patch_scale`), charged to the
   construction budget.
3. **Plant turnover** (`death_stock`, `death_seconds`, `local_dispersal`,
   `dispersal_radius`). Starvation timers join the state hash, which already covers
   positions; positions and timers join the checkpoint (a new checkpoint format).
   History and metrics readers treat the new params as absent-means-zero, which is
   how older runs ran.
4. **Calibration.** Multi-seed sweeps for viability and for the perception question,
   against both controls, reporting wired sensors, `supply_captured`, and plant
   clustering. Defaults ship with their reasons on the fields (a golden-hash change);
   the calibration's shipped-configuration runs are the post-M9 evidence.

## M10 - Acceptance

Run the checks for every changed surface (`AGENTS.md`,
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)). Structural birth/death runs
must cover energy conservation, no hot-loop allocation, deterministic hashing, and
native/WASM agreement; founder-only scenarios are insufficient.

Human review must distinguish useful inherited complexity and persistent species from
transient mutations, deletion-driven marker turnover, threshold-created labels, or a
misleading control comparison. A missing or inconclusive structural comparison blocks
acceptance rather than being waived. Record seeds, complete params, source revision,
both control protocols, duration, metric variance, limitations, and what was observed.
Mechanical success alone cannot mark this phase complete.

**Mechanical.** The checks are the CI suite (`.github/workflows/ci.yml`), which
passes on `main`. They exercise structural births and deaths, not founder-only
scenarios:

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

**Judgment: delegated.** On 2026-10-04, David Webster, the project's owner, asked the
coding agent (Claude, working in Claude Code) to do M10's watching and to make the
final call itself. So the decision below is the agent's, not a human's. The agent watched an
evolved world in the browser and read the multi-seed metrics. No human watched these
runs for this decision.

It is written so that a human reviewer can check the reasoning against the evidence and
overturn the call; the case against it is listed below.

The evidence is in [Acceptance runs (M10)](phase-2-m8-evidence.md#acceptance-runs-m10):
five seeds, 1,000,000 ticks, both controls, configs and summaries in
[`experiments/phase-2-acceptance/`](../experiments/phase-2-acceptance/).

**Watched.** The agent loaded seed 42's evolving world at tick 1,000,000 into the
browser. That is a native `--save-run` bundle at revision `8e6bdbc`. It held 935 agents, 53 species,
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
the metrics in the evidence report.

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
  is reported in the evidence rather than assumed away.

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

## Scope

**In scope:** variable-length genomes and compiled brains; neural and sensor
structural mutation; genetic distance and species; phylogeny; the telemetry and 2D UI
to observe them; manual save/load in both shells; founder experiments and the
structural-null comparison.

**Not in scope:** sex or reproductive isolation, new sensory modalities, new effector
kinds or effector mutation, predation, signaling, body morphology, gene duplication,
evolvable meta-genes, Three.js, autosaves and retention scheduling, checkpoint
migration, timeline scrubbing, compression, automatic sweeps, and the Phase 7
performance rewrite. Serialized forward-compatibility hedges stay intact (spec §9.1).

## Working rules

- Tune existing parameters rather than changing mechanisms in response to outcomes;
  mechanism changes need human review and a spec update first.
- Report vectors with both controls across seeds; produce an unranked shortlist for a
  human to watch. Never maximize species count or brain size.
- A changed default's rationale lives on its `SimParams` field; a deliberate behavior
  change updates golden references with its explanation in the same commit, and
  independent causes stay in separate commits.
- Capacity saturation is a warning that growth may reflect allocator limits rather
  than metabolic selection.
