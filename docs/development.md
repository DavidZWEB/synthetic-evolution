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
glam = { version = "0.30", default-features = false, features = ["libm", "serde"] }
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
`.github/mcp.json`. `setup.sh` installs that version's Chromium binary.
`.github/mcp.json` is Copilot CLI's repository configuration; other agents use their
own MCP discovery locations, listed in `AGENTS.md`. Copilot CLI also discovers the
shared visual-check skill under `.github/skills/`.

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

The native shell runs an evolving world beside a same-seed, same-params random-brain
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

The committed files under `shells/native/tests/fixtures/` deliberately induce extinction,
exact-genome monoculture, or a cheap sustaining population. They test telemetry and
diagnostics; they are not candidate simulation defaults.

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
