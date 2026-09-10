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
mutated. Sensor operators are separately opt-in below; body mutation and sexual
reproduction remain unavailable.

Evolving offspring receive legacy neural-scalar mutation followed by sensor edits
and then the five neural structural edits. Scalar-control offspring receive those
same sensor and neural structural edits followed by a full neural-scalar redraw on
the resulting topology, retaining sensors and other non-neural genes. This control
**can inherit and evolve topology and sensors**: it is neither a no-evolution
control nor the separate structural-null comparison planned for M8. Report both
cohorts' metric vectors across multiple seeds, not a ranking or a claim of useful
structure based only on gene counts.

### Opt-in sensors and founder composition (M3)

Both shipped rates under **`SimParams.mutation.organs` are zero**. Missing fields in
older params documents or URLs retain those defaults. For example:

```json
{
  "mutation": {
    "organs": {
      "remove_sensor_rate": 0.001,
      "add_sensor_rate": 0.001,
      "vision_weight": 1.0,
      "chemo_weight": 1.0,
      "energy_weight": 1.0,
      "neuron_bias": 0.0
    }
  }
}
```

These are protocol examples, not recommended rates. Removal runs before addition,
and both run before the five neural structural operators. Removing a sensor keeps
its target neurons and connections. Addition chooses among current vision,
food-chemo, and energy-interoception modalities, adding one fresh target neuron per
channel and one sensor, **without automatic wiring**. Neural operators can subsequently
wire those neurons. Rates must be in `[0, 1]`, modality weights finite and nonnegative,
and `neuron_bias` finite. Enabled addition requires a positive total modality weight.
These rates, weights, and bias may be retuned on a running world.

Founder composition is separately configurable at construction. The following opt-in
partial JSON works with `--params params.json`, the WASM constructor, or the existing
shared URL `params` fragment:

```json
{
  "sensing": {
    "vision_rays": 0,
    "chemo_sensors": 1,
    "energy_sensors": 0
  },
  "brain": {
    "hidden_neurons": 0,
    "oscillators": 0,
    "connections_per_target": 1
  }
}
```

This has **seven neurons and four connections**: three chemo inputs and the four
required effector outputs (thrust, turn, ingest, reproduce). It retains four body and
three meta genes, for 23 total genes including the sensor and effectors. It is a
minimal candidate to investigate, **not evidence of viability or a new shipped
founder**. Dense defaults remain unchanged: `vision_rays: 3`, `chemo_sensors: 1`,
`energy_sensors: 1`, six hidden neurons, two oscillators, and
`connections_per_target: null`.

`null` or an omitted connectivity field means the original full dense topology.
An integer `k` chooses `min(k, sources)` distinct inputs per hidden/output target;
`0` creates no connections. The sparse template is chosen once per world after plants
are seeded and shared by every founder, while founder neural scalars are still
drawn individually. Sensor counts, hidden/oscillator counts, and connectivity are
construction-time frozen: changing them requires a new world/reseed, not
`Sim.set_params`. Counts and connectivity must be unsigned integers; resulting
genomes must fit the configured per-genome caps and construction budget. No full
parameter editor or preset selector is introduced.

### Genetic-distance foundation (M4)

`sim_core::distance::between(a, b, &params.distance)` compares two validated,
canonically sorted genomes without allocating or consuming RNG. Its result exposes
the raw components and weighted value. It does not assign species or change
reproduction, energy, or simulation trajectories itself; World uses this distance
for the observational classification described below.

The existing native/WASM parameter JSON accepts:

```json
{
  "distance": {
    "disjoint_coefficient": 1.0,
    "excess_coefficient": 1.0,
    "weight_coefficient": 0.4
  }
}
```

Coefficients must be finite and nonnegative. Omitted values retain these defaults.
World freezes them with its species policy at construction, even when classification
capacity is zero; changing them requires a new world rather than a retune. Standalone
Rust callers validate genomes with `genome::validate` and coefficients with
`DistanceParams::validate` before calling `between`.

Distance aligns innovation-bearing genes within each kind and excludes body/meta
traits from normalization. Matching disabled connections still contribute weights.
Other scalar/binding differences add no terms. For a two-neuron, one-connection
example at the starting coefficients, toggling that connection measures 0, deleting
it measures 1/3, and recreating the identical wiring with a fresh ID measures 2/3
against the retained original. These are historical-marker effects, not evidence of
new species or useful divergence. Ecological threshold and deletion-default calibration
remain pending. The distance function does not emit observations; World classification
provides the species telemetry described below.

### Species classification (M4)

Native `--params` and the WASM constructor accept this partial JSON:

```json
{
  "species": {
    "capacity": 256,
    "threshold": 0.5
  }
}
```

These are the shipped construction defaults. Threshold is a finite positive `f64`;
0.5 is a provisional measurement scale, not a calibrated biological boundary.
Each representative reserves up to `storage.max_genes` genes, and distance coefficients
use `DistanceParams` validation. All species fields and distance coefficients are
frozen for the world's lifetime. Classification consumes no RNG and does not change
reproduction, metabolism, feeding, or other ecological decisions.

World classifies each successful founder or birth once, stores its historical species
ID, and retires its membership once on death, including command-driven admissions.
Storage or ID exhaustion leaves a successfully admitted agent unclassified; it never
refuses the birth. Zero capacity disables classification and leaves all admissions
unclassified with the `NULL_ID` sentinel. Unclassified agents are not a species.

Standalone callers can still construct
`sim_core::species::Classifier::try_new(capacity, max_genes, threshold, coefficients,
max_memory_bytes)`. Outside World, those callers own the exactly-once `classify` and
`remove_member` bookkeeping. Failed classification returns an explicit `Unclassified`
reason without consuming an ID or member. Unclassified individuals stay unclassified,
while their descendants can be independently assigned.

Representatives are immutable copies. They survive the original agent and are
released only when membership reaches zero; that departure returns `Extinct`
once. Retired IDs never return, even if a storage slot is reused. Active iteration
is in ascending historical-ID order. There is no retained history buffer.

`estimated_construction_bytes` includes full per-slot gene reservations plus all
metadata. The approved 256-slot / 1,024-gene policy requests **10,492,936 bytes**
with the current layouts. World charges this to its existing core budget before
allocation rather than silently increasing that budget. The current default world's
total construction request is **88,613,076 bytes**, including this classifier component
and the M5 birth-identity arrays,
within the unchanged **100,663,296-byte (96 MiB)** per-world limit. These are requested
construction bytes, not process RSS or browser resident memory. Zero capacity requests
no classifier buffers and reports unclassified capacity outcomes. The constructor
validates portable buffer limits and propagates host reservation failures.

To reproduce the bounded high-churn exercise:

```bash
cargo test --release -p sim-core --test species full_capacity_churn_reuses_storage_without_reusing_ids -- --nocapture
```

It retires and recreates 2,048 representatives at 256 active species, checking IDs
and membership totals and reporting time/reserved memory without a timing threshold.
Its three-gene inputs exercise infrastructure, not worst-case genome comparison cost
or ecological adaptation.

The observational-integration regression compares capacity 1 against disabled
classification for seeds **7, 42, and 99**, in both evolving and randomized-at-birth
modes. It exercises actual births and compares observed stepping against plain
stepping for 120 ticks:

```bash
cargo test -p sim-core classification_and_observation_leave_ecology_unchanged_across_seeds_and_modes
```

The test-only `ecology_hash` masks classification and lifetime identities while retaining ecological state
and RNG, and must agree after seeding and every step. Full state hashes need not agree:
they include per-agent species IDs and persistent birth/parent identities. The integration's organ-control
golden change reflects those labels, not changed heredity or ecology; expanding
classifier-state hash coverage is a separate change with its own reference updates.
Shell event counters are observations and are not hashed. This regression establishes
mechanical isolation, not calibrated clusters or ecological success.

### Persistent birth identities (M5 foundation)

World assigns every successful admission a monotonic 64-bit `BirthId`. This identifies
an individual, unlike a reusable agent slot or a species shared by many organisms.
IDs are local to one World and start at zero. Pool/arena refusals consume no ID;
exhaustion yields explicit unavailable identity without refusing the ecological birth.

Inspection adds `birth_id`, `parent_birth_a`, and `parent_birth_b`, encoded as exact
decimal strings or null. Never convert these strings to JavaScript `Number`.
The unavailable u64::MAX sentinel is null, not a decimal string. The three identity
arrays reserve 24 bytes per agent slot (120,000 bytes at the default 5,000 slots)
inside the unchanged 96 MiB core budget; they do not widen the render snapshot.

The live parent's identity is captured at admission, before child-slot allocation
could reuse a dead parent slot. `SpawnSpec.parent_a` still means the current parent
slot at that moment, not a historical ancestry handle. Dead/unknown parents retain
unavailable persistent references; they must not be guessed from a later occupant.
The existing `parent_a`/`parent_b` slot fields stay separate and unchanged. Ordinary
asexual reproduction leaves `parent_birth_b` empty.

An ID can outlive the corresponding live slot without retaining that organism's
genome or phenotype. This foundation is not a history archive, ancestry viewer, or
resumable checkpoint. Retention, pruning, and persistent graph export remain later
M5 decisions.

### Telemetry protocol and observations

New output uses metrics schema **7**, `phase: 2`, and
`control: "randomized_at_birth_v3"` (the shared core protocol constant).
Species classification does not change heredity, so the control protocol is unchanged.
The reader explicitly supports **schemas 6 and 5 / phase 2 /
`control: "randomized_at_birth_v3"`**, **schema 4 / phase 2 /
`control: "randomized_at_birth_v2"`** and **schema 3 / phase 1 /
`control: "randomized_at_birth"`** (v1), without relabeling them as v3.
Schemas 3 and 4 reject nonzero organ mutation rates or nondefault M3 founder
fields (`chemo_sensors != 1`, `energy_sensors != 1`, or non-null
`connections_per_target`). Schema 3 also rejects every nonzero neural structural
rate and any claimed structural observations. Schema 4 may carry the five neural
operator observations but cannot claim measured sensor counts, even zeros.
Schema 5 retains its organ observations and nondefault M3 founder configurations.
Schemas 1 and 2, unknown schemas, and all other schema/phase/control combinations
are rejected.

Schemas 3–5 predate World classification. Their absent species metrics stay `null`,
not invented zeroes. The reader rejects species observations or any explicit
`params.species` field in those schemas; only a genuinely omitted field is internally
decoded with classification disabled to preserve the metadata's meaning. Historical
buffer selection is independent of that normalization and is owned by `LayoutEra`.
Schemas 6 and 7 require explicit species capacity, threshold, and all distance coefficients
in the header, rather than silently filling missing classification metadata.

The native reader maps supported wire schemas to core buffer inventories:

| Native schema | Core `LayoutEra` | Additional buffers |
|---|---|---|
| 3-5 | `BeforeSpecies` | Neither species representatives nor birth identities |
| 6 | `Species` | Species representatives only |
| 7 | `BirthIdentities` | Species representatives and three lifetime identity arrays |

`SimParams::validate_for_layout(era)` replaces the era-specific pre-birth validator.
The core owns which buffers existed; it does not know native schema numbers. This
validation changes neither parameters nor recorded budgets and does not invent
observations. The reader still independently rejects unsupported feature claims.

Historical validation is not permission to construct an old runtime or bypass
the current memory ceiling: `World::new` always uses `LayoutEra::CURRENT`.
Selecting an era neither parses nor migrates checkpoint data; M7 still requires
explicit rejection of incompatible formats. Schema 7 marks the identity-aware runtime/storage contract, not
the addition of per-organism history records to these population samples.

Schema 7 retains the earlier metric fields. `arena_usage` contains current element
counts for `Genes`, `Neurons`, `Synapses`, `Sensors`, and `Effectors` (`capacity`, `free_elements`,
`largest_free_block`, `live_blocks`); `spawn_failures` contains cumulative saturating
`u64` counters for `pool_full`, `genome_limit`, `arena_capacity`,
`arena_fragmentation`, `arena_block_limit`, and `invalid_genome`. These count actual
refused spawn attempts, not founder requests clamped to pool capacity.

Each current cohort also contains **`species`**:

```json
{
  "populations": [{"species_id": 7, "population": 12}],
  "unclassified_population": 2,
  "events": {
    "created": 8,
    "extinct": 7,
    "unclassified_capacity": 2,
    "unclassified_id_exhausted": 0,
    "unclassified_genome_too_large": 0,
    "unclassified_member_count_exhausted": 0,
    "unclassified_storage": 0
  }
}
```

This illustrative sample describes 14 living agents, not 14 classified agents.
Rows contain positive populations, unique non-NULL IDs in ascending historical-ID
order, and no extinct representatives. The reader checks that classified plus
unclassified populations equal the cohort population and that population and active
species counts fit configured capacities. Historical IDs can exceed the active
capacity because retired IDs are never reused. An empty or extinct current world
has an empty row array and zero unclassified population, not unavailable state.

Species population data is authoritative current World state even when stepping was
not observed. Only `events` is nullable for unobserved sampling; collected runs
accumulate its seven saturating `u64` counters from seeding and every step, separately
for evolving and scalar-control worlds. Unclassified event counts describe admitted
agents that lacked classification, not spawn refusals or current unclassified totals:
those agents may subsequently die. Species creation/extinction events likewise
describe classifier membership transitions, not evidence of adaptive success.

Each cohort also has **`structural_mutations`**, with separate `remove_connection`,
`remove_neuron`, `toggle_connection`, `add_connection`, and `add_neuron` counters,
plus nullable **`remove_sensor`** and **`add_sensor`** counters.
Each operator records cumulative saturating `u64` values for `attempted`, `applied`,
`no_candidate`, `genome_limit`, `scratch_limit`, and `innovation_exhausted`.
An attempt is counted only after a positive-rate Bernoulli gate succeeds. An applied
edit changes an **offspring candidate**, which can subsequently fail to spawn;
a refused edit can still result in an unedited successful birth. Neither applied
edits nor refused edits establish live complexity or a count of births.

Event and refusal counters belong to each shell/world, not deterministic simulation
state. The `--metrics` path observes seeding, command spawns, natural births, and
species extinctions, collecting structural edits and classification transitions
separately in both cohorts; plain runs use the unobserved stepping path.
Samples created outside a collected run encode unavailable observations as `null`,
not invented zeroes. Schema 3's absent `structural_mutations` fields likewise decode
to `null`. Schema 4's missing sensor-counter fields decode individually to `null`;
they are not inferred from neural counts, zero configured rates, or later samples.
Unknown historical sensor totals stay unknown even if later edits are observed.
Current shell observers instead begin with measured zeroes for all seven operators.
`diagnose` reports availability separately for each organ operator and cohort, and
reports mutation caps (including sensor and vision-ray caps), scratch limits, and
innovation exhaustion separately from spawn pressure, and distinguishes arena capacity
from fragmentation; more energy does not resolve these limits. It also reports the
latest active-species and unclassified populations, with event-count availability
separate for each cohort, and reports classification capacity, ID, representative
storage/genome, and member-count pressure separately from spawn pressure. Labels are
observational, not adaptive success. Exact `genome_variants` remain distinct from
species: the monoculture heuristic still uses exact genomes, not a new species-based
judgment.

`SimParams.storage` reserves shared arena allowances and sets a default
`max_memory_bytes` of **100663296 (96 MiB) per world**. Larger native configurations
must explicitly raise that budget in their params JSON; the shell does not silently
resize storage or lower the requested founders. Construction failures and founder
undersupply are errors. The budget covers core-construction requests, not process
RSS: a paired run owns two separately budgeted worlds, with allocator/OS overhead,
metrics, and any shell snapshots/transports additional. The WASM shell similarly
exposes `Sim.storage_diagnostics()` JSON on demand with `arena_usage` and cumulative
`spawn_failures`; that envelope is unchanged. The separate
`Sim.structural_mutation_diagnostics()` method returns the same counter object,
extended with nullable `remove_sensor` and `add_sensor` fields.
`Sim.species_diagnostics()` returns the same `populations`,
`unclassified_population`, and `events` envelope as native species metrics, with
measured event counters. `Sim.species_count()` and `Sim.unclassified_population()`
provide the current scalar counts without allocating a diagnostics JSON response.
All current shell counters start at zero on world construction, are isolated per
world, and survive retuning. JSON requests can grow WASM memory,
so clients must refresh detached snapshot views. Snapshots and browser transports
are outside the core budget. This is not a browser resident-memory safety guarantee.

The committed files under `shells/native/tests/fixtures/` deliberately induce extinction,
exact-genome monoculture, or a cheap sustaining population. `structural.json` forces
all five structural operators for shell integration checks. These test telemetry and
diagnostics; they are not candidate simulation defaults.

For visual control checks, the web app's **heredity** selector labels the alternatives
**evolving** and **scalar control**, explaining that topology and sensors are inherited
and may evolve while neural scalars are redrawn, including newly added sensor-target
neurons. Inspector activation and genome lengths can therefore differ between parent
and child. The JS `random_control` constructor and browser/URL
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
