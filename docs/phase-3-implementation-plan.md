# Phase 3 - Implementation Plan

Phase 3 adds **predation** from [`synthetic-evolution-spec.md`](synthetic-evolution-spec.md)
section 8: a bite effector, damage, energy transfer through corpses, and decomposition.
Design decisions live in the spec (§4.2 the bite, §5.1 corpses, §5.5 constants); this
plan tracks status and order. Phase 2's record is in
[`phase-2-implementation-plan.md`](phase-2-implementation-plan.md).

**Status: designed; M1 next.** The success criterion is **a carnivorous lineage becomes
established without going extinct or eating everything**, measured as a species whose
members take most of their energy from corpses, persisting for at least half a run on
several seeds while plant-eaters persist beside it, against both controls (spec §7.8).
Like every phase, it is judged by a human watching the sim, with multi-seed evidence
alongside; mechanical tests cannot certify it.

| Milestone | State |
|---|---|
| M0 Design | Recorded in the spec |
| M1 Corpses | Not started |
| M2 Bite, health, and kills | Not started |
| M3 Diet telemetry and the browser | Not started |
| M4 Calibration | Not started |
| M5 Acceptance | Not started |

## Working rules

Phase 2's rules carry over (`AGENTS.md`). Each mechanism lands with a setting that
reproduces Phase 2 exactly, so its pull request moves golden references only where it
widens hash coverage or changes storage; calibration then switches predation on, with
multi-seed evidence, as its own golden change. Energy conservation, no hot-loop
allocation, determinism, and native/WASM agreement are re-proved with predation active,
not only with it off.

## M1 - Corpses

A fixed pool of corpses (`corpses.max_corpses`), each a position and an energy pair.
Every death leaves `corpse_energy_fraction` of the agent's energy as a corpse at its
position and dissipates the rest; a death that finds the pool full dissipates the whole,
counted. Starved agents die empty, so until M2 nothing visible changes. Corpses decay at
`corpse_decay` per second into dissipation and are removed below `corpse_min_energy`.
`ingest` takes from the nearest food within reach, plant or corpse, in agent-index
order. Vision rays see corpses in `corpses.signature`.

- World state: hashed, checkpointed (a new checkpoint format), and in the snapshot; the
  renderer draws them.
- Tests: the energy ledger with corpses created, eaten, and decaying; no allocation;
  checkpoint continuation through corpse churn; ingest choosing the nearest food.

## M2 - Bite, health, and kills

`Action::Bite`, an intent per agent, and a per-agent cooldown. Swings resolve at the start
of step 7 in agent-index order: drive above `combat.gate`, cooldown elapsed, and energy
of at least `attack_cost`; pay the cost; damage the nearest agent in reach and in the
bite arc. Health regenerates in step 9; health at 0 dies in step 10 and leaves a corpse.
Founders carry a bite effector when `founder.bite` is set (default off until M4), which
raises the per-genome effector limit to 5.

- Tests: a bite transfers exactly what it should and the loss fraction is exact
  (spec §7.8 tier 1); damage order cannot decide a kill; cooldown and cost; arc and reach
  geometry; regeneration; conservation and no allocation with combat on; checkpoint
  continuation through kills; native/WASM agreement.

## M3 - Diet telemetry and the browser

Per-agent energy eaten from plants and from corpses, so a lineage's diet is measurable;
kills per sample; corpse stock. `summarize` and `diagnose` report diet fractions and the
spec §7.8 predation signals (carnivore biomass → 0; prey biomass → 0). The browser draws
corpses and bites, and the inspector shows health and diet.

## M4 - Calibration

The spec §7.8 sweep: `attack_cost` × `attack_damage` × plant input, several seeds per
cell, both controls, reporting which cells sustain coexistence and an unranked shortlist
rather than a winner. Ship defaults with their reasons on the fields.

## M5 - Acceptance

Run every check; record seeds, params, revision, controls, durations, variance, and what
was observed; a human decides whether a carnivorous lineage established.

## Scope

**In scope:** the bite effector, health, kills, corpses, decomposition into dissipation,
diet telemetry, and the browser views those need.

**Out of scope:** effector mutation (adding or removing a bite), carrion scent, corpse
nutrients feeding plants, armour or body-plan defences (Phase 5), signalling (Phase 4),
and prey refugia beyond what plant patchiness already gives.
