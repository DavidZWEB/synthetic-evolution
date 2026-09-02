# CLAUDE.md

**Synthetic Evolution** — an open-ended artificial life simulator: neural-network-brained organisms under implicit selection in a closed energy economy. Rust sim core compiled to WASM, Svelte + Three.js client. Full design in `docs/synthetic-evolution-spec.md`.

**Read `docs/synthetic-evolution-spec.md` §1–§2 before your first change.** Read the sections a phase names when you start that phase. Don't load the whole spec every session.

**Setup, toolchains, and dependency rules:** `docs/development.md`. A fresh machine is `./scripts/setup.sh`; `README.md` is the entry point for a human arriving at the repo.
**Current work:** `docs/phase-1-implementation-plan.md` — the milestone breakdown and what is done so far.

## The five invariants

Violating any of these is a bug even if tests pass and the sim runs.

1. **`sim-core` is deterministic.** Same seed + same params = byte-identical run, on every platform. Use `libm` for all transcendentals, never platform `sin`/`cos`/`exp`. Seeded PRNG only (`rand_pcg`), never `thread_rng`. Never iterate a `HashMap` to drive simulation — use agent-index order.
2. **`sim-core` does no I/O.** No file access, no network, no time, no logging to stdout. It is a pure library. The wasm and native shells own all I/O.
3. **No `static` mutable state in `sim-core`.** Including the innovation-ID counter, which is a field on `World`. A process must be able to hold several worlds.
4. **No allocation in the hot loop.** Fixed-capacity pools with free lists, arenas for variable-length data. There is a test for this.
5. **Every tunable is runtime config.** All constants live in `SimParams` (serde-serializable, settable from JS). Never hardcode a number someone might want to twiddle — tuning happens in the browser, not the compiler.

## Do not simplify these

They look arbitrary. They are load-bearing, and removing them produces a sim that runs fine and evolves nothing.

- **Offspring spawn near parents** (spatial viscosity). Without it, kin selection can't operate and communication will never evolve. Do not "improve" this to random placement.
- **Metabolic cost scales with brain and sensor complexity.** Without it, genomes bloat without limit and the sim slows to a crawl over hours.
- **Energy is conserved.** Input at a fixed rate, dissipation the only sink. Do not add free energy to fix a population crash — tune the input rate.
- **No explicit fitness function.** Fitness is survival and reproduction. Never add a scoring function or training objective.
- **Sensors query the world; they are not fed world state.** Agents get sensor outputs only. Reading world arrays directly makes every organism omniscient and destroys the selection pressure.
- **Dead genome fields stay** (`parentB`, sensor `elevation`, quaternion orientation, `partCount`). They are forward-compatibility hedges; removing them invalidates every saved genome later. See spec §9.1.

## Code design

### Shape

- **One module, one concept.** A file should be describable in a single sentence without "and."
- **~400 lines is a smell, 500 means split** (excluding inline `#[cfg(test)]` blocks). Long files here almost always mean several systems got tangled — split by responsibility, not by cutting at a line count.
- **Every file opens with a `//!` module doc**: what it's responsible for, and what it deliberately isn't. Two or three sentences.
- **Public surface is small.** Default to private; `pub` is a decision. A module exposing its internals invites the next module to reach in.

### Data-oriented, not object-oriented

There is no `Agent` struct with methods. Agents are indices into parallel arrays; behavior lives in systems that operate over slices.

**Systems take the slices they need, not `&mut World`.**

```rust
// Good — data dependencies are visible in the signature, and it's unit-testable.
fn perceive(
    positions: &[Vec3], signatures: &[Vec3], hash: &SpatialHash,
    genes: &GeneArena, params: &SimParams, out: &mut [f32],
) { ... }

// Bad — touches anything, testable only with a whole world, and the
// borrow checker will fight you the moment two systems overlap.
fn perceive(world: &mut World) { ... }
```

This isn't style preference. Narrow signatures are what let you run systems in isolation in tests, and what makes the eventual `rayon` parallelization (Phase 7) a change of iterator rather than a redesign.

### Patterns that fit this codebase

- **Newtype IDs.** `AgentId(u32)`, `NeuronId(u32)`, `InnovationId(u32)`, `PartId(u32)`. Zero cost, and it makes the "passed an array index where an innovation ID was wanted" bug a compile error instead of a subtly wrong simulation.
- **Arena + handle.** Variable-length data (brains, genomes, parts) lives in arenas; agents hold `(offset, len)`. Handles are indices, never pointers — arenas must serialize as flat buffers for checkpointing.
- **Intents, not direct mutation.** Systems that would mutate shared state push to an intent buffer; one applier resolves it in agent-index order. This is what makes §2.4's tick deterministic, and it generalizes beyond the steps that already use it.
- **Polymorphism at the edges, monomorphic core.** Traits at boundaries (`SnapshotTransport`), concrete types inside. No `dyn` in the tick — indirect calls through WASM function tables are expensive (spec §7.5).
- **Validate at the boundary, then use total functions.** `Result` at deserialization and shell entry points, where malformed input is a real possibility. Inside the tick, invariants are already established — use `debug_assert!` rather than threading `Result` through the hot loop. `sim-core` must not panic in release on any input that passed validation.
- **Instrumentation behind a feature or callback.** Stats collection must cost nothing in a headless overnight run.

### Comments

- **Explain why, never what.** `// increment index` is noise. `// previous activations, so behavior doesn't depend on pool order` is the reason the line exists.
- **Cite the spec, never this file.** `CLAUDE.md` is instructions for the agent, not
  project documentation — it is not shipped, not versioned with the design, and means
  nothing to someone reading `sim-core` on its own. Every rule worth putting in a comment
  has a home in `docs/synthetic-evolution-spec.md`; cite that. Where a rule genuinely
  has no spec section, state the reasoning in the comment rather than pointing at
  anything.

- **Anything load-bearing and strange gets a comment naming the spec section.** This is the important one. Code like the `parentB` field, the clamped elevation param, or deferring births to step 10 all look like dead weight or arbitrary choices. Without an anchor, someone eventually tidies them away.

  ```rust
  // Births deferred so free-list allocation doesn't depend on
  // iteration order. Determinism invariant — see spec §2.4.
  ```

- Otherwise prefer clear names over explanatory comments. A comment restating a badly named function is a rename waiting to happen.

### Don't pre-abstract

The spec describes phases years out. Build for the current phase only.

The forward-compatibility hedges in the phase plan are **exhaustive** — take those, and invent no others. A trait added now for a Phase 7 need is a guess about a design you don't understand yet, and it will be wrong in a way that's harder to remove than to have skipped. Spec §9.2 covers where this line falls and why.

## Layout

```
synthetic-evolution/         cargo workspace root
CLAUDE.md        this file — must stay at root to be picked up automatically
docs/            spec, phase build plans, development.md
scripts/         setup.sh — one command to make a fresh machine work
sim-core/        pure Rust, no I/O, no wasm-bindgen — the invariants above apply here
  ids.rs         newtype ids. InnovationId is genome identity; NeuronId is a brain slot
  math.rs        libm wrappers, yaw-constrained quaternion helpers
  rng.rs         the world's seeded PRNG
  params.rs      SimParams — every tunable constant
  pool.rs        fixed-capacity slot allocation, free list, alive flags
  arena.rs       flat arenas for variable-length per-agent data
  agents.rs      the SoA state arrays (spec §2.2a)
  spatial.rs     uniform grid hash over a toroidal world, plus its brute-force reference
  genome.rs      the typed-gene list and the rules that make one coherent
  founder.rs     Phase 1's fixed topology, instantiated with random scalars
  brain.rs       CTRNN: compiling a genome to a runnable network, and the Euler step
  perceive.rs    sensors: running an agent's organs against the world, into its brain
  chemo.rs       the pheromone field — sample, gradient, deposit, diffuse, decay
  effectors.rs   brain outputs into the intent buffer; changes nothing itself
  movement.rs    draining the thrust and turn intents into velocity and position
  plants.rs      the autotrophs: where every joule enters the world
  metabolism.rs  spec §5.2's cost function; what it costs to be alive for a tick
  ledger.rs      every joule in and out, so conservation is measured not assumed
  feeding.rs     moving energy from a plant into the agent touching it
  reproduction.rs when an agent may bud, and where the offspring lands
  mutate.rs      mutation operators (scalars only this phase)
  crossover.rs   NEAT alignment. Written and tested; nothing calls it until Phase 6
  world.rs       World struct, spawn/despawn, and step() when it lands
  tests/         invariants.rs — scans src for banned patterns; no_alloc.rs
shells/wasm/     wasm-bindgen bindings, snapshot pointer export
shells/native/   CLI: headless runs, batch sweeps, golden-hash tests
web/             Vite + Svelte 5 client. src/wasm/ is wasm-pack output, never committed
```

Still to come, one concept each: `tick.rs` (the 11 steps, order normative), and the
plants, metabolism, and reproduction that close the energy economy.

## Commands

```
./scripts/setup.sh                   # fresh machine: toolchain, wasm-pack, npm ci
cargo test --workspace               # unit tests + invariant scan + no-alloc
cargo run -p native -- --seed 42 --ticks 100000
npm run wasm --prefix web            # rebuild bindings into web/src/wasm/
npm run dev  --prefix web            # client on http://localhost:5173
```

`npm run dev` does not rebuild the wasm bindings — run `npm run wasm` after changing
`sim-core`. It gets chained into `dev` and `build` at M9, when the client imports them.

The golden-hash and energy-conservation tests arrive with the tick; `cargo test`
currently covers the unit tests, the source-invariant scan, and the no-alloc check.

## Definition of done

A phase is complete when **both** hold:

- **Mechanical:** golden-hash test updated and passing, energy conservation passing, no-alloc test passing, cross-target hash agreement.
- **Judgment:** the phase's success criterion in spec §8 is met, verified by a human watching the sim.

The second cannot be self-certified. Phase success criteria are about whether something *interesting* evolved, which no test asserts. Report what you observe and let the human make the call. Never mark a phase done on mechanical tests alone.

## Tuning vs. code change

These are separate loops and must stay separate.

**Tuning `SimParams` is yours.** Run headless sweeps, read metrics, run `diagnose`, report findings (spec §7.9). Always use several seeds per configuration and report the variance — a single good run is the most common way an automated report misleads.

**Editing `sim-core` in response to metric outcomes requires human review.** "Population is unstable" can be fixed by weakening a metabolic cost: every metric improves and the simulation is quietly ruined. If a metric looks wrong, check spec §10 first — most symptoms there map to one constant, not to a bug.

**Never optimize toward a scalar objective.** Report a vector of metrics with the random-brain control alongside. Don't rank configurations or pick a winner; produce a shortlist for a human to watch. Every metric here is Goodhart-able — maximizing species count just means lowering the speciation threshold until noise counts as speciation.

**Where a default departs from spec §5.5, the reason lives on the field.** Not in a commit message and not here — on the `SimParams` doc comment, where the next person to tune it will be looking. Add to those notes rather than replacing them when the numbers move.

## Landing changes

**Never commit to `main`.** Every change goes on a branch and lands through a pull request —
code, docs, tuning notes, a one-line typo fix. There is no "it's only documentation"
exception: a docs change that records a decision is exactly the kind worth a second pair of
eyes, because nothing else in the repo will catch it if the reasoning is wrong. Branch names
follow the work: `phase-1/m8-tick`, `docs/memory-footprint-findings`, `fix/arena-empty-block`.

**Then review the PR you just raised.** Opening it is not the end of the task. Read the diff
back as a reviewer would — `/code-review`, or `gh pr diff` — and report the findings in the
same reply that hands over the PR. What to look for, roughly in the order things go wrong here:

- **The five invariants.** `tests/invariants.rs` scans for 1–3 lexically and `no_alloc.rs`
  measures 4, so the review's job is the part a grep cannot see: a constant hardcoded where
  a `SimParams` field belongs — invariant 5, which nothing tests — and determinism that is
  semantic rather than textual, such as a summation whose order varies with input, or
  iteration driven by anything but agent index. The golden hash catches that second class
  from M8 onward; until M8 this review is the only thing that does.
- **The load-bearing and strange.** Dead genome fields, the clamped elevation param, deferred
  births. Check each one still has a comment naming the spec section that justifies it —
  these are what a later tidy-up removes.
- **Comments.** Why and not what, citing `docs/synthetic-evolution-spec.md` and never this
  file (see *Comments* above).
- **Shape.** A module doc on every file, ~400 lines a smell and 500 a split, systems taking
  the slices they need rather than `&mut World`, newtype IDs at the boundaries, `pub` as a
  decision.
- **Ordinary correctness.** Inverted conditions, off-by-one, the other callers of a changed
  function, a validation quietly dropped.
- **Tests that assert less than they appear to.** A misparenthesised `abs`, a threshold
  loose enough to pass either way, a fixture rich enough that the thing under test never
  binds. A green test that cannot fail is worse than no test.

**Fixes from the review land as a second commit on the same PR** — never amended into the
first, never squashed onto it. The history is the evidence that the step happened: a fix
folded into the original commit makes a review that caught something look exactly like a
review that caught nothing, and leaves the next person unable to tell whether one ran at
all. Two commits — *the change*, then *what reviewing it found* — also give the human a
diff of the correction on its own, which is usually the more interesting half.

Fix what is plainly wrong; raise what is a judgment call as a comment and let the human
decide. Say what you reviewed even when you found nothing — an explicit "here is what I
checked and it was clean" is worth reading, and a silent PR is indistinguishable from an
unreviewed one.

## Working style

- Behavior changes require updating the golden hash deliberately, in the same commit, with a note on why the behavior changed. An unexplained hash update is a red flag.
- When a phase's tuning constants don't produce the expected outcome, that is a tuning problem, not a code bug. Check spec §10 before refactoring — most symptoms there map to one constant.
- Prefer adding a gene *kind* over adding a fixed field. The genome is a typed-gene list precisely so extensions stay additive (spec §3.1).
