# AGENTS.md

These instructions apply to every coding agent working in this repository. Tool-specific
instruction files should point here rather than duplicate this guidance.

**Synthetic Evolution** — an open-ended artificial life simulator: neural-network-brained organisms under implicit selection in a closed energy economy. Rust sim core compiled to WASM, Svelte + WebGL2 client. Full design in `docs/synthetic-evolution-spec.md`.

**Before changing `sim-core`, simulation behavior, or serialized world state, read
`docs/synthetic-evolution-spec.md` §1–§2 once and the sections relevant to the task.**
For tooling, documentation, or UI-only changes, read only the relevant documentation.
Don't load the whole spec every session.

**Setup, toolchains, and dependency rules:** `docs/development.md`. A fresh checkout is `./scripts/setup.sh`; `README.md` is the entry point for a human arriving at the repo.
**Roadmap and current status:** `docs/phase-2-implementation-plan.md` tracks current
implementation and remaining design decisions; `docs/phase-1-implementation-plan.md` retains Phase 1's milestones and
acceptance evidence. These plans are authoritative for milestone status; don't infer
it from this file.

**Agent tooling:** MCP configuration discovery is client-specific. Copilot CLI reads
`.github/mcp.json`; Claude Code uses `.mcp.json`; VS Code uses `.vscode/mcp.json`.
The shared browser workflow lives in `.github/skills/web-client-visual-check/`; Copilot
CLI discovers project skills there, while other agents may require their own supported
skill location.

## The five invariants

Violating any of these is a bug even if tests pass and the sim runs.

1. **`sim-core` is deterministic.** Same seed + same params = byte-identical run, on every platform. Use `libm` for all transcendentals, never platform `sin`/`cos`/`exp`. Seeded PRNG only (`rand_pcg`), never `thread_rng`. Never iterate a `HashMap` to drive simulation — use agent-index order.
2. **`sim-core` does no I/O.** No file access, no network, no time, no logging to stdout. It is a pure library. The wasm and native shells own all I/O.
3. **No `static` mutable state in `sim-core`.** Including the innovation-ID counter, which is a field on `World`. A process must be able to hold several worlds.
4. **No allocation in the hot loop.** Fixed-capacity pools with free lists, arenas for variable-length data. There is a test for this.
5. **Every tunable is runtime config.** Every value that changes simulation behavior or
   might reasonably be tuned belongs in `SimParams` (serde-serializable, settable from
   JS). Constants that encode representation rather than a runtime choice, such as
   sentinel values or serialized byte widths, may remain compile-time constants. Never
   hardcode a behavioral number someone might want to twiddle — tuning happens in the
   browser, not the compiler.

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

- **One module, one concept.** A file should be describable in a single sentence without "and." That's the real test — not a line count.
- **Long files are worth a second look, not an automatic split.** Past ~500 lines (excluding inline `#[cfg(test)]` blocks), ask whether it's still one concept. A flat list of fields (`params.rs`) or a struct's `impl` split across focused modules (`world.rs`, `tick.rs`, `world/lifecycle.rs`) can honestly stay long. Several unrelated systems sharing a file cannot — split by responsibility.
- **Keep in-crate Rust tests inline.** Put `#[cfg(test)] mod tests { ... }` in the implementation file. Keep integration tests and their shared fixtures under the crate's `tests/` directory.
- **Place modules by ownership.** Keep independently scoped systems as siblings when crate-visible inputs suffice; use a private child module for owner-specific implementation that needs the parent's private items rather than widening visibility.
- **Every Rust source module opens with a `//!` module doc**: what it's responsible for, and what it deliberately isn't. Two or three sentences.
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
- **Evolve through gene kinds, not fixed fields.** Prefer adding a gene kind over adding
  a fixed field. The genome is a typed-gene list precisely so extensions stay additive
  (spec §3.1).

### Comments

- **Explain why, never what.** `// increment index` is noise. `// previous activations, so behavior doesn't depend on pool order` is the reason the line exists.
- **Cite the spec, never this file.** `AGENTS.md` is agent guidance, not the authoritative
  project design, and means nothing to someone reading `sim-core` on its own. Every rule
  worth putting in a comment has a home in `docs/synthetic-evolution-spec.md`; cite that.
  Where a rule genuinely has no spec section, state the reasoning in the comment rather
  than pointing at anything. If implementation needs a design change, discuss it with a
  human first and update the spec in the same change — don't leave the spec stale and
  compensate with a long code comment explaining the divergence.
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
docs/          authoritative design, roadmap, and development workflow
sim-core/      deterministic, I/O-free Rust simulation; all five invariants apply
shells/wasm/   wasm-bindgen boundary and snapshot-memory export
shells/native/ native CLI and host-side I/O
web/           Svelte 5 + WebGL2 renderer and snapshot transport
scripts/       repository setup and automation
```

Module responsibilities live in each module's `//!` documentation.
`sim-core/src/lib.rs` is the authoritative module index; inspect the tree rather than
maintaining a second inventory here. `web/src/generated/` is committed; `web/src/wasm/`
is generated locally and is not.

## Commands and validation

```
./scripts/setup.sh                   # fresh machine: toolchain, wasm-pack, npm ci
npm run dev  --prefix web            # client on http://localhost:5173
npm run wasm --prefix web            # rebuild only web/src/wasm/
```

For native smoke runs and headless telemetry:

```
cargo run -p native -- --seed 42 --ticks 100000
cargo run --release -p native -- \
  --seed 42 --ticks 500000 --sample-every 1000 --metrics run.jsonl
cargo run -p native -- diagnose run.jsonl
```

Run the smallest relevant checks while developing. Before opening a pull request, run
the checks covering every changed surface; `.github/workflows/ci.yml` is the
authoritative full list.

| Changed surface | Required checks |
|---|---|
| Rust | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` |
| `sim-core` | Rust checks plus `cargo check -p wasm --target wasm32-unknown-unknown` |
| WASM boundary | `sim-core` checks plus `wasm-pack test --node shells/wasm` |
| Tick arithmetic or determinism | `sim-core` checks plus `wasm-pack test --node shells/wasm` |
| Rust/TypeScript contract | Rust checks plus `npm run types --prefix web`; `git diff --exit-code -- web/src/generated` |
| Web | `npm test --prefix web`; `npm run check --prefix web`; `npm run build --prefix web`; `npm run test:browser --prefix web` |

`npm run dev` and `npm run build` rebuild the WASM bindings before Vite starts.
Native tests include the golden hash but cannot prove cross-target agreement; run the
WASM test after anything that touches tick arithmetic.

## Declaring a phase complete

A phase is complete when **both** hold:

- **Mechanical:** the golden hash, energy conservation, no-allocation, and cross-target
  agreement tests pass. Update the golden hash only for a deliberate behavior change,
  in the same commit, and explain why it changed.
- **Judgment:** the phase's success criterion in spec §8 is met, verified by a human watching the sim.

The second cannot be self-certified. Phase success criteria are about whether something *interesting* evolved, which no test asserts. Report what you observe and let the human make the call. Never mark a phase done on mechanical tests alone.

## Tuning vs. code change

These are separate loops and must stay separate.

**Adjusting existing `SimParams` values is tuning.** That includes parameters which alter
selection pressure. Changing checked-in defaults is also a deliberate behavior change
and follows the golden-hash rule above. Changing algorithms, state flow, energy
accounting, or the mechanisms that create selection pressure is a code or design change
and requires human review first. Check spec §10 before proposing one — most symptoms
there map to a parameter, not a bug.

Use several seeds per configuration and report the variance. When the active phase
provides the experiment tooling, use its headless metrics and `diagnose` command
(spec §7.9). A single good run is the most common way an automated report misleads.

**Never optimize toward a scalar objective.** Report a vector of metrics with the random-brain control alongside. Don't rank configurations or pick a winner; produce a shortlist for a human to watch. Every metric here is Goodhart-able — maximizing species count just means lowering the speciation threshold until noise counts as speciation.

**Where a default departs from spec §5.5, the reason lives on the field.** Not in a commit message and not here — on the `SimParams` doc comment, where the next person to tune it will be looking. Add to those notes rather than replacing them when the numbers move.

## Landing changes

**Never commit to `main`.** Every change goes on a branch and lands through a pull request —
code, docs, tuning notes, a one-line typo fix. There is no "it's only documentation"
exception: a docs change that records a decision is exactly the kind worth a second pair of
eyes, because nothing else in the repo will catch it if the reasoning is wrong. Branch names
follow the work: `phase-1/m8-tick`, `docs/memory-footprint-findings`, `fix/arena-empty-block`.

**Then review the PR you just raised.** Opening it is not the end of the task. Read the
diff back as a reviewer would — use the agent's code-review workflow or `gh pr diff` —
and report the findings in the same reply that hands over the PR. Review against the
five invariants, load-bearing rules, and code-design guidance above, then check ordinary
correctness, every changed caller, and whether each test could fail for the bug it claims
to catch. Report explicitly when the review is clean.

**Fixes found after the PR opens land as a second commit** — never amended into the first.
The audience is the human reading the open PR, not `main`'s history: a separate commit hands
them the corrections as a diff of their own, and lets GitHub show what moved since they last
looked. Amending destroys precisely that, and makes a review that caught a real error look
identical to one that caught nothing.

**PRs squash on merge**, so the branch collapses to a single commit on `main`. GitHub
prefills that commit's body by concatenating the branch's messages, which is why each one is
still worth writing properly — but the prefill is editable and the title falls back to the
PR's, so read the squash message before merging rather than trusting it. The explanation
for any golden-hash update must survive into the squash message.

Keep independent causes of golden-hash changes in separate commits, each with its own
reference update and explanation. Review, revert, and bisect must be able to distinguish
a dynamics change from expanded hash coverage or corrected metadata, even when the PR
will eventually squash.

Fix what is plainly wrong; raise judgment calls as comments and let the human decide.
