# Development setup

Two toolchains: Rust (the sim core) and Node (the web client). Both pin their versions in files that their version managers read automatically, so cloning on a new machine is two prerequisites and one script.

## The rules

1. **Commit both lockfiles.** `Cargo.lock` and `package-lock.json`. This is what makes two machines build identical dependency trees. (You may read that libraries shouldn't commit `Cargo.lock` — true, and irrelevant: this is an application workspace.)
2. **Pin both toolchains in-repo.** `rust-toolchain.toml` and `.nvmrc`. `rustup` and `nvm` read these automatically when you `cd` into the directory.
3. **Never commit build output.** `target/`, `node_modules/`, `web/src/wasm/` are all regenerable and enormous.
4. **One setup script.** If a new machine needs a step that isn't in `scripts/setup.sh`, the script is wrong.

Rust needs no virtualenv equivalent. Cargo builds into a project-local `target/` and dependencies are resolved per-project by default — isolation is the default rather than something you arrange.

## Prerequisites on a fresh machine

Only two things installed globally:

- **rustup** — <https://rustup.rs>. Installs and manages Rust itself; don't install Rust through a package manager, it fights the toolchain file.
- **Node 22** — nvm (<https://github.com/nvm-sh/nvm>), or fnm, Volta, or a package manager. The version is what matters, not the manager: `setup.sh` uses nvm when it finds it and otherwise checks the Node you already have against `.nvmrc`.

Then `git clone`, `./scripts/setup.sh`, `npm run dev --prefix web`.

Run setup once in every clone or worktree. Rust toolchains and downloaded browser
binaries, when requested, are reused from their user-level caches, but
`web/node_modules/` is intentionally checkout-local. The setup and WASM build scripts
resolve Cargo through rustup and `wasm-pack` through Cargo's configured install root, so
the web build does not depend on machine-specific Cargo or wasm-pack PATH entries.
The scripts find rustup on `PATH`, under the rustup.rs default, or in standard
Homebrew/Linuxbrew locations; nonstandard installations still need their bin directory
on `PATH`.

## Files

### `rust-toolchain.toml` (repo root)

```toml
[toolchain]
channel = "1.98.0"
components = ["rustfmt", "clippy"]
targets = ["wasm32-unknown-unknown"]
```

The important file for your multi-machine question. `rustup` reads it on every `cargo` command and silently installs the right compiler and the WASM target if they're missing. No one has to remember `rustup target add`.

Pin an exact version rather than `"stable"`. Determinism is a project invariant, and while a compiler bump is unlikely to change simulation output, "unlikely" is not what you want a golden-hash test resting on. Bump it deliberately, in its own commit, and re-run the hash tests.

### `Cargo.toml` (repo root — the workspace)

```toml
[workspace]
resolver = "3"
members = ["sim-core", "shells/native", "shells/wasm"]

[workspace.package]
edition = "2024"
rust-version = "1.98"

# Versions declared once here; each crate references them.
[workspace.dependencies]
glam = { version = "0.30", default-features = false, features = ["libm", "scalar-math", "serde"] }
rand = { version = "0.9", default-features = false }
rand_pcg = { version = "0.9", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc"] }
libm = "0.2"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
```

**Resolver 3, not 2.** It is the edition-2024 default and is MSRV-aware, so dependency resolution cannot quietly pick a version newer than the pinned toolchain.

**The feature flags are load-bearing, not decoration.** `glam` with `default-features = false` plus `libm` routes its transcendentals through software implementations rather than the platform's, which is invariant 1 — platform `sin`/`cos` differ between native and WASM. `scalar-math` is what turns glam's SIMD backends off; `default-features = false` does **not**, which this file claimed until CI on an x86_64 runner disagreed with an aarch64 laptop about a pinned golden hash. Note that neither flag governs `Vec3`'s 12-byte layout — `Vec3` is scalar either way, so the compile-time `size_of` assert in `agents.rs` (which the zero-copy snapshot views depend on) is not the thing protecting determinism here. `Quat` is the SIMD-backed type. `rand_pcg`'s `serde` feature is what lets RNG state be checkpointed and resumed mid-run. Changing any of these is a determinism decision, not a tidy-up.

A workspace produces **one `Cargo.lock` at the root** covering every crate, which is what you want — the native and WASM shells cannot drift onto different dependency versions.

`[workspace.dependencies]` declares each version once; member crates then write `glam = { workspace = true }`. With three crates sharing most dependencies this saves real maintenance.

In `Cargo.toml`, `"0.30"` means "compatible with 0.30" — 0.30.4 is acceptable, 0.31 is not. The lockfile pins the exact resolved version. Cargo.toml states intent; Cargo.lock states fact.

### `.nvmrc` (repo root)

```
22
```

`nvm use` reads it. Set it to whatever `node --version` gives you after installing an LTS release.

### `.gitignore`

```gitignore
# Rust
/target
**/*.rs.bk

# Node
node_modules/
dist/

# wasm-pack output — regenerated by `npm run wasm`, never committed
web/src/wasm/
pkg/

# Run artifacts
*.jsonl
checkpoints/
!seeds/*.json

# Profiling output — perf and flamegraphs on the native shell (spec §7.2)
perf.data*
flamegraph.svg
*.profraw

# OS and editors
.DS_Store
.idea/
.vscode/

# Per-machine tool override; shared agent guidance lives in AGENTS.md
.claude/settings.local.json
```

`web/src/wasm/` is `wasm-pack` output — regenerated on every build, never committed. The `seeds/` exception is deliberate: keep interesting seeds (see below).

### `scripts/setup.sh`

In order: `rustup show` (installs the pinned toolchain, components, and wasm target),
`cargo install --locked wasm-pack@0.15.0`, Node via nvm if it is present,
`npm ci --prefix web`, then `cargo test --workspace` to prove it worked. It is
idempotent — running it on an already-set-up machine does nothing but check.

`--skip-tests` drops the verification step. `--with-browser` additionally installs the
Chromium version matched to the locked Playwright package; on Linux it also installs
Chromium's required system libraries. Browser installation is opt-in because it is a
large download needed only for automated visual validation. Use the default setup
interactively unless that checkout will run the Playwright skill.

Read the script rather than trusting a copy pasted here; a duplicated script drifts. Four details in it are worth understanding:

**`npm ci` rather than `npm install`.** `ci` installs exactly what the lockfile says and errors if `package.json` and the lockfile disagree. `install` will happily update the lockfile, which is how two machines silently diverge. Use `install` only when deliberately adding a dependency.

**`cargo install --locked wasm-pack@0.15.0`.** This is the one real gap in the setup. Cargo-installed *binaries* are not covered by your `Cargo.lock` — they're separate programs. Pinning the exact version in this script is the workaround. Without `--locked`, cargo resolves *wasm-pack's own* dependencies fresh, which occasionally fails to build.

**nvm has to be sourced, not called.** `nvm` is a shell function rather than a binary, so a script cannot invoke it without first sourcing `$NVM_DIR/nvm.sh` — and under `set -u` the naive version fails outright. The script sources it when it exists, and otherwise compares your installed Node against `.nvmrc` and warns rather than failing. Requiring one particular version manager to build the project is not worth it.

**Rust tools used by npm are resolved, not assumed.** `scripts/rustup.sh` checks
`PATH`, the rustup.rs default, and standard Homebrew/Linuxbrew locations before
explaining how to repair a nonstandard installation. It also normalizes native Windows
paths for Git Bash. `rustup which cargo` then selects the Cargo paired with the
repository's pinned toolchain.
`scripts/cargo.sh` exposes that Cargo to the type-generation package script, while
`scripts/wasm-pack.sh` also locates the Cargo install root and makes Cargo visible to
`wasm-pack`. Package scripts invoke these through `bash`, so the documented Git Bash
workflow does not depend on npm's platform-specific script shell. Normal command-line
examples remain standard `cargo` and `wasm-pack` commands.

### `web/package.json` (scripts section)

```json
{
  "scripts": {
    "wasm": "bash ../scripts/wasm-pack.sh build shells/wasm --target web --out-dir ../../web/src/wasm",
    "browser:install": "playwright install chromium",
    "browser:install-with-deps": "playwright install --with-deps chromium",
    "types": "bash ../scripts/cargo.sh test -p sim-core --lib export_bindings",
    "test": "node --test src/**/*.test.js src/**/*.test.ts",
    "test:browser": "npm run wasm && node --test browser/*.test.js",
    "check:types": "tsc --noEmit",
    "check": "svelte-check --tsconfig ./tsconfig.json",
    "dev": "npm run wasm && vite",
    "dev:transferable": "npm run wasm && vite --mode transferable",
    "build": "npm run wasm && vite build",
    "preview": "vite preview"
  }
}
```

The WASM build has to run before Vite, since M9's worker imports its output. Chaining it
into both entry points means `dev` and `build` work from a clean checkout and cannot
silently use stale local bindings.

`dev` supplies the cross-origin isolation headers required for the shared-buffer
transport. `dev:transferable` deliberately omits them so the browser fallback can be
exercised rather than only unit-tested.

The Playwright MCP package is an exact dev dependency, so its server and Playwright
versions are recorded in `package-lock.json`; a test keeps that pin aligned with
`.github/mcp.json`. The direct Playwright test dependency is kept on the same runtime,
so `setup.sh` installs one matching Chromium binary for both entry points.
MCP can pin a prerelease Playwright build. If that runtime becomes unavailable, upgrade
MCP and its matching direct Playwright dependency together and regenerate the lockfile;
do not change the browser-test pin alone.
`.github/mcp.json` is Copilot CLI's repository configuration; other agents use their
own MCP discovery locations, listed in `AGENTS.md`. Copilot CLI also discovers the
shared visual-check skill under `.github/skills/`.

`npm run test:browser --prefix web` uses the existing Node test runner with the pinned
Playwright Chromium runtime. It builds WASM, starts its own local Vite servers, and
exercises real workers on both transports, paused inspection, and narrow-viewport
layout. Install the browser once with `./scripts/setup.sh --with-browser`; CI opts into
that setup as well. CI caches `~/.cache/ms-playwright` by runner OS, architecture, and
the resolved Playwright version in the lockfile. Setup still runs on a cache hit so fresh
Linux runners receive the required system libraries. The ordinary `npm test` remains
browser-independent.

`web/src/generated/` is different from `web/src/wasm/`: its TypeScript genome bindings
are generated by `ts-rs` and committed so the inspector has a reviewable contract.
`npm run types --prefix web` refreshes them. Native tests generate the same files, and
CI fails if that changes the checked-in output.

## Adding a dependency

```bash
# Rust
cargo add glam --package sim-core        # updates Cargo.toml + Cargo.lock

# Node
npm install package-name --prefix web    # updates package.json + package-lock.json
```

Commit the lockfile change in the same commit as the manifest change. A manifest change without its lockfile change is what breaks the next machine.

## Headless metrics

The native shell runs an evolving world beside a same-seed, same-params scalar-heredity
control and writes both metric vectors into one self-describing JSONL stream:

```bash
cargo run --release -p native -- \
  --seed 42 --ticks 500000 --sample-every 1000 --metrics run.jsonl
cargo run -p native -- diagnose run.jsonl
```

Use `--params params.json` for a partial or complete `SimParams` document; absent fields
use shipped defaults. Omit `--metrics` for only a completion summary and final hashes;
use `--metrics -` to stream JSONL to stdout. `diagnose --json` emits a machine-readable
report.

### Opt-in neural structural mutation (M2)

All five shipped rates under **`SimParams.mutation.structural` are zero**. Missing
fields, including old saved params and shared URLs, retain zero rates; the default
scalar-only dynamics and random-draw sequences remain unchanged. To opt in, pass a
partial params document such as this with `--params params.json` (the WASM constructor
and `Sim.set_params` accept the same JSON):

```json
{
  "mutation": {
    "structural": {
      "remove_connection_rate": 0.001,
      "remove_neuron_rate": 0.001,
      "toggle_connection_rate": 0.01,
      "add_connection_rate": 0.01,
      "add_neuron_rate": 0.005,
      "split_neuron_bias": 0.0,
      "split_input_weight": 1.0
    }
  }
}
```

This is a protocol example, not a recommended configuration. Rates are per-offspring
operator probabilities in `[0, 1]`, applied in the order shown: remove connection,
remove neuron, toggle connection, add connection, then split an enabled connection
to add a neuron. Initializers must be finite; when splitting is enabled,
`abs(split_input_weight)` cannot exceed `mutation.weight_limit`. Rates and
initializers can be retuned without resizing a world. Founders are not structurally
mutated. M2 does not enable sensor mutation (M3), body mutation, or sexual reproduction.

Evolving offspring receive legacy neural-scalar mutation followed by structural
edits. Scalar-control offspring receive structural edits followed by a full
neural-scalar redraw on the resulting topology, retaining topology and non-neural
genes. This control **can inherit and evolve topology**: it is neither a no-evolution
control nor the separate structural-null comparison planned for M8. Report both
cohorts' metric vectors across multiple seeds, not a ranking or a claim of useful
structure based only on gene counts.

### Telemetry protocol and observations

New output uses metrics schema **4**, `phase: 2`, and
`control: "randomized_at_birth_v2"` (the shared core protocol constant).
The reader explicitly supports legacy **schema 3 / phase 1 /
`control: "randomized_at_birth"`** records, without relabeling them as v2; legacy
records claiming any nonzero structural rate are rejected. Schemas 1 and 2, unknown
schemas, and other schema/phase/control combinations are rejected.

Schema 4 retains the earlier metric fields. `arena_usage` contains current element
counts for `Genes`, `Neurons`, `Synapses`, `Sensors`, and `Effectors` (`capacity`, `free_elements`,
`largest_free_block`, `live_blocks`); `spawn_failures` contains cumulative saturating
`u64` counters for `pool_full`, `genome_limit`, `arena_capacity`,
`arena_fragmentation`, `arena_block_limit`, and `invalid_genome`. These count actual
refused spawn attempts, not founder requests clamped to pool capacity.

Each cohort also has **`structural_mutations`**, with separate `remove_connection`,
`remove_neuron`, `toggle_connection`, `add_connection`, and `add_neuron` counters.
Each operator records cumulative saturating `u64` values for `attempted`, `applied`,
`no_candidate`, `genome_limit`, `scratch_limit`, and `innovation_exhausted`.
An attempt is counted only after a positive-rate Bernoulli gate succeeds. An applied
edit changes an **offspring candidate**, which can subsequently fail to spawn;
a refused edit can still result in an unedited successful birth. Neither applied
edits nor refused edits establish live complexity or a count of births.

Counts belong to each shell/world, not deterministic simulation state. The `--metrics`
path observes seeding, command spawns, and natural births, collecting structural edits
separately in both cohorts; plain runs use the unobserved stepping path.
Samples created outside a collected run encode unavailable observations as `null`,
not invented zeroes. Schema 3's absent `structural_mutations` fields likewise decode
to `null`. `diagnose` reports availability and mutation caps, scratch limits, and
innovation exhaustion separately from spawn pressure, and distinguishes arena capacity
from fragmentation; more energy does not resolve these limits. Exact `genome_variants`
remain distinct from species: species-cluster diagnostics await M4 clustering.

`SimParams.storage` reserves shared arena allowances and sets a default
`max_memory_bytes` of **100663296 (96 MiB) per world**. Larger native configurations
must explicitly raise that budget in their params JSON; the shell does not silently
resize storage or lower the requested founders. Construction failures and founder
undersupply are errors. The budget covers core-construction requests, not process
RSS: a paired run owns two separately budgeted worlds, with allocator/OS overhead,
metrics, and any shell snapshots/transports additional. The WASM shell similarly
exposes `Sim.storage_diagnostics()` JSON on demand with `arena_usage` and cumulative
`spawn_failures`; that envelope is unchanged. The separate
`Sim.structural_mutation_diagnostics()` method returns the five operator-counter
objects on demand. Both kinds of counters start at zero on world construction,
are isolated per world, and survive retuning. JSON requests can grow WASM memory,
so clients must refresh detached snapshot views. Snapshots and browser transports
are outside the core budget. This is not a browser resident-memory safety guarantee.

The committed files under `shells/native/tests/fixtures/` deliberately induce extinction,
exact-genome monoculture, or a cheap sustaining population. `structural.json` forces
all five structural operators for shell integration checks. These test telemetry and
diagnostics; they are not candidate simulation defaults.

For visual control checks, the web app's **heredity** selector labels the alternatives
**evolving** and **scalar control**, explaining that topology is inherited and may evolve
while neural scalars are redrawn. The JS `random_control` constructor and browser/URL
mode ID `randomized_at_birth` remain compatible; this mode ID is not the versioned
telemetry protocol. Changing heredity takes effect on
**reseed** because heredity mode is construction-time experiment configuration, not a
`SimParams` retune. Shared URL fragments retain the mode, so evolving and control tabs can
be opened with identical seed, founders, and params. The live **descendants** count excludes
generation-zero founders; a control with a few agents but zero descendants has not sustained
a randomized lineage.

Before adding anything to `sim-core`, check it against the invariants in `AGENTS.md`: no I/O, no allocation in the hot loop, deterministic. A crate that internally uses `HashMap` iteration order or platform floating-point math will silently break replay. This is a real constraint — audit dependencies in the sim core rather than assuming.

## Interesting seeds

```
seeds/
  0042-first-foragers.json      # params + seed + a note on what happens
  0117-predator-stable.json
```

Not really dependency management, but it belongs in the same habit. You will hit a run that does something remarkable and lose it otherwise. Because the sim is deterministic, a seed plus a params blob is a complete record.

## Windows

The Rust and Node toolchains are cross-platform; `scripts/setup.sh` is bash. Run it from
WSL or Git Bash, or follow the current steps in that script manually. Nobody has tried
this yet, so treat it as untested rather than supported.

## Not yet

**Docker / devcontainers.** Perfect reproducibility, but WASM toolchains in containers add friction, and file-watching across the container boundary is unpleasant. `rustup` and `nvm` reading pinned files gets you most of the benefit for none of the cost. Revisit if you're ever onboarding someone else.

**CI.** Worth adding once Phase 1's tests exist — running `cargo test`, `clippy`, and the cross-target hash comparison on every push is exactly what catches a determinism break the day it happens rather than three weeks later. It's the natural first task after Phase 1 closes.
