# Synthetic Evolution

An open-ended artificial life simulator. Neural-network-brained organisms evolve under
implicit selection in a closed energy economy — no fitness function, no training
objective. Fitness is survival and reproduction.

Rust simulation core compiled to WASM, Svelte + WebGL client.

**Status:** Phase 1 complete through M12, including human acceptance of food-seeking
across three seeds against randomized-at-birth controls. Phase 2, genetic architecture,
has started with the variable-length allocator foundation; world integration and
structural mutation are still pending. See
[docs/phase-2-implementation-plan.md](docs/phase-2-implementation-plan.md).
The complete loop runs in WASM with live
instrumentation; the native shell produces paired evolving/control telemetry and
diagnoses known failure modes. The
remaining 5k-agent simulation-throughput limit is recorded separately from acceptance in
[docs/phase-1-implementation-plan.md](docs/phase-1-implementation-plan.md).

## Quick start

Two prerequisites: **[rustup](https://rustup.rs)** and **Node 22** (via
[nvm](https://github.com/nvm-sh/nvm), fnm, Volta, or a package manager — the version is
what matters, not the manager).

```bash
git clone <this repo> && cd synthetic-evolution
./scripts/setup.sh
```

The script installs the pinned Rust toolchain and the wasm target, installs `wasm-pack`,
installs the npm packages, and runs the test suite to prove it worked. It is idempotent,
so run it in every clone or worktree and again any time you suspect drift. Add
`--with-browser` when the checkout will run Playwright visual validation.

Then:

```bash
cargo test --workspace               # unit tests, invariant scan, no-alloc check
npm run dev --prefix web             # client on http://localhost:5173
cargo run --release -p native -- --seed 42 --ticks 500000 --metrics run.jsonl
cargo run -p native -- diagnose run.jsonl
```

On Windows, run `setup.sh` from WSL or Git Bash — the Rust and Node steps are
cross-platform but the script is bash.

If something in setup doesn't work on a fresh machine, that is a bug in
`scripts/setup.sh` rather than a step to remember. Fix the script.

## Layout

```
sim-core/        the simulation. Pure Rust: no I/O, no wasm-bindgen, deterministic
shells/native/   CLI — headless runs, batch sweeps, golden-hash tests
shells/wasm/     wasm-bindgen bindings for the browser worker
web/             Vite + Svelte 5 client
docs/            spec, phase plans, development setup
scripts/setup.sh one command to make a fresh machine work
```

## Where to read next

| If you want to | Read |
|---|---|
| Understand the design | [docs/synthetic-evolution-spec.md](docs/synthetic-evolution-spec.md) §1–§2 |
| Set up, add a dependency, or bump a toolchain | [docs/development.md](docs/development.md) |
| Follow Phase 2 implementation and remaining design decisions | [docs/phase-2-implementation-plan.md](docs/phase-2-implementation-plan.md) |
| Read Phase 1 milestones and acceptance evidence | [docs/phase-1-implementation-plan.md](docs/phase-1-implementation-plan.md) |
| Change code in `sim-core` | [AGENTS.md](AGENTS.md) — the five invariants, first |

## The short version of the rules

`sim-core` is deterministic: same seed and params produce a byte-identical run on every
platform. It does no I/O, holds no `static` mutable state, and does not allocate in the
tick. Energy is conserved, offspring spawn near their parents, and there is no explicit
fitness function. Each of those looks like a detail and is load-bearing —
[AGENTS.md](AGENTS.md) explains why before you change one.
