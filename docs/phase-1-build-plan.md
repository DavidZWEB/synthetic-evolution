# Synthetic Evolution — Phase 1: The loop works

**Goal:** agents evolve to move toward food, within minutes of sim time.

**Spec sections to read** (`docs/synthetic-evolution-spec.md`)**:** §2 (all), §3.1–3.3, §4.1–4.2, §5.1–5.5, §7.2–7.4, §7.8.
Do not read §9 — it is post-roadmap and will only add noise.

Phase 1 deliberately hardcodes things that later phases make genetic. That is not technical debt; it is sequencing. What must *not* be hardcoded is anything in the forward-compatibility checklist below.

---

## Forward-compatibility checklist

These cost almost nothing now and are expensive to retrofit. Every one must be in place before Phase 1 is done, even though nothing uses them yet.

**Audited at M8, and now executable.** Ticking a box in a document does not stop the next
person deleting a field that is always `NULL`, so each hedge has a test that fails if it
goes. `sim-core/tests/forward_compat.rs` holds the five that had nothing pinning them and
names where the other six are already tested.

- [x] `positions`/`velocities` are `N*3`, z pinned to 0
- [x] `orientation` is a quaternion `N*4`, constrained to yaw about Z — **not** a scalar heading
- [x] `parentA` **and** `parentB` exist; `parentB` is `NULL_ID` always
- [x] `partOffset`/`partCount` exist; every agent has exactly 1 part at the origin
- [x] Sensor direction params are `(azimuth, elevation)` pairs; elevation clamped to 0, its mutation operator disabled
- [x] Turn effector takes a rotation axis, pinned to Z
- [x] Spatial hash is a triple-nested cell loop with Z range `[0,0]`
- [x] Chemo field is a 3D grid with depth 1
- [x] All world mutations go through a serde-serializable `Command` enum carrying `apply_at_tick`
- [x] Innovation counter is a field on `World`, not a `static`
- [x] Crossover function written and unit-tested, though nothing calls it

Rationale for each is in spec §9.1 and §3.4. Do not remove any as unused code.

---

## Tasks

### 1. Scaffold
Cargo workspace `synthetic-evolution`: `sim-core` (no I/O, no wasm-bindgen), `shells/native`, `shells/wasm`. Vite + Svelte 5 in `web/`. Azure `staticwebapp.config.json` with COOP/COEP headers at `app_location` (spec §7.7).
**Done when:** `cargo test` and `npm run dev` both run clean.

### 2. World state and pools
SoA arrays per spec §2.2a, fixed capacity, free list. Arenas for brains and genomes with per-agent offset/length. `SimParams` from §5.5, serde-serializable.
**Done when:** spawn/despawn 10k agents in a loop with zero allocations after warmup.

### 3. Spatial hash
Uniform grid, cell size = max sense radius, counting sort rebuild. Triple-nested neighbor iteration, Z pinned.
**Done when:** neighbor queries match a brute-force O(n²) reference on random populations.

### 4. Genome and CTRNN
Typed-gene list (§3.1). Fixed topology this phase: a set number of neurons, no add/remove-neuron mutation yet — but the *representation* is already the variable-length gene list. Euler-integrated CTRNN with `tau` and oscillator neurons (§3.2). Weight perturbation and reset mutations only (§3.3).
**Done when:** genome round-trips through serde; property test confirms no mutation orphans a sensor's target neuron.

### 5. Sensors and effectors
Hardcoded set: `vision_ray`, `chemo`, `interoception` (energy). Effectors: `thrust`, `turn`, `ingest`, and brain-gated `reproduce`. Perception phase writes to `sensorScratch`; effectors write to an intent buffer.
**Done when:** an agent with hand-written weights demonstrably chases a food gradient.

### 6. World and economy
Plants as simple non-brained entities. Chemo field with deposit/diffuse/decay. Closed energy economy per §5.1. Metabolic costs per §5.2 including the brain-complexity term. Death at zero energy. Asexual reproduction with spatial viscosity — **offspring spawn near the parent**.
**Done when:** energy conservation test passes over 10k ticks.

### 7. The tick
Wire the 11 steps in spec §2.4, in that order. Intents buffered. Births and deaths deferred to step 10, resolved in agent-index order.
**Done when:** golden-hash test passes twice in-process, and native and WASM builds agree.

### 8. Renderer
Instanced 2D canvas or minimal WebGL — circles, sized and tinted by state. Not Three.js yet, but **built to be presentable**, since sharing starts at Phase 2 (spec §8). Sim in a worker, `SnapshotTransport` interface with both SAB and transferable implementations, selected at runtime.
**Done when:** 5k agents render at 60fps with the sim at 1× and at 100×.

### 9. Instrumentation
Population and mean-energy time series. Agent inspector: click an agent, see genome and live neuron activations. Seed URL encoding. Speed control including pause.
**Done when:** you can watch a run, pause, click an agent, and read its brain.

### 10. Headless telemetry
`--metrics run.jsonl` on the native shell emitting one sample line per interval (spec §7.9). A `diagnose` subcommand mapping metric signatures to §10 failure modes. A random-brain control population runnable alongside for comparison.
**Done when:** a 500k-tick headless run produces a metrics file, and `diagnose` correctly identifies a deliberately induced extinction and a deliberately induced monoculture.

---

## Acceptance

**Mechanical:** golden hash, energy conservation, no-alloc, cross-target agreement, brute-force neighbor parity — all passing.

**Judgment (human, not self-certified):** starting from random genomes, a population reliably evolves food-seeking within minutes of sim time, across at least three different seeds. Compare side by side against a random-weights control population — if the two are indistinguishable, this phase is not done regardless of what the tests say.

If food-seeking doesn't emerge, it is almost always one of: metabolic cost too low so idling isn't fatal, energy input too high so nothing is scarce, or mutation rate too high (error catastrophe). See spec §10 before changing code.
