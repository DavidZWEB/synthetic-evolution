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
  mutate.rs      mutation operators (scalars only this phase)
  crossover.rs   NEAT alignment. Written and tested; nothing calls it until Phase 6
  world.rs       World struct, spawn/despawn, and step() when it lands
  tests/         invariants.rs — scans src for banned patterns; no_alloc.rs
shells/wasm/     wasm-bindgen bindings, snapshot pointer export
shells/native/   CLI: headless runs, batch sweeps, golden-hash tests
web/             Vite + Svelte 5 client. src/wasm/ is wasm-pack output, never committed
```

Still to come, one concept each: `tick.rs` (the 11 steps, order normative), `brain.rs`,
`perceive.rs`, and the systems that write the intent buffer.

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

## Working style

- Behavior changes require updating the golden hash deliberately, in the same commit, with a note on why the behavior changed. An unexplained hash update is a red flag.
- When a phase's tuning constants don't produce the expected outcome, that is a tuning problem, not a code bug. Check spec §10 before refactoring — most symptoms there map to one constant.
- Prefer adding a gene *kind* over adding a fixed field. The genome is a typed-gene list precisely so extensions stay additive (spec §3.1).
