# Phase 2 - Implementation Plan

Phase 2 builds the **genetic architecture** from
[`synthetic-evolution-spec.md`](synthetic-evolution-spec.md) section 8. Design
decisions live in the spec; this plan tracks status and the remaining work. Phase 1's
acceptance evidence is in [`phase-1-implementation-plan.md`](phase-1-implementation-plan.md).

**Status: M1–M8 are implemented; M9 acceptance needs human judgment.** The success
criterion is **brains grow in complexity and distinct species appear**, judged by a
human with reproducible multi-seed evidence and both controls alongside: the scalar
control and the structural null (spec §7.8). More genes or more cluster labels alone
are not evidence that useful complexity evolved.

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
| M8 Founder experiments | Evidence reported, including the shipped-configuration rerun and species calibration; awaiting human review |
| M9 Acceptance | Not started |

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

## Open work

The shipped-configuration rerun and species calibration are reported in
[`phase-2-m8-evidence.md`](phase-2-m8-evidence.md): growth is still not distinguished
from the structural null, evolving lineages shed perception, the threshold (not
deletion) sets species counts.

1. **Make perception pay, then re-measure.** The leading explanation for growth that
   the null cannot distinguish is an environment where a blind, clock-driven grazer
   does as well as a forager. Investigate with parameters first (food input,
   density, and patchiness; sensor and brain costs), across seeds, reporting
   functional wiring alongside size. This is tuning, not a mechanism change.
2. **Functional-complexity telemetry.** Add to the metrics how much structure lies on
   a sensor-to-effector path (`experiments/phase-2-m8/wiring.py` is the prototype),
   so growth can be judged by what is wired in rather than gene counts.
3. **Species reporting.** Report persistent species at more than one threshold; the
   0.5 default stays a provisional scale.
4. **M9 acceptance**, below.

## M9 - Acceptance

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
