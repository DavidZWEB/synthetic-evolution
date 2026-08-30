# Phase 1 — Implementation Plan

Working breakdown of `phase-1-build-plan.md` into ordered milestones. The build plan lists
*deliverables*; this lists *build order*. They differ because the golden hash — the highest-value
test in the project (spec §7.8) — only becomes possible once the tick exists, so several earlier
deliverables cannot be verified end-to-end until M8.

Each milestone ends in a state that compiles and has a test proving it. Milestone numbers are
referenced in commit messages.

## Status

| Milestone | State |
|---|---|
| M0 Scaffold | done |
| M1 Foundations | done |
| M2 Pools and arenas | done |
| M3 Spatial hash | done |
| M4 Genome, mutation, crossover | done |
| M5 CTRNN | next |
| M6–M12 | not started |

## Cross-cutting rules for this phase

- The forward-compatibility checklist in `phase-1-build-plan.md` is **not** a milestone. Each item
  lands in the milestone that creates the struct it lives in, with a comment naming spec §9.1.
  The whole checklist is audited as one pass at M8.
- The golden hash is not pinned until M8. M4–M8 all change behavior by construction. Every update
  after M8 is deliberate, in the same commit as the behavior change, with a note on why.
- `sim-core` invariants (CLAUDE.md) apply from M0. M0 ships a lint test for the two that fail
  silently: no I/O, no platform transcendentals, no `thread_rng`.

---

## M0 — Scaffold

Build-plan task 1.

- Cargo workspace: `sim-core`, `shells/native`, `shells/wasm`.
- Dependencies pinned: `glam`, `rand` + `rand_pcg`, `serde` + `postcard`, `libm`. (`ts-rs`
  arrives at M10 with the inspector that consumes it.)
- Vite + Svelte 5 in `web/`.
- `staticwebapp.config.json` with COOP/COEP `globalHeaders`, in `web/public/` so Vite copies it
  into the `dist/` that Azure deploys — a repo-root config with a build subdirectory is silently
  ignored (spec §7.7).
- Guard test: greps `sim-core/src` for `thread_rng`, `std::time`, `std::fs`, `std::net`, and bare
  `.sin()` / `.cos()` / `.exp()` / `.powf()`. Invariants 1 and 2 fail silently weeks later;
  a short test is cheap insurance.

**Done when:** `cargo test` and `npm run dev` both run clean.

## M1 — Foundations

- `ids.rs` — newtype `AgentId`, `NeuronId`, `InnovationId`, `PartId`.
- `math.rs` — `libm` wrappers, yaw-only quaternion helpers.
- `rng.rs` — PCG wrapper with serializable state (the golden hash folds RNG state).
- `params.rs` — every constant from spec §5.5, serde, `Default`.

**Done when:** params serde round-trip, quaternion op tests, libm wrapper tests pass.

## M2 — Pools and arenas

Build-plan task 2.

- SoA world arrays exactly per spec §2.2a, including the dead fields (`parentB`, `partOffset`,
  `partCount`).
- Fixed capacity, free list.
- Gene and brain arenas with per-agent `(offset, len)`.

Phase 1 has fixed topology, so arena slots are a fixed stride and recycle through the same free
list. Access still goes through `(offset, len)`, so Phase 2's variable-length genomes are a change
of allocator rather than a change to every caller.

Counting-allocator test harness lands here.

**Done when:** spawn/despawn 10k agents in a loop with zero allocations after warmup.

## M3 — Spatial hash

Build-plan task 3.

- Uniform grid, cell size = max sensing radius, counting-sort rebuild.
- Triple-nested neighbour loop with the Z range pinned to `[0,0]` (spec §2.3).
- Brute-force O(n²) reference committed permanently — it is not deleted once the fast path passes
  (spec §7.8).

**Done when:** `proptest` differential parity against the brute-force reference on random
populations.

**The world is a torus.** It wraps in x and y, so distances are minimum-image and cell walks
wrap. The spec does not state a boundary condition either way; wrapping was chosen because a
bounded world hands out edge and corner geography for free, and an artificial refugium is hard
to tell apart from evolved anti-predator behaviour later. The choice is contained in
`spatial.rs` and the movement integrator, so it stays reversible.

## M4 — Genome, mutation, crossover

Build-plan task 4, first half.

- Typed-gene list per spec §3.1, serde. (`ts-rs` moved to M10: nothing consumes the
  generated TypeScript until the inspector exists, and derives with no consumer are
  exactly the pre-building CLAUDE.md warns against.)
- Innovation counter as a field on `World`, never a `static`.
- Fixed-topology builder: sensor→input neurons, hidden, oscillators, effector-source neurons.
- Mutation operators: weight perturbation and weight reset only.
- Crossover written and unit-tested by innovation alignment, called by nothing (spec §3.4).
- The `meta` gene kind exists and serializes into its slot, with its mutation operator disabled —
  the same treatment sensor `elevation` gets. Evolvable mutation rates unlock in a later phase.

**Done when:** genome serde round-trips; property test confirms no mutation orphans a sensor's
`target` neuron; crossover of two valid genomes yields a valid genome.

**Decisions taken here.**

*A sensor binds one target neuron per channel.* Spec §3.1 writes the binding as a single
`target`, but §2.2c has modalities returning two to four values — a `vision_ray` writes
distance plus signature RGB. Binding by id rather than by index is the principle that
indirection exists to serve, and one id per channel is what that means when a modality is
wide. `SensorGene::targets` is a fixed array; only the first `modality.channels()` entries
are read.

*`InnovationId::default()` is `NULL`, not 0.* A gene with a field left unset then fails
validation as a dangling reference, rather than silently binding to whichever neuron
happens to sort first.

*The founding topology is fully connected.* Phase 1 has no add-connection operator, so any
connection absent from the founder can never appear in any descendant. Dense means evolution
can reach any wiring by tuning weights toward zero; sparse would forbid some permanently. It
is also why a default genome is 284 genes, 240 of them connections.

*`k_brain` charges neurons and connections, `k_sensor` charges sensors.* Spec §5.2 keeps the
two coefficients apart so eyes can be made expensive without making brains expensive.
`genome::brain_complexity` and `genome::sensor_load` are the two quantities; nothing may be
counted by both.

## M5 — CTRNN

Build-plan task 4, second half.

- Euler integration, one step per tick, evaluated from previous activations.
- Evolvable `tau`; always-present oscillator neurons with evolvable period.

**Scale founder weights by fan-in — check this first.** M4 draws initial connection weights
uniform over ±`weight_limit` (±4), and the default topology gives every output and hidden
neuron 24 incoming connections. Summed input is then order ±20 before the activation
function ever sees it, so every sigmoid in every founder saturates on tick one and stays
pinned at 0 or 1. A saturated brain does not respond to its sensors, and weight mutation
moves it far too slowly to recover, so the population looks alive and evolves nothing —
which reads as a tuning failure rather than an initialisation bug.

The standard fix is to scale initial weights by fan-in (draw from roughly
`±weight_limit / sqrt(fan_in)`, or normalise per neuron). Two things this needs: the
initialisation range has to become a `SimParams` field rather than reusing `weight_limit`,
which is a *bound* and not a starting scale; and there should be a test asserting that a
freshly instantiated founder's activations are not all saturated. Do this before judging
anything about M5's dynamics.

**Done when:** a hand-built network produces known outputs, an oscillator neuron oscillates at
its specified period, and a founder brain's activations are distributed rather than pinned.

## M6 — Sensors and effectors

Build-plan task 5.

- Perception (`perceive.rs`): `vision_ray` (spatial-hash raycast → distance + signature RGB),
  `chemo` (concentration + gradient), `interoception` (energy). Writes `sensorScratch`.
- Directional params stored as `(azimuth, elevation)` pairs, elevation clamped to 0, its mutation
  operator absent (spec §4.1).
- Effectors: `thrust`, `turn` (rotation axis pinned to Z), `ingest`, brain-gated `reproduce`.
  All write to an intent buffer; none mutate the world.

**Done when:** an agent with hand-written weights demonstrably climbs a food gradient.

## M7 — World and economy

Build-plan task 6.

- Plants as simple non-brained entities.
- Chemo field as a 3D grid of depth 1; per-channel deposit, diffuse, decay.
- Metabolic costs per spec §5.2, including the brain-complexity term.
- Death at zero energy.
- Asexual reproduction with spatial viscosity — offspring spawn near the parent, parent energy is
  split (spec §5.4).
- Explicit `input` / `dissipated` ledger accumulators, so conservation is measured rather than
  inferred.

**Done when:** energy conservation holds over 10k ticks within epsilon.

## M8 — The tick

Build-plan task 7.

- The 11 steps of spec §2.4, in that order. Intents buffered; births and deaths deferred to step 10
  and resolved in agent-index order.
- `Command` enum, serde-serializable, carrying `apply_at_tick`, as the only route into the world.
- `state_hash` folding positions, energies, genomes, RNG state, and tick. Floats hashed via
  `to_bits`, fixed iteration order, never a `HashMap` walk.
- Forward-compatibility checklist audited here.

**Done when:** the golden hash is stable across two in-process runs, and native and
`wasm-pack test --node` agree on the same seed.

## M9 — WASM shell and renderer

Build-plan task 8.

- WASM shell: `step_many`, snapshot pointer/length, `inspect_agent` → JSON, `set_params`,
  `push_command`. Agent pool pre-allocated at max capacity so WASM memory never grows and JS
  typed-array views never detach (spec §7.3).
- Web: sim in a worker; `SnapshotTransport` interface with a SAB implementation and a transferable
  implementation, selected once at runtime from `crossOriginIsolated` (spec §7.7).
- Renderer: WebGL2 instanced quads, not canvas2d. The build plan allows either; canvas2d will not
  hold 5k agents with the sim at 100×, and would be discarded at Phase 5 regardless. Built to be
  presentable — sharing starts at Phase 2 (spec §8).

**Done when:** 5k agents render at 60fps with the sim at 1× and at 100×.

## M10 — Instrumentation

Build-plan task 9.

- Population and mean-energy time series, canvas or uPlot — never an SVG/DOM chart library.
- Agent inspector: click an agent, see genome and live neuron activations, pulled on demand for the
  one selected agent (spec §2.2b).
- `ts-rs` derives on the genome types, generating the TypeScript the inspector reads. Deferred
  here from M4: the point of generating them is to stop the inspector drifting from the genome,
  and until the inspector exists there is nothing to drift.
- Seed URL encoding. Speed control including pause.

**Done when:** you can watch a run, pause, click an agent, and read its brain.

## M11 — Headless telemetry

Build-plan task 10.

- `--metrics run.jsonl` on the native shell, one sample line per interval (spec §7.9).
- `diagnose` subcommand mapping metric signatures to the spec §10 failure modes.
- Random-brain control as a **separate world with the same seed and params**, not a marked lineage
  inside the live world. A control lineage sharing the world competes for the same energy and
  perturbs the thing it is measuring.

**Done when:** a 500k-tick headless run produces a metrics file, and `diagnose` correctly
identifies a deliberately induced extinction and a deliberately induced monoculture.

## M12 — Acceptance

**Mechanical:** golden hash, energy conservation, no-alloc, cross-target agreement, brute-force
neighbour parity — all passing.

**Judgment:** three or more seeds run side by side against the random-brain control, watched by a
human. Not self-certifiable (CLAUDE.md, spec §7.8 tier 3).

If food-seeking does not emerge, check spec §10 before changing code. It is almost always
metabolic cost too low, energy input too high, or mutation rate past error catastrophe — all
tuning, not bugs.
