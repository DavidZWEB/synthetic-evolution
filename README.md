# Synthetic Evolution

An open-ended artificial life simulator. Neural-network-brained organisms evolve under
implicit selection in a closed energy economy — no fitness function, no training
objective. Fitness is survival and reproduction.

Rust simulation core compiled to WASM, Svelte + WebGL client.

**Status:** Phase 1, milestone M2 of 12. The world state, pools, and arenas exist; the
tick does not yet. See [docs/phase-1-implementation-plan.md](docs/phase-1-implementation-plan.md).

## Quick start

Two prerequisites: **[rustup](https://rustup.rs)** and **Node 22** (via
[nvm](https://github.com/nvm-sh/nvm), fnm, Volta, or a package manager — the version is
what matters, not the manager).

```bash
git clone <this repo> && cd synthetic-evolution
./scripts/setup.sh
```

The script installs the pinned Rust toolchain and the wasm target, installs `wasm-pack`,
installs the npm packages from the lockfile, and runs the test suite to prove it worked.
It is idempotent, so run it again any time you suspect drift.

Then:

```bash
cargo test --workspace               # unit tests, invariant scan, no-alloc check
npm run dev --prefix web             # client on http://localhost:5173
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
| Know what is being built right now | [docs/phase-1-implementation-plan.md](docs/phase-1-implementation-plan.md) |
| Change code in `sim-core` | [CLAUDE.md](CLAUDE.md) — the five invariants, first |

## The short version of the rules

`sim-core` is deterministic: same seed and params produce a byte-identical run on every
platform. It does no I/O, holds no `static` mutable state, and does not allocate in the
tick. Energy is conserved, offspring spawn near their parents, and there is no explicit
fitness function. Each of those looks like a detail and is load-bearing —
[CLAUDE.md](CLAUDE.md) explains why before you change one.
