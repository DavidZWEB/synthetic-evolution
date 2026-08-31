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
| M5 CTRNN | done |
| M6 Sensors and effectors | done |
| M7 World and economy | plants done; economy next |
| M8–M12 | not started |

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
counted by both. Their *values* are an M7 problem — see the budget note there.

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

**Decisions taken here.**

*A brain is compiled at birth, not read from the genome each tick.* The genome binds
neurons by innovation id, and resolving one is a binary search; at 240 connections per
agent per tick that lookup costs several times the arithmetic it feeds. `brain::compile`
resolves every endpoint to a slot once, and the tick is a flat scan over two slices.
The cost is memory: a compiled brain is 3.6 KB per agent and took the default world
from 57 MB to 71 MB, against a 96 MB ceiling. The two levers for getting it back are
recorded on `arena`'s module doc, in the order they should be pulled.

*The fan-in check found the initialisation bug this milestone predicted.* Measured over
64 founders, drawing weights from the full ±`weight_limit` bound pins 52% of wired
neurons against a rail of the sigmoid and cuts the mean activation slope to 0.064 of a
possible 0.25. With `weight_init_scale` at ±2 over √fan-in it is 0% and 0.217.
`brain`'s `without_fan_in_scaling_a_founder_saturates` holds that second measurement so
the first cannot quietly become vacuous.

*The Euler gain `dt/tau` is clamped at 1.* Mutation floors tau at 1e-3 against a dt of
1/60, and an unclamped gain of 16 makes the state alternate and diverge within a few
dozen ticks. The clamp is not a fudge: a neuron faster than the timestep can only be
observed tracking its input exactly.

*An oscillator's state is a phase, and its bias is a phase offset.* A free-running
neuron ignores its input, so it has no potential to integrate and the state slot is
free for the one thing it does need. Feeding `phase + bias` through the activation
makes the bias gene a phase offset rather than dead weight — the second thing an
oscillator can evolve, after its period. Output is ranged to [0, 1] to match the
sigmoid neurons it feeds, so one weight scale is right across a whole brain.

*`dt` is not folded into the compiled brain.* `SimParams` is settable at runtime, and a
baked-in timestep would silently ignore the new value. The reciprocals stored per
neuron — `inv_tau`, `inv_period` — are pure functions of the genome for the same reason.

## M6 — Sensors and effectors

Build-plan task 5.

- Perception (`perceive.rs`): `vision_ray` (spatial-hash raycast → distance + signature RGB),
  `chemo` (concentration + gradient), `interoception` (energy). Writes `sensorScratch`.
- Directional params stored as `(azimuth, elevation)` pairs, elevation clamped to 0, its mutation
  operator absent (spec §4.1).
- Effectors: `thrust`, `turn` (rotation axis pinned to Z), `ingest`, brain-gated `reproduce`.
  All write to an intent buffer; none mutate the world.

**Done when:** an agent with hand-written weights demonstrably climbs a food gradient.
Met — `sim-core/tests/steering.rs`. Read that file's header before treating it as more
than it is: the weights are hand-written, so it is a **wiring** test. It says the
sensorimotor chain closes with every sign convention agreeing, and says nothing about
whether food-seeking *evolves*, which is spec §8's criterion and M12's job. What it
measures is closest approach from eight starting headings including the one pointing
directly away, against a field that diffuses and decays every tick, with a control agent
whose single steering connection is cut.

**Scope: this milestone is wider than its bullet list.** The acceptance criterion needs
three things the bullets do not name — a field to carry a gradient, a way to sample it,
and movement to climb with. So M6 also lands the chemo field and the movement
integrator. Both are *systems*; M8 still owns assembling systems into the normative
11-step tick, along with the `Command` enum and the state hash. That is the seam M5 set:
`brain::step` and `World::step_brains` exist, `tick.rs` does not.

Split across two changes, because it is four modules: **perception** (compiled sensors,
`chemo.rs`, `perceive.rs`) and then **action** (intent buffer, effectors, movement, and
the gradient-climbing test).

**Decisions taken in the perception half.**

*Diffusion is part of the sensor working, not part of the economy.* A deposit lands in
one cell, so with no spreading the gradient is zero everywhere except inside that cell —
an agent one cell away smells nothing and there is no slope to climb. Diffusion is the
mechanism that turns food into a signal, so it lands here rather than at M7. M7 keeps
what it was actually for: plants as the deposit source, the `emit_chemo` effector, and
the conservation ledger.

*Explicit diffusion carries a stability limit, and 1.0 is past the useful part of it.*
The checkerboard mode is scaled by `1 - 2·diffuse` per tick: damped hardest at 0.5,
damped less above it, and at exactly 1.0 it flips sign forever without shrinking, so a
point deposit leaves a permanent grid artefact a nose would chase as if it were food.
Validation still allows [0, 1] — that is where the arithmetic stays bounded — and the
reasoning lives on `ChemoParams::diffuse`, where the next person tuning it will be.

*`Neuron` grew a fourth state field, and M5's shared scratch buffer is gone.* Perception
runs for every agent before any brain steps, so a tick's sensor input has to survive
between two passes; a single buffer cannot hold it. Per-neuron `input` costs 0.6 MB,
which is what a second arena would have cost, and removes a buffer whose clearing was
the caller's problem. `brain::step` consumes and clears it in one move.

*An eye reports nearness, not distance.* 1 at the eye, 0 at the limit of range, and 0
for an empty view. A raw distance would make "nothing there" and "something 60 units
away" different numbers, and would saturate any neuron it reached before the fan-in
scaling of M5 ever got a say.

**Decisions taken in the action half.**

*Every sensor channel is bounded, and directional ones are egocentric.* Designing the
acceptance test found the chemo sensor unusable: it returned a gradient in **world**
coordinates, which an agent cannot act on without knowing its own heading, and Phase 1
has no proprioceptor to tell it. It is also a global fact handed to a local organ, which
is the omniscience spec §2.2c exists to prevent. The gradient is now a unit vector in the
agent's own frame — `+x` ahead, `+y` left — which is what spec §4.1's "gradient
*direction*" asks for anyway. Magnitude went the same way: a raw gradient is a spatial
derivative of order 0.01 and would never move a neuron, a raw concentration accumulates
without limit and would saturate one permanently, and raw energy of 100 saturates on the
first tick. Strength now saturates as `c / (1 + c)` and energy is reported in tanks. The
rule is the one `vision_ray` already followed: **a sensor returns a value in the range
the brain can use**, because the fan-in scaling of M5 is calibrated for inputs near ±1
and an unbounded channel walks straight through it.

*Thrust is unsigned, turn is signed.* Spec §4.2 says thrust is a forward force, and an
agent that can reverse has less reason to evolve a turn. An angular velocity that can
only go one way is a permanent circle, so turn maps a sigmoid's neutral 0.5 to straight
ahead and swings either side of it.

*Intents are struct-of-arrays, not a queue.* All four Phase 1 effectors are
self-directed, so one slot per agent is the whole story and agent-index order is free
rather than something a sort has to restore. `bite` and `grab` name another agent and
will need a real queue; they can have one when they arrive.

*Turn is applied before thrust within a tick.* Either order is deterministic, but this
one makes a turn take effect immediately — otherwise a tick of sensing buys nothing and
steering always lags the thing it is steering at.

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

**The metabolic budget needs a pass here — it is the first point at which it is measurable.**
Two of the constants were left deliberately unresolved at M4, because guessing at three at
once, with no running population to check against, makes the result unfalsifiable.

`k_brain` was settled: 0.00005 rather than spec §5.5's 0.001, because that table's two
statements about it cannot both hold — a 200-unit brain at "~20% of base" wants the term near
0.01 when `base` is 0.05, but 200 × 0.001 is 0.2. At 0.00005 the default topology's 268
neurons and connections cost 0.013/tick, 0.27× base, which is where §5.5 asks for it.

What remains, at the default topology and body:

| term | per tick | vs base |
|---|---:|---:|
| `base` | 0.050 | 1.0× |
| `k_brain` × 268 units | 0.013 | 0.27× |
| `k_sensor` × 16 units | 0.160 | 3.2× |
| `k_size` × size² | 0.180 | 3.6× |
| total | 0.403 | ~250 idle ticks |

§5.5 wants ~2000 idle ticks on a full tank. Both overshooting terms come from defaults chosen
in this repo rather than from the spec: `k_size` is quadratic in `body.size`, which defaults
to 3 (at size 1 that term is 0.4× base), and `k_sensor` is multiplied by channel count rather
than sensor count — this codebase's reading of "0.01 each, weighted by modality", which makes
the default sensor set 16 weighted units instead of 5.

Change these against a running population and several seeds, not by arithmetic. The live
values and their reasoning are on `MetabolismParams::base` and `::k_brain`; update those
comments in the same commit.

**Decided: agents can see plants as well as smell them.** Recorded on
`PlantParams::signature`, where the next person asking will be looking.

Smell was already settled — `scent_rate` deposits into chemo channel 0, and M6's field
and chemo sensor consume it. Sight was not, and the falling-by-default answer was the
expensive one: `perceive::vision_ray` walked only the *agent* grid, and plants are a
separate pool. `k_sensor` charges by channel, so the default sensor set is 12 units of
eye against 4 of everything else; there is no predation until Phase 3 to make seeing
another agent worth anything, and Phase 1 has no remove-sensor operator, so selection
could not have shed the useless organs for the whole of the phase §8's criterion is
judged in. It would have surfaced during the tuning pass above looking like a `k_sensor`
problem rather than a missing query.

An emptied plant stays visible: the site persists and regrows, so blinking it out would
be stranger than leaving it, and telling a fat plant from a bare one now needs the nose —
a selective pressure worth having rather than a defect.

**Plants are fixed sites that regrow in place.** Spec §5.1's "get eaten, and reseed" is
read as regrowth, the weaker of the two meanings and the one Phase 1 needs. Positions
never change, so the plant neighbour grid is built once rather than every tick. The risk
is a static food map that rewards camping over foraging; if an M12 run shows that,
relocating a depleted site is a small change, and `plants.rs` names it.

**At carrying capacity the world absorbs less than its input rate.** The nominal rate is
shared evenly and each plant is capped, so the surplus never enters rather than being
stored anywhere. That is the honest behaviour of a saturated ecosystem, and it is why the
ledger records what `Plants::grow` returns rather than `energy_input_rate * dt` —
conservation has to be measured, not inferred.

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

**Keep the snapshot's sizing separate from the world's.** These two look like one decision and
are not. The snapshot is 57 bytes per agent — 0.5% of per-agent state, 0.27 MB at the default
5k and 2.7 MB even at Phase 7's 50k. World state is ~11.3 KB per agent, almost all of it
genome. Pre-allocating the snapshot at capacity is therefore free and should just be done.

The reason that matters: §7.3's detach hazard is the argument for pre-allocating *everything*,
but it only bites what JS actually views, which is the snapshot alone — inspector data is
pulled for one agent on demand through the command queue (§2.2b), and nothing else crosses the
boundary. Wiring the transport so it reads the world arenas directly would couple the cheap
buffer to the expensive one and make the pool impossible to grow later without breaking every
view. Spec §7.5 has the measurements.

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
