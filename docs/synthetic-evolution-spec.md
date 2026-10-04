# Synthetic Evolution — Design Spec v0.1

**Status:** draft for iteration. Opinionated defaults are chosen throughout so we have something concrete to argue with. Resolved choices are marked **DECIDED**; what remains open is in §11.

---

## 1. Goal and non-goals

**Goal:** Synthetic Evolution is an open-ended artificial life simulator where neural-network-brained organisms evolve under implicit selection pressure, and where non-trivial ecological behaviors (predation, herding, camouflage, signaling, kin altruism, symbiosis) appear without being programmed in.

**Non-goals for v1:**
- Articulated bodies / rigid-body physics (the Karl Sims direction). Out of scope for v1 — it consumes all your compute budget in physics rather than in evolution. Note that *rigid* multi-part morphology is in scope at Phase 5 (§3.5); it is joints and torques specifically that are deferred (§9.2).
- Explicit fitness functions or training objectives. Fitness is survival and reproduction, full stop.
- Photorealism. Legibility beats beauty.

### The central design bet

Interesting behavior does not emerge from having a neural network. It emerges from the **environment and the economy**. A GA over neural nets in a poorly designed world produces agents that jitter in place forever. Most of the design effort below is on the world, not the brain.

Three specific things do most of the work:

1. **A closed energy economy with an autotroph base.** Predation cannot evolve unless there is an energy pyramid to climb and unless attacking has a real cost.
2. **Spatial viscosity** (offspring spawn near parents). This makes your neighbors your relatives, which makes kin selection operate, which is what makes altruism and honest signaling evolutionarily stable. Without it, communication will never evolve — a signal that only helps the receiver is selected against.
3. **Sensors and effectors that are themselves genetic.** If the input vector is fixed at design time, you have capped the space of possible organisms on day one.

---

## 2. Architecture

### 2.1 Hard separation of sim and render

```
┌────────────────────────────────┐   ┌─────────────────────────────┐
│  Worker thread                  │   │  Main thread                 │
│                                 │   │                              │
│  Sim core (Rust → WASM)         │   │  Svelte UI shell             │
│   · fixed timestep              │──▶│  WebGL2 renderer             │
│   · seeded PRNG                 │SAB│   · Instanced quads          │
│   · spatial hash                │   │   · reads snapshot buffer    │
│   · brain eval                  │   │  Charts / inspector          │
│   · genetics                    │◀──│  Command queue               │
└────────────────────────────────┘   └─────────────────────────────┘
```

Non-negotiable properties:

- **The sim never touches the DOM or rendering libraries.** It owns world state outright and publishes a narrow render snapshot into a leased triple-buffered `SharedArrayBuffer` (see §2.2), with a transferable-buffer fallback (§7.7). The renderer reads whatever the latest complete snapshot is.
- **Sim rate is decoupled from frame rate.** Fixed timestep (e.g. 60 ticks/sec of sim time). The renderer samples the latest completed snapshot rather than requiring every tick to be displayed, even at 1×. Headless mode disables the renderer entirely and runs as fast as the CPU allows overnight.
- **Fully deterministic given a seed.** Same seed + same params = byte-identical run. This is worth real effort: without it you cannot debug an emergent behavior you saw once, and you cannot share interesting worlds as a seed + param blob.
- The Rust sim core is a black box behind a narrow stepping and snapshot interface. Native and WASM shells own I/O and scheduling; neither the renderer nor a shell reaches into the tick. Basic manual full-world checkpoint save/load arrives in late Phase 2 (§7.10); automatic checkpoint scheduling and the larger overnight-run workflow remain Phase 7 work (§8).

### 2.2 Data layout

Struct-of-arrays, typed arrays, no per-agent objects in the hot loop. There are **three distinct buffers** here and it's worth keeping them separate in your head, because they have different owners, different lifetimes, and different sizes.

#### (a) World state — owned by the worker, never leaves it

The authoritative simulation state. Mutated in place every tick.

```
positions:    Float32Array(N * 3)   // z pinned to 0 in V1
velocities:   Float32Array(N * 3)
orientation:  Float32Array(N * 4)   // quaternion; V1 constrains to yaw about Z
energy:       Float32Array(N)
energyResidual: Float64Array(N)      // sub-f32 stock; paired with energy
health:       Float32Array(N)
age:          Uint32Array(N)
speciesId:    Uint32Array(N)
signature:    Float32Array(N * 3)   // evolvable "color" — see §4.2
size:         Float32Array(N)
alive:        Uint8Array(N)
parentA:      Uint32Array(N)        // legacy parent slot, not a persistent identity
parentB:      Uint32Array(N)        // NULL_ID in V1 — see §3.4
birthId:      BigUint64Array(N)     // stable world-local organism identity
parentBirthA: BigUint64Array(N)     // captured at admission; u64::MAX means unavailable
parentBirthB: BigUint64Array(N)     // empty during asexual reproduction
gridCell:     Uint32Array(N)        // spatial hash bucket, rebuilt per tick
brainOffset:  Uint32Array(N)        // index into the brain arena
genomeOffset: Uint32Array(N)        // index into the genome arena
partOffset:   Uint32Array(N)        // index into the parts arena — always 1 part in V1
partCount:    Uint32Array(N)
```

Two of these look like over-engineering for a 2D sim with spherical agents, and are deliberate. `orientation` as a quaternion rather than a scalar `heading` float, and the `partOffset`/`partCount` indirection rather than treating an agent as a single sphere, are the two hedges that make the eventual move to volumetric 3D with articulated bodies survivable. See §9 for why these specifically.

Fixed-capacity pools with a free list. Never allocate in the loop. Brains and genomes are variable-size, so they live in separate arenas with per-agent offset/length indices.

**DECIDED: Phase 2's variable-length allocator uses address-ordered first fit with
adjacent-free-span coalescing.** Element storage and free-span metadata are reserved
at construction, with an explicit maximum number of simultaneous nonempty blocks.
There is no compaction, relocation, or backing-buffer growth in allocation/release.
Element reset copies a cached default value rather than invoking element
constructors in the hot loop.

An empty allocation succeeds without consuming space or a block. Nonempty requests
report block-limit exhaustion, insufficient total free space, or fragmentation
separately, in that precedence order; a refusal changes neither live data nor
allocator state. Freeing an empty block is a no-op. All other releases must return
an exact live block owned by the arena. Construction checks portable element and
metadata byte bounds and propagates host reservation failure.

Fragmentation can refuse a request despite sufficient total free space; diagnostics
must expose that outcome rather than silently growing or dropping genome data.
**DECIDED: the initial world-storage policy is conservative and configurable.**
`SimParams.storage` gives pooled allowances of 284 genes, 28 neurons, 240 synapses,
5 sensors, and 4 effectors per agent-pool slot. These multiply `max_agents` to size
shared arenas; they are not per-organism strides and do not depend on founder
composition. The per-genome limits are 1,024 genes, 128 neurons, 1,024 connections
(including disabled ones), 32 sensors, 32 vision rays, and 4 effectors.

The default core-construction budget is 96 MiB per world. Checked layout accounting
must bound construction requests for arenas, metadata, templates, scratch, pools,
fields, and spatial indices before constructing the world. Larger configurations
must explicitly raise the budget; never lower population or expand storage silently.
This is not a host-RAM guarantee: shell snapshots/transports, allocator/OS overhead,
and a second control world are additional. Storage configuration is fixed for a
world's lifetime, because changing it would require rebuilding reserved buffers.
Live retuning validates scalar values and the unchanged storage policy against the
already-allocated grids. It must not charge a sensing-radius reduction for the finer
grid that a new world would construct; startup budgeting and live retuning are
different boundaries.

A spawn claims every constituent arena before claiming a pool identity. Refusal
unwinds prior claims without changing live data, free-span bookkeeping, pool order,
or incarnations. A refused birth leaves parent energy untouched. Random draws used
to prepare an attempted birth/founder remain consumed; no RNG rewind or retry is
introduced. Seeding stops at the first storage refusal and reports the placed count.
This storage work introduces no new structural mutations or innovation-ID policy.

M4 additionally charges the world-owned species classifier's full representative
reservations and metadata to this same construction budget (§3.4). The default
256-slot classifier at the 1,024-gene cap requests 10,492,936 bytes; it does not
increase the 96 MiB ceiling or reduce ecological pool allowances to make room.

Refusals are explicit results at spawn boundaries and optional monomorphic callbacks
for tick/command spawns. Shells can count them without allocating in a tick; ordinary
headless stepping uses a no-op observer. Arena usage is sampled on demand. Observer
counters do not affect simulation state or deterministic hashing.

`energy` remains the compact, hot-path value. Every agent and plant also owns an
`energyResidual`: compensated storage for a quantity below the current `Float32`
resolution. Energy operations act on the pair. This prevents long runs from creating,
destroying, or permanently blocking energy when a cost or transfer is smaller than one
endpoint's `Float32` ULP; operations account for the delta actually represented at the
remaining `Float64` precision floor. Residuals are authoritative state and participate
in deterministic hashing, but the render snapshot needs only the rounded visible value.

#### (b) Render snapshot — the SharedArrayBuffer, leased and triple-buffered

A narrow projection of (a), containing only what's needed to draw a frame:

```
agents: positions, orientation, size, signature, alive, speciesId, partOffset, partCount, incarnation
plants: positions, energy
```

Plant energy is the current stock, used to show whether a plant is full or depleted (§5.1); it is not a history. Plant positions change when a plant dies and reseeds, so they travel with every frame too. There is no agent energy, no genomes, and no brain state. This buffer is written once per tick and read by the main thread at whatever rate it happens to be rendering. Keeping it small matters: at 50k agents you're copying it 60 times a second, and every field you add is bandwidth you don't get back.

`incarnation` changes whenever a pool slot is allocated. A slot index alone is not an agent identity because the free list reuses it; `(index, incarnation)` lets a click-driven inspector reject a response for a replacement born after the displayed frame.

Three frames are required because the renderer must lease one while it issues uploads. The worker publishes into either remaining frame and only reclaims an older published frame after the replacement is complete. An unleased two-frame flip can overwrite the renderer's live typed-array view after two worker publications, producing a frame assembled from different ticks.

Inspector data (full genome, live brain activations, lineage) is **pulled on demand** for the one selected agent via a request on the command queue, not streamed for everybody.

Everything crossing into the sim — including UI actions like placing food or spawning an agent — goes through a single serde-serializable `Command` enum stamped with an `apply_at_tick`. Replay, batch scripting, and the multi-client path in §9.5 all fall out of that one choice.

#### (c) Perception buffer — inputs to the neural networks

**This is the part the field list above does not answer, and it's the important distinction:** agents do not read the world-state arrays directly. Nothing in (a) is a network input.

Each tick, a **perception phase** runs before brain evaluation. For each agent, it walks that agent's sensor genes and executes them against the world:

```
for each agent:
  for each sensor gene:
    vision_ray  → spatial-hash raycast → writes [distance, sig.r, sig.g, sig.b]
    chemo       → sample pheromone field at position → writes [conc, grad.x, grad.y]
    kin_sense   → nearest-neighbour query + genetic distance → writes [similarity]
    interocept  → reads own energy/age → writes [value]
  → sensorScratch[brainOffset .. +sensorCount]
```

The output lands in a per-agent scratch buffer that is then clamped into the CTRNN's input neurons by the `target` neuron ID on each sensor gene. So the world-state arrays are the **substrate that sensors query**, not the input vector itself.

This indirection is doing real work, and it's the reason §3.1 binds sensors to neurons by ID rather than by array index:

- **The input vector is per-agent and variable-length.** An agent with three eyes and a nose has a different input width than its cousin with one eye. There is no global input schema.
- **Sensors are lossy and local by construction.** An agent cannot perceive the global `energy` array; it perceives one number from one sensor with a limited range and an evolvable field of view. If agents could read world state directly you'd have accidentally given every organism omniscience, which collapses most of the interesting selection pressure — camouflage is worthless against an agent that reads the array.
- **Losing an organ is survivable.** Delete a sensor gene and the brain is still coherent; the target neuron just stops receiving input.

Practically, perception is also where your time goes. It's the only phase that touches the spatial hash, and it will be 60–80% of tick cost once vision is in. Budget for it: cap rays per agent, and make ray count a metered metabolic cost (§5.2) so evolution has to pay for its own compute.

### 2.3 Spatial partitioning

Uniform grid spatial hash, cell size = max sensing radius. Rebuilt each tick (cheap with a counting sort). Sensing is the O(n²) trap; everything else is nearly free by comparison.

**DECIDED: 2D simulation plane.** The current client renders instanced quads with WebGL2; the Three.js presentation upgrade arrives with Phase 5 (§8). V1 continues simulating on a plane, keeping the search space small enough that evolution can converge in an afternoon rather than a week.

This is explicitly a staging decision, not a permanent one. Full volumetric 3D with non-spherical morphology is a stated long-term target (§9), and several choices elsewhere in this spec — 3-component position vectors, quaternion orientation, azimuth/elevation sensor params, the parts indirection in §2.2 — exist specifically so that transition is an unlock rather than a rewrite. The spatial hash should be written as a triple-nested cell loop with the Z range pinned to `[0,0]` in V1 for the same reason.

---

### 2.4 The tick

Order of operations within one `step()`. This is normative — changing it changes both behavior and reproducibility.

```
 1. Rebuild spatial hash                    (counting sort over gridCell)
 2. Perception    → sensorScratch           (§2.2c; reads world, writes scratch)
 3. Brain         → CTRNN integration       (all agents, from previous activations)
 4. Effectors     → intent buffer           (no world mutation yet)
 5. Movement      → integrate velocity, position
 6. Collision     → sphere overlap resolution
 7. Interaction   → bite damage, ingest, grab
 8. Plants/fields → grow, plant deaths and reseeds, deposit, diffuse, decay
 9. Metabolism    → charge costs, update energy
10. Births/deaths → resolve deferred, in agent-index order
11. tick += 1
```

Four properties this ordering exists to guarantee:

- **Brains read a consistent world.** Step 3 uses every agent's *previous* activations, so no agent's decision depends on where it sits in the array. Sequential in-place updating would make behavior an artifact of pool index.
- **Intents are buffered, not applied.** Step 4 writes intents; steps 5–7 apply them. Otherwise an agent acting early sees a different world than one acting late.
- **Births and deaths are deferred to step 10** and resolved in agent-index order. Mutating the pool mid-tick makes free-list allocation depend on iteration order, which is the fastest way to lose determinism.
- **Never iterate a hash map to drive simulation.** Step 7 in particular must resolve in agent-index order, not spatial-hash bucket order. Rust's `HashMap` iteration order is deliberately nondeterministic; use sorted indices or a `Vec`. This is the single most likely source of a determinism bug in this codebase.

---

## 3. Genome

### 3.1 Representation

The genome is a **variable-length list of typed genes**, not a flat weight vector. This is the single most important representational choice, because it lets sensors, effectors, neurons, connections, and body parameters all evolve through the same machinery.

```ts
type Gene =
  | { kind: 'neuron';     id: InnovationId; bias: f32; tau: f32; activation: ActivationFn }
  | { kind: 'connection'; id: InnovationId; from: InnovationId; to: InnovationId;
      weight: f32; enabled: bool }
  | { kind: 'sensor';     id: InnovationId; modality: Modality; params: f32[];
      target: InnovationId }
  | { kind: 'effector';   id: InnovationId; action: ActionType; params: f32[];
      source: InnovationId }
  | { kind: 'body';       trait: BodyTrait; value: f32 }
  | { kind: 'meta';       trait: 'mutationRate' | 'crossoverRate' | ...; value: f32 }
```

`InnovationId` is a monotonic counter (NEAT-style). Two genomes' shared ancestry is visible by matching IDs, which gives you both meaningful crossover and a cheap genetic distance metric. It is a **field on `World`, not a `static`** — see §7.2, since a process must be able to hold more than one world.

Runtime admission additionally requires unique innovation IDs across gene kinds and
at most one retained connection per ordered neuron pair, including disabled edges.
Successful external spawns advance the counter past supplied IDs; failed spawns do
not. Checked reservations never issue `NULL_ID` or wrap. Generic offline crossover
still operates on coherent gene lists, not guaranteed runtime-admissible offspring;
its eventual sexual pathway must reconcile or reject admission conflicts rather
than inheriting an unreviewed M2 recombination policy.

Sensors and effectors bind to neurons **by ID**, not by index. That indirection is what makes the interface dynamic — an agent can lose an eye and its brain is still coherent.

### 3.2 Brain

**CTRNN** (continuous-time recurrent neural network). Each neuron has a time constant `tau` and integrates rather than being a pure function:

```
dy_i/dt = (1/tau_i) * (-y_i + Σ_j w_ij * σ(y_j + b_j) + I_i)
```

Why CTRNN over a feedforward MLP: it gives you memory, oscillation, and temporal dynamics for free, from the same weight mutations. Locomotion gaits, patience, timing-based hunting strategies, and rhythmic signaling all fall out of tau evolution. A feedforward net requires you to hand-build a memory mechanism, and it will never produce a gait.

Add a small number of always-present **oscillator neurons** with evolvable period. They're a cheap scaffold — evolution finds them fast and builds on them.

Evaluation: Euler integration, one step per tick. Topologically sorting is pointless (recurrence is the point), so just evaluate all neurons from the previous timestep's activations. This is naturally vectorizable.

### 3.3 Mutation operators

| Operator | Default rate | Notes |
|---|---|---|
| Weight perturbation | 0.025 per connection | Gaussian, σ evolvable; ~6 of Phase 1's 240 connections per birth |
| Weight reset | 0.0015625 per connection | Uniform resample; ~0.375 per Phase 1 birth |
| Neuron scalar perturbation | 0.00625 per neuron | Bias, tau, and oscillator period; ~0.175 of Phase 1's 28 neurons per birth |
| Add connection | 0.05 since M8 | Existing neurons, including recurrence/self-edges |
| Add neuron | 0.02 since M8 | Split an enabled connection |
| Add oscillator | 0.01 since M8 | One oscillator plus one outgoing connection |
| Disable/enable connection | 0.02 since M8 | Retain identity and weight |
| Remove connection | 0.02 since M8 | Physical deletion, enabled before distance calibration (§3.4) |
| Remove neuron | 0.01 since M8 | Remove an eligible neuron and incident edges; same caveat |
| Add sensor | 0.001 since M8 | Configured modality mixture and fresh target neurons |
| Remove sensor | 0.001 since M8 | Retain target neurons and wiring |
| Add effector | 0.01 | |
| Mutate body trait | 0.1 | |
| **Gene duplication** | 0.005 | Duplicate a subgraph with fresh IDs |
| Mutate meta-genes | 0.05 | Mutation rates evolve |

**DECIDED (M8):** every structural and sensor rate in the table ships enabled; the
full set was viable on seeds 42/117/314 over 200k ticks (1,315–1,645 agents; mean
genome 26–34 genes from the 25-gene founder). Deletion ships before D4's
distance/threshold calibration, accepting that remove-and-re-add marker turnover can
split species labels without real divergence; read species counts with that in mind.
Rates live under `mutation.structural` and `mutation.organs` and are runtime-tunable;
`SimParams::without_structural_mutation` gives fixed topology, and metrics or
archives that omit a rate decode it as the zero they ran. The later operators in the
table (effectors, body, duplication, meta-genes) keep proposed starting rates, not a
claim that they exist in the current phase.

Each positive-rate operator gets one Bernoulli gate and at most one edit attempt per
offspring, in this order: remove connection, remove neuron, toggle connection, add
connection, add neuron. Zero rates consume no random draws. Selection is from the
eligible candidates without retry loops. Preflight per-genome limits, scratch
capacity, and required IDs before modifying genes; a refused edit leaves the
candidate unchanged. Draws already used remain consumed. IDs reserved by an applied
edit remain consumed even if a later edit or the eventual world spawn is refused.
ID exhaustion declines growth rather than wrapping; deletion/toggling and re-enabling
an existing edge do not require fresh IDs.

Neuron removal protects oscillators, active sensor targets, and effector sources,
and deletes all incident connections. Adding a connection re-enables a retained
disabled pair with its original ID and weight; a new pair receives a fresh ID and
a weight initialized from `min(weight_init_scale, weight_limit)` divided by the
square root of the new active target fan-in. A split retains its old edge disabled
and atomically adds one sigmoid neuron and two fresh-ID edges. Initial bias and
incoming weight are runtime fields (defaults 0 and 1); tau uses the configured
founder range, and the outgoing weight is inherited. The incoming weight must fit
the configured weight limit when splitting is enabled. A CTRNN split is not assumed
to preserve behavior.

**DECIDED (M8): oscillator addition.** Founders' oscillators are otherwise the only
clocks a lineage can have, since splits add sigmoid neurons. A sixth neural operator,
run after the split with its own rate (`add_oscillator_rate`, zero by default like
the other additions), adds one oscillator neuron, drawn like a founder oscillator
(bias in [-1, 1], tau and period from the founder ranges), and one enabled connection
from it to a uniformly chosen non-oscillator neuron, weighted as an added connection.
Wiring it at once matters because an oscillator ignores its inputs: an unwired clock
would wait on a later edit. With no non-oscillator neuron there is no candidate.
Neuron removal keeps protecting oscillators, so oscillator counts only grow.
Observations count it separately (`add_oscillator`), unknown in older telemetry.

Operator observations count candidate edits, not necessarily surviving births.
Record applied edits and refusal reasons separately from spawn failures; observers
are optional and must not alter RNG, identities, or simulation state.

The scalar pass must supply coherent genes to structural edits. If bias arithmetic
overflows with an extreme finite perturbation scale, clamp at the finite f32
representation limits rather than carrying an infinite bias into the child. This
does not change ordinary finite results or the random-draw sequence.

**DECIDED for M3:** organ edits run before the existing neural pass, in the order
remove sensor, add sensor. This lets a removed organ's targets become eligible for
neural pruning and a new organ's inputs acquire wiring in the same birth. Zero rates
consume no draws. Sensor/organ logic lives
in `mutate/organs.rs`; neural logic remains in `mutate/structural.rs`.

Addition chooses vision, food chemo, or energy interoception using configurable
nonnegative weights, before checking the selected modality's
limits. Do not reroll a different modality on refusal. A successful edit atomically
adds one fresh sigmoid target neuron per channel and one sensor, with no automatic
connections. Target bias is configurable, tau uses the existing
brain range, and sensor initialization matches founders: random vision azimuth,
zero elevation, configured range/FOV, food channel zero/current chemo radius, or
energy selector zero. Deleting a sensor leaves its neurons and connections intact.
All gene/neuron/sensor/ray, scratch, and ID checks precede changes or initialization
draws; the same refusal/consumption rules as neural edits apply.

Runtime sensor admission checks allocated channel and sensing-envelope bounds.
Inherited genes remain valid after live range reductions because the world's
allocated grid is retained; validate against that envelope, not the latest founder
initialization range. Unsupported interoception selectors and nonzero elevation are
not accepted as active Phase 2 sensor behavior.

**Gene duplication deserves emphasis.** It is the primary mechanism by which biological complexity actually increases — duplicate then diverge. Without it, genomes grow one connection at a time and complex sensory organs essentially never appear. With it, an agent can duplicate a working eye and then specialize the copy.

**Evolvable mutation rates** let lineages self-tune. Stable niches evolve low rates; lineages under pressure evolve high ones. It costs almost nothing to implement and it visibly improves the dynamics.

**Founders should get simpler through measurement once these operators exist.** Phase 1 issues every founder the complete sensory suite: three eyes, a nose, an interoceptor, fully connected. That is forced rather than chosen. With no add-sensor and no add-connection operator, anything missing from the founder is unreachable for every descendant for the whole of the phase, so density is the only safe default when structure cannot change.

That argument expired once the structural operators above — add/remove neuron, connection, and sensor — existed. Now a founder carrying a full suite of organs is not a neutral starting point — it is a strong prior that skips the part of the search actually worth watching. Nothing began with eyes; single-celled life began with a gradient and a way to move along it, and every organ after that was paid for. A lineage that *acquires* an eye and covers its metabolic cost is the interesting result, and it cannot be observed in a population that was issued one at birth.

So the founder is the **simplest organism that can still close the loop**, with complexity earned rather than granted. Three consequences:

- **The metabolic terms change meaning.** With a maximal founder, `k_sensor` (§5.2) is a tax every agent pays equally, so it selects for nothing within a generation-0 population. With a minimal founder it becomes the price of an upgrade, which is the selective role it was designed for.
- **Minimal may not be viable, and that is a measurement.** A founder too simple to find food starves before it can reproduce, and the floor depends on the §5.5 energy economy and on food density, not on principle. Sweep it; do not reason it out.
- **Founder composition should be runtime config**, for the same reason every other tunable is (§7.6). It is a parameter to sweep, not a constant to rewrite.

**M3 founder configuration:** retain the existing vision/hidden/oscillator fields,
add founder chemo and energy-sensor counts, and allow an optional incoming-connection
count per hidden/output target. `None` means the original dense topology; a count
selects up to that many distinct sources from the existing input/hidden/oscillator
source set. Choose sparse wiring once per world after plant seeding, using the same
world RNG, and share its template/innovation IDs across founders. Dense construction
consumes no topology draws.

Preserve all four effectors and body/meta compatibility fields. Counts and allocation
budgets must describe the exact sparse template without charging for dense wiring.

**DECIDED (M8): the shipped founder is minimal and chemo-led** — no eyes, one
chemoreceptor, no hidden neurons, two oscillators, and one incoming edge per target. It was the most viable founder measured across seeds
(`docs/phase-2-m8-evidence.md`) and a human approved it. A human then chose to keep
the oscillator scaffold, since no operator could otherwise create one.

### 3.4 Crossover and speciation

Genetic distance (NEAT compatibility):

```
δ = c1 * (disjoint / N) + c2 * (excess / N) + c3 * meanWeightDiff
```

**DECIDED for M4's distance foundation:** align innovation-bearing genes by kind
and ID, using the canonical gene order, without coupling the comparison to crossover.
Within each kind, unmatched IDs beyond the other genome's maximum are excess and
the remaining unmatched IDs are disjoint. If the other genome has no genes of that
kind, all are excess. `N` is the larger total innovation-bearing gene count, with a
minimum of one; there is no special small-genome normalization rule. Body/meta genes
do not participate in these counts or in `N`.

Average absolute weight differences over matching connections, including disabled
ones; use zero when none match. Bias, tau, sensor parameters, bindings, enabled
state, and body/meta values introduce no extra terms. Zero distance therefore does
not imply full genome equality or functional equivalence.

`SimParams.distance` holds finite, nonnegative f32 coefficients
`disjoint_coefficient: 1.0`, `excess_coefficient: 1.0`, and `weight_coefficient: 0.4`.
Widen weights before subtraction and evaluate the mean and weighted sum in f64,
so all finite f32 weights and coefficients produce finite results. Expose raw
components alongside the weighted value for later calibration. The comparison is
allocation-free, deterministic, and consumes no RNG.

This first slice supplies measurement only: it does not assign species, introduce
a threshold, or change trajectories. Coefficients may be supplied through existing
JSON configuration. With World classification integrated, distance and species
configuration are frozen for the life of the world, including when capacity is zero.
The threshold chosen below is provisional, not ecological calibration.

This measures retained innovation history, not functional wiring equality. Physical
deletion discards an ancestry marker: independently recreating the same connection
with a fresh ID increases disjoint/excess distance against a retained-ID counterpart,
even if the resulting wiring is equivalent. Surviving matching IDs still denote
shared origin. Phase 2 must review deletion policy and distance interpretation
together, measuring deletion/recreation against retention/toggling and the resulting
cluster changes before approving nonzero shipped physical-removal rates. Extra
species labels caused by marker turnover are not evidence of useful divergence.

**DECIDED for M4 species classification:** compare each successfully admitted founder
or newborn with immutable copies of active species representatives. Choose the
nearest representative with `distance < threshold`, breaking exact ties by the
lowest species ID; otherwise create a fresh species. Founders use this same rule,
not a forced root species. Membership is fixed for the individual's lifetime.
The classifier consumes no RNG and has no effects on reproduction or energy.

Representatives are owned copies, retained until the last member leaves. Retire
immediately at zero members and emit extinction exactly once. Preserve §2.4's
existing deaths-before-births implementation order and agent-index resolution;
after extinction, even identical recolonizing genomes receive new IDs. Species IDs
are monotonic u32 newtypes, starting at zero, excluding NULL and never reused.

The approved initial capacity is 256 representative slots, each reserving up to
`storage.max_genes`. At the current 1,024-gene cap this is 10 MiB of gene storage
plus metadata, independent of representative length; full equal-size reservations
avoid fragmentation failures. Include all classifier buffers in the core-construction
budget when integrating with World. Capacity and the gene cap are configurable;
zero capacity leaves all agents unclassified without allocating classifier buffers.

Storage or ID exhaustion must never deny an otherwise valid birth or merge it into
an incompatible species. Return an explicit unclassified outcome, retain no member
for that outcome, and still allow matches to existing species when creation is
exhausted. Do not later relabel unclassified individuals; classify their descendants
independently. Report unclassified totals separately so active-species totals plus
unclassified population account for the whole population. Oversized standalone inputs
and unrepresentable membership counts also have explicit, atomic refusal outcomes.

Freeze distance coefficients, threshold, capacity, and representative gene limit for
the life of a classifier. Thresholds must be finite and positive; no automatic
threshold adjustment or target species count is introduced. **DECIDED for World
integration:** `SimParams.species` starts with `capacity: 256` and `threshold: 0.5`.
This replaces the earlier provisional 3.0, which is above the structural-only maximum
of 2 at the starting coefficients. The approved 0.5 is a provisional measurement
scale, not a calibrated biological boundary or an optimization toward species count.

The standalone classifier was delivered separately. World now owns its classifier
and current unclassified population. Classification happens only after ecological
admission succeeds, across external spawns, founders, commands, and births; removal
updates the corresponding classification count exactly once. Live agent
`species_id == NULL_ID` explicitly means unclassified. Optional synchronous callbacks
report creation, extinction, and unclassified admissions; shell-owned cumulative
counters never affect RNG or authoritative state. Existing methods use no-op
observers unless a caller opts in.

Native metrics and WASM/browser status distinguish active populations, unclassified
population, and optional event observations. Scalar-control protocol remains
`randomized_at_birth_v3` because heredity is unchanged. World integration and subsequent full classifier hash coverage
are separate attributable commits; neither changes ecological dynamics.
Retained ancestry/history remains M5 work; this component returns synchronous
assignment and departure outcomes rather than owning an event archive.

**DECIDED for M5's identity foundation:** every successfully admitted founder or
newborn receives a world-local monotonic u64 `BirthId`, starting at zero. Rejected
admissions consume no ID. IDs never wrap or get reused with agent slots; u64::MAX is
reserved for unavailable identity. Exhaustion must not refuse an ecological birth:
keep the counter exhausted and mark that individual's identity unavailable.

Store the individual's ID and two persistent parent references in preallocated
per-slot arrays. Ordinary reproduction captures the actual live parent's BirthId
before allocating the child slot, so an unavailable/dead parent slot cannot be
reused by the child and then mistaken for its own parent. The existing `SpawnSpec`
parent slot names the current live parent at admission, not an imported historical
identity; invalid/dead parent slots produce unavailable stable parentage rather than
guessed links. Capture once, never resolve a child's ancestry through its old parent
slot during inspection. Preserve the legacy parent-slot fields unchanged. The
second stable parent stays empty in asexual births; representation supports two
parents without enabling sexual reproduction.

Human-readable serialization and JS inspection use canonical decimal strings for
IDs 0 through u64::MAX-1 and explicit null for unavailable identity, never JSON
numbers. Binary serde retains raw u64 values, without defining a checkpoint format.
The identity arrays add 24 bytes per agent-pool slot inside the existing core budget;
no per-birth allocation or RNG draw is introduced. They are on-demand inspection
data, not additions to the narrow render snapshot.

This identity slice provides stable references, not a retained ancestry graph or an
archive of dead organisms. The capture/export contract below is the next D6 slice;
browser retention and graph presentation remain separate decisions. Identity integration
preserves existing references; its full state-hash coverage is a separate refresh.
Population telemetry carries no individual history rows. Native metrics are read only
at the current schema; older metrics files are rejected rather than migrated, so the
core keeps one storage layout and validates every params document against it.

**DECIDED for M5 history capture and native export:** record species origins and
extinctions, not every organism's life. Each origin records the new species, its
founding individual's BirthId, and both parent positions. Parentage distinguishes
absent parents, declared-but-unavailable parents, and observed live parents. An
observed parent carries its captured BirthId and an optional species ID; unclassified
parents must not be relabeled as founders. Capture parent metadata before slot
allocation, not when the archive is later drained. Ordinary births still have no
second parent; the event representation supports two without enabling sex.

Events carry the current simulation tick (zero during initial seeding; the tick
being processed during a step). Within a World, callback order is authoritative.
The optional history callback is separate from existing species counters. A
shell-owned, preallocated recorder receives those callbacks; World gains no archive
state, persistence I/O, or history allocation. Plain stepping uses a no-op history
callback. Capturing, overflowing, or draining history must leave the complete
World state/hash and RNG unchanged.

Capture is opt-in. Native capture initially uses a configurable 4,096-record FIFO.
When full, retain queued records and drop new events, assigning sequence positions
even to dropped events. Coalesce dropped positions into explicit inclusive gap ranges.
Keep a pending gap outside the occupied ring so it can be drained even if no later
event arrives; emit it before any newer retained event. Flushing a gap may consume
the last free slot and cause a newer event to start another gap. No overflow policy
may change simulation admission or energy. Sequence exhaustion is explicit and
nonwrapping; already queued records remain drainable.

Native persistence is streaming, versioned JSONL with run/cohort provenance, exact
decimal-string ticks/sequences, origins/extinctions, gaps, and a completion marker.
Drain after seeding and between step batches, including the final batch, without
retaining the entire archive in memory. Surface I/O failures and distinguish a
truncated execution from a completed run with capture gaps. Complete capture is
not proof of complete biological ancestry: unavailable parents and unclassified
lineages remain explicit. This stream is not a checkpoint.

**DECIDED for M5 browser persistence:** enable capture only before the initial
seeding of a new/reseeded World. The WASM shell owns the same 4,096-record recorder.
History delivery is independent of render snapshots: at most one complete drained
batch awaits acknowledgement, issued only after persistence commits. Slow storage
does not stall stepping; queued events overflow into the recorder's ordered gaps.
Drain the whole available prefix at a single tick boundary, including a pending gap.
Do not hash the World or write empty batches every tick. Explicit export/stop
barriers drain and hash one boundary after earlier delivery is acknowledged.

IndexedDB stores archives by a unique run ID, never by seed. Repeating a seed and
configuration creates a different run; pause/play/step keep the same run. Imported
paired native cohorts remain separate sequence/identity namespaces in one archive.
The initial local limits are 10 MiB per run, 50 MiB total, and 20 saved runs, measured
as serialized archive data rather than IndexedDB implementation overhead. Reserve
footer space while capturing so a limit-sized saved prefix can still round-trip.
Quota accounting and appends are atomic across tabs. There is no automatic pruning
or eviction. Explicit deletion must not allow a stale writer to recreate a run.

A limit, quota, or capture failure stops recording only, preserving the last
committed prefix and visibly marking it incomplete. A storage failure may also
prevent updating its status; an open/unfinalized record is never a completed run.
Open records can still belong to another tab, so reload must not invent a crash.
Page reload restores archived history, not World state or simulation continuation.
Browser data may be evicted or cleared; an exported file is the user's backup.

Archive schema 2 extends schema 1 without relabeling old files. Headers declare an
opaque run ID and either cohort alone or both in canonical order; no missing control
cohort is invented. Planned ticks and periodic drain interval may be explicitly null
for open-ended, acknowledgement-drained browser runs. Event/gap records and exact
decimal u64/nullable BirthId encoding are unchanged. Both schemas use the historical
`BirthIdentities` layout inventory and retain the 1 MiB encoded-line limit.

Schema 2 footers give the actual captured boundary and a `capture_end` reason.
`finished` requires the planned end and final hashes. `snapshot`, `stopped`,
`reseeded`, and `params_changed` describe intentional prefixes with boundary hashes,
not finished simulations; capture is complete only when no events were dropped.
`unfinalized`, `storage_limit`, `storage_error`, and `capture_error` explicitly mark
incomplete prefixes even without a recorded gap, and may lack a hash. A zero-record
incomplete prefix is valid if the initial batch never committed. Footer cohort
order, provenance, run ID, counts, and event tick bounds must match the saved prefix.
Native writing stays schema 1; both shells read schemas 1 and 2.

Successful live retuning ends capture at the pre-retune boundary rather than
misrepresenting later events with the original parameters. Rejected retuning keeps
capture active. Browser build provenance identifies dirty builds and development
sessions explicitly; a development server is not an immutable clean revision.
The M6 species-origin graph and M7 resumable checkpoints remain separate work.

**DECIDED for M6 representative history:** retain an immutable copy of each new
species' representative genome at its origin so historical comparisons can outlive
the species. This is one representative per species, not a per-organism genome
archive. Capture before the live classifier can retire it; a delayed lookup after
extinction cannot supply the missing data. Already saved origin/parent/extinction
records and representative genomes survive extinction.

Keep capture shell-owned and bounded, with no ecological effects, RNG changes, or
hot-loop allocation. Representative payloads count toward the existing archive
limits, so richer captures may exhaust that budget sooner. Missing payloads and
capture gaps must stay explicit. Use a versioned extension while retaining readback
of older event-only archives; their absent genomes remain unavailable, never guessed
from descendants. Species-origin graph presentation still needs approval.

**DECIDED mechanism:** World's history callback lends the new species' stored
representative, borrowed for that call only. Shells copy it into a preallocated,
bounded staging buffer until the origin record drains. When staging is full, or the
record would exceed the archive line limit, the origin is archived with an explicit
unavailable reason. History schema 3 is that extension for both the native and
browser archive shapes. Native output stays schema 1 unless representatives are
requested.

**DECIDED species-origin graph (M6):** a **layered tree without a time axis**, built
per archived cohort from recorded origins and extinctions only. Each species links to
the species of its founding parent(s); a node's depth is one more than its deepest
known parent, so depth counts speciation steps, not time. The model holds two parent
links so a later sexual DAG fits. Unclassified or unavailable parents, and parent
species lost to a capture gap or preceding a resumed segment, are marked unknown,
never inferred. Extinct species are distinguished; pruning hides extinct lineages
with no living descendants while keeping every ancestor of a living species.

**DECIDED representative comparison (M6):** an **aligned gene diff** of two archived
representatives: per-kind counts of shared and unique innovation IDs, every
connection whose presence, weight, or enabled state differs, and the exact distance
with its disjoint, excess, and weight terms against the archive's threshold, computed
by the core through WASM so it matches the classifier. A missing representative is
shown as unavailable with its reason; nothing is reconstructed. The browser offers representative capture as its own opt-in, separate
from recording, because genomes consume the same per-run archive budget; without it
browser archives stay schema 2. Browser staging is fixed at 65,536 genes, matching
the native default, and is not user-configurable. A representative comparison view
follows as a separate slice; its design needs its own approval.

Uses:
- **Species assignment** by threshold clustering, for visualization and stats.
- **Reproductive isolation:** agents will only mate if δ < threshold. This means speciation is a real event in the sim, not just a coloring of the plot — and it means mate choice itself becomes an evolvable behavior.

**DECIDED: asexual budding for V1; the sexual pathway unlocks in Phase 6.**

One framing correction first, because it affects what you build. Sex is not something that can emerge from nothing — the *mechanism* (gamete pairing, recombination) has to be provided by the engine. What genuinely emerges is everything around it:

- **Whether it gets used at all.** Make reproduction mode a brain-gated choice rather than a fixed species property, and adoption of sex becomes a real evolutionary outcome you can observe rather than a setting you toggle.
- **Mate-finding.** Locating a compatible partner is a non-trivial behavior requiring sensing, approach, and rendezvous. Nothing about it is hardcoded.
- **Mate choice and advertisement.** `set_signature` and `emit_chemo` are already in the effector catalog (§4.2); courtship displays and attraction pheromones are reachable from them without new machinery.
- **Reproductive isolation.** The compatibility threshold above means mate rejection is mechanical, but *preference within* the compatible range is free to evolve.

So: the engine ships the pathway, evolution decides whether and how to walk it.

**V1 hedges — cheap now, painful later:**

| Hedge | V1 form |
|---|---|
| `reproduce(parentA, parentB?)` | second parent always `null` |
| `parentA` / `parentB` in world state | `parentB = NULL_ID` — see §2.2 |
| Crossover function written and unit-tested | never called by the sim |

The crossover machinery is already specified in §3.4 — NEAT-style alignment by innovation number. Write it and test it offline in V1 even though nothing calls it; it's the piece most likely to have subtle bugs, and debugging it inside a live ecosystem is miserable.

The phylogeny hedge is the one people miss: **with sex, lineage stops being a tree and becomes a DAG.** Storing a single `parentId` bakes a tree assumption into the world state, the serialization format, and the §6 tree viewer. Two fields now, one of them dead, saves reworking all three later.

**Do not unlock sex before predation exists.** Sex carries a twofold cost — an asexual lineage reproduces at double the rate — so in a stable world asexuals win outright and you'll conclude the feature is broken. What pays for that cost is Red Queen dynamics: coevolving predators and parasites that make recombination worth its price. This is exactly why sexual reproduction sits at Phase 6 in the roadmap, after predation lands at Phase 3. Ordering matters here more than usual.

If it works, you get to watch the evolution of sex as an observed transition rather than an assumption. That's one of the more interesting things this simulator could produce.

---

### 3.5 Morphology

**Bodies are multi-part and genome-derived from Phase 5** (§8), before the volumetric-3D step. The `partOffset`/`partCount` indirection in §2.2 exists to receive this.

Crucially, this is the **rigid** version: parts are fixed relative to the agent root. The body is a *shape*, not a machine. No joints, no torques, no physics engine — locomotion stays `thrust`/`turn` on the whole assembly. That single restriction is what makes morphology affordable this early, and it still delivers the thing you actually want, which is that every organism looks like itself.

```ts
{ kind: 'part'; id: InnovationId; parent: InnovationId;
  offset: [f32, f32, f32];   // body-local, z = 0 in 2D
  radius: f32;
  mirrored: bool;            // see below
  pigment: [f32, f32, f32];  // contributes to the agent's signature
}
```

**Appearance must be functionally coupled, or it's just decoration.** Every visual property has to pay rent somewhere in the simulation:

| Property | Consequence |
|---|---|
| Part radius | Metabolic cost (§5.2), collision cross-section |
| Part offset | Where sensors and effectors physically mount |
| Distance from root | `bite`/`grab` reach, and vulnerability of that part |
| Total part count | Genome size, metabolic overhead |
| Pigment | Aggregates into the `signature` that others see (§4.2) |

Get this right and body plans co-evolve with niche: long-reach ambush predators, compact armored grazers, wide-set eyes on prey species. Get it wrong and you have a random blob generator with a nice screenshot.

**Bilateral symmetry is the cheap trick that makes creatures look like creatures.** A `mirrored` flag on a part gene expresses it twice, reflected across the body axis, from one gene. It costs almost nothing, halves the genome size for symmetric plans, and is the single biggest factor in whether output reads as *organism* or as *noise*. Take this before anything fancier.

Encoding: **direct** (an explicit list of part genes) is right for Phase 5. Recursive or L-system encoding — one gene expressing as a repeated limb segment, the Karl Sims approach — is more compact and produces more organic repetition, but it's a real design problem and belongs with articulation (§9.2), not here.

Sensors and effectors gain a `part` binding alongside their existing `target`/`source` neuron ID. In V1 that's always part 0; from Phase 5 it's under genetic control, which means **where an eye sits becomes evolvable** — and eye placement is one of the classic predator/prey divergences in real biology.

### 3.6 Generated appearance (optional layer)

An image model can generate each species' visual assets, grounded in its genome. This is a **render-layer feature that must never touch the sim**, and with that boundary respected it costs nothing architecturally.

**Hard constraints:**

- **Outside the deterministic core.** Image models are not bit-reproducible across hardware or versions, and they involve a network call. §7.4's byte-identical replay, §6's checkpoint-and-fast-forward workflow, and §9.5's lockstep option all depend on the sim being pure. Assets are downstream, cached, async, and droppable — a world must replay identically with no art at all.
- **Per species, not per organism.** At 50k agents with steady-state reproduction, an overnight run has millions of births. Generation keys off speciation events (§3.4 already detects them — species count runs to tens or low hundreds) plus on-demand generation for whatever the user zooms in on. Cache by genome hash with a nearest-neighbour fallback: within ε of a cached genome, reuse that asset.
- **Structural conditioning, not text prompting.** Rasterize the actual part assembly — circles at their genome-specified offsets and radii — as a control image, then condition generation on it. Text-only prompting produces creatures that don't match their simulated bodies, which destroys the functional coupling §3.5 depends on. A long-reach ambush predator must *look* long-reach or the world stops being readable.

**Heredity, which is where this gets genuinely interesting.** Generate a new species' asset via img2img seeded with the **parent species' asset**, at denoising strength proportional to genetic distance (§3.4 already computes it):

```
strength = clamp(k * compatibilityDistance(child, parent), 0.15, 0.6)
```

Small divergence produces slight visual drift; a major speciation event produces a visible jump. Over a long run, lineages diverge visually in a way that mirrors the phylogenetic tree — a morphological tree you can read by eye, which is a genuinely novel artifact for this kind of simulator.

The failure mode is **generational drift**: iterated img2img decays toward the model's attractor states, and after a few hundred generations everything trends to the same brown mush. Mitigation is to re-anchor structurally every time — the parent asset supplies palette and texture continuity, the control image supplies geometry, and because the control image comes from the genome, drift stays bounded by the actual body plan.

**Do the procedural version first, and build it to be sufficient.** Genome-driven pigment, part shape, and hash-seeded texture gets perhaps 60% of the distinctiveness at zero cost, zero latency, and full determinism. Treat it as the finished renderer rather than scaffolding: this layer is what draws at LOD-far (§6), what fills the gap before an asset resolves, and what the simulator ships with if this section is never built.

The cohesion between the two layers comes from the same source — Phase 5's part assembly is literally the control image, so a generated asset cannot wander far from the body the sim is actually simulating. That is the mechanism, not a stylistic preference.

**Sequencing.** This is gated behind Phase 5's success criterion, not bundled with it. If body plans don't yet co-vary with niche, generated art makes a random blob generator *look* good, which is worse than it looking bad — it hides the failure you most need to see. But once coupling is proven, do it while still in the 2D era: sprite-based assets don't survive the volumetric-3D branch (§9.3) at arbitrary viewing angles, so this feature is substantially cheaper now than later.

---

## 4. Sensors and effectors

This catalog is the actual API surface of the world. Each entry is a possible gene.

### 4.1 Sensors

| Modality | Params | Returns |
|---|---|---|
| `vision_ray` | azimuth, **elevation**, range, fov | distance + signature (RGB) of first hit |
| `chemo` | channel, radius | concentration + gradient direction (2–3 scalars) |
| `touch` | azimuth, **elevation** | contact force |
| `hearing` | channel | attenuated sum of nearby emissions + bearing |
| `proprioception` | — | own speed, heading rate |
| `interoception` | which | own energy / age / health |
| `kin_sense` | radius | genetic similarity to nearest agent **← enables kin recognition** |
| `clock` | period | sine oscillator (also available as internal neuron) |
| `light` | — | ambient light level (day/night, depth) |

**Every directional param is stored as an `(azimuth, elevation)` pair, with `elevation` clamped to 0 and its mutation operator disabled in V1.** The field exists in the gene, occupies its slot in the serialized genome, and simply doesn't vary. Going 3D is then a matter of unclamping it. If you store a single scalar angle instead, every saved world and every evolved population you've accumulated becomes unloadable the day you switch — and by then you will have runs you care about.

### 4.2 Effectors

| Action | Params | Effect |
|---|---|---|
| `thrust` | — | forward force, cost ∝ force² |
| `turn` | — | angular velocity |
| `bite` | azimuth, **elevation**, reach | damage to target, transfers energy on kill |
| `ingest` | — | absorb food/corpse in contact radius |
| `reproduce` | — | **gated by the brain** — the agent decides when |
| `emit_chemo` | channel | deposit pheromone, costs energy |
| `emit_sound` | channel | broadcast, costs energy |
| `set_signature` | — | change own displayed color over time |
| `grab` | azimuth, **elevation** | attach to another agent **← enables symbiosis / multicellularity** |

Two of these are disproportionately valuable and cheap:

**`set_signature` + `vision_ray` returning signature.** Together these produce aposematism (warning coloration), crypsis (camouflage against background), mimicry, and species recognition badges. You are giving evolution a channel and letting it decide what the channel means. This is probably the highest interest-per-line-of-code feature in the whole spec.

**Brain-gated reproduction.** Letting the network decide *when* to reproduce, rather than triggering it at an energy threshold, turns life-history strategy into an evolvable trait. You will get r-strategists and K-strategists in the same world, and reproductive timing tied to seasons or population density.

---

## 5. The world and its economy

### 5.1 Energy is conserved

```
sunlight → autotrophs → herbivores → carnivores
              ↑                          │
              └──── decomposition ◀──── corpses
```

Energy enters at a fixed global rate and leaves only through metabolic dissipation. This is what forces genuine competition; unbounded energy input produces a boring world where every strategy works. The rate is a ceiling, not a guarantee: each plant is offered an equal share, a full plant refuses it, and a grazed plant takes only part of it (below), so the ledger records what plants actually absorbed.

Autotrophs (plants) should be simple non-brained entities that grow where nutrients are, get eaten, and reseed. They are the substrate, not agents.

**DECIDED (Phase 2, M9): plants are a population, not fixed scenery.** Phase 1 read "reseed" as regrowth in place: fixed, uniformly scattered sites that refill at a constant rate however hard they are grazed. That world rewards covering ground over sensing. M8's perception sweep found no plant density or input rate where evolving lineages kept sensors wired better than the structural null ([`phase-2-m8-evidence.md`](phase-2-m8-evidence.md)). Three rules replace it, each a `PlantParams` field whose zero value reproduces the Phase 1 behavior exactly:

- **Regrowth depends on what is left.** A plant holding fraction `x` of `max_energy` takes `1 − grazing_lag · (1 − x)` of its share. A full plant takes its whole share; a stripped one only `1 − grazing_lag`, as grass regrows from remaining leaf area and reserves. This is the lower half of a logistic (Noy-Meir) growth curve, with `max_energy` as the upper cap. Stripping a site therefore has a lasting cost, and an overgrazed world absorbs less of its input. `grazing_lag` stays below 1, so an emptied plant always regrows.
- **Plants die and reseed elsewhere.** A plant whose stock stays below `death_stock · max_energy` for `death_seconds` dies, and its slot re-establishes at once. With probability `local_dispersal` the new site lies within `dispersal_radius` of a uniformly chosen plant (seed falls near parents); otherwise it can be anywhere. Either way the site must pass the fertility test in §5.3. The dead plant's remaining stock moves with its slot, so turnover neither creates nor destroys energy, and the plant count never changes. Patches drift as overgrazed plants die and their neighbours spread.
- **Fertility is patchy** (§5.3), so plants cluster where the soil allows.

Deaths are found after growth in step 8 (§2.4) and resolved in plant-index order. Reseeding draws from the world RNG only when a plant dies. Plant positions and starvation timers are world state: hashed, checkpointed (§7.10), and sent in the render snapshot every frame.

### 5.2 Metabolic costs

Every agent pays, per tick:

```
cost = base
     + k_size   * size²
     + k_brain  * (neurons + connections)     ← keeps brains from bloating
     + k_sensor * Σ sensor costs              ← eyes are expensive
     + k_move   * |force|²
     + k_signal * emissions
```

Charging for brain and sensor complexity is what makes the "why not just add everything" strategy lose. Without it genomes grow monotonically and the sim slows to a crawl over a few hours.

### 5.3 Heterogeneity is the engine of diversity

A uniform world produces one optimal strategy and then stagnates. Introduce variation deliberately:

- **Spatial:** fertility varies by region, terrain affects movement cost, obstacles create ambush geography.
- **Temporal:** day/night cycle (drives light-sensing and activity rhythms), seasons (drives storage, migration, dormancy).
- **Stochastic:** occasional local disturbances — a fire, a bloom — that reset a region and open niches.

Every one of these creates a niche, and niches are what let multiple species coexist instead of one clone sweeping the world.

**DECIDED (Phase 2, M9): spatial fertility ships first.** A static map of smoothed value noise, periodic across the torus with feature size `patch_scale`, decides where plants can establish. A candidate site is accepted with probability `(fertility / peak fertility)^patchiness`, both when the world is built and at every reseed (§5.1). Zero `patchiness` is uniform. The map is drawn from the world RNG only when `patchiness` is non-zero, and is regenerated from the seed rather than saved. Fertility decides where plants live, not how fast they grow; a dense patch already receives more input because every plant is offered an equal share. Terrain and obstacles remain later work, and temporal cycles and disturbances stay in Phase 6 (§8).

### 5.4 Reproduction and spatial viscosity

Offspring spawn within a small radius of the parent, with parent energy split between the two. This clustering is what makes kin selection operate. **Do not spawn offspring at random locations** — it's a one-line choice that quietly makes cooperative behavior impossible.

---

### 5.5 Starting constants

These began as guesses so nothing was invented ad hoc. Phase 1 values now include the
M12 acceptance tuning; future-phase values remain provisional. What matters more than
the absolutes are the **relationships**, which are stated alongside.

| Constant | Start | Relationship that actually matters |
|---|---|---|
| `base_metabolism` | 0.05 /tick | Idling must be fatal within ~2000 ticks on a full tank |
| `k_size` | 0.00125 | Doubling radius should roughly quadruple upkeep |
| `k_brain` | 0.00005 /unit | A 200-unit brain costs ~20% of base — noticeable, not crippling |
| `k_sensor` | 0.000625 /weighted channel | An eye costs four times a one-channel interoceptor |
| `k_move` | 0.5 · force² | Sprinting drains a full tank in ~200 ticks |
| `agent_start_energy` | 100 | — |
| `reproduce_threshold` | 200 | With a 50/50 split, a marginal birth leaves both lives at start energy |
| `feeding_reach` | 4 | Local tolerance beyond body + plant radii; still far below sensor range |
| `attack_cost` | 8 | **20–40% of typical prey energy** — the single most sensitive ratio in the sim |
| `attack_damage` | 25 | Several bites to kill, so prey can escape |
| `corpse_energy_fraction` | 0.6 | The rest is lost; the economy must leak |
| `plant_energy_input_rate` | 12000 /sim-second | Supports the 2,000-founder web profile while plant caps reject unused supply |
| `plant_grazing_lag` | 0 until M9 calibration | A stripped plant must regrow clearly slower than a lightly grazed one, or stripping a site costs nothing |
| `plant_death_stock`, `plant_death_seconds` | 0 until M9 calibration | Only sustained overgrazing kills: longer than one grazer's meal, within a few agent lifetimes |
| `plant_local_dispersal`, `plant_dispersal_radius` | 0 until M9 calibration | Dispersal radius comparable to chemo range, so a patch creeps rather than jumps |
| `plant_patchiness`, `plant_patch_scale` | 0 patchiness until M9 calibration | A patch is wider than chemo range and much smaller than the world, so finding the next one takes sensing |
| `chemo_decay` | 0.98 /tick per channel | Trails persist ~50 ticks; **make this per-channel** |
| `chemo_diffuse` | 0.1 | Too high and every gradient flattens to zero |
| `mutation_rate_init` | see §3.3 | Evolvable — this is only the seed value |
| `speciation_threshold` | 0.5 | Approved provisional M4 scale; calibrate interpretation, not toward a target species count |

All of these live in a single `SimParams` struct, serde-serializable, settable at runtime from JS (§7.6). None are compile-time constants.

### 5.6 Senescence (Phase 6)

Through Phase 5, age is observable state and a reproductive-maturity gate, not a direct
cause of death. A successful forager can therefore live indefinitely. **Senescence lands
in Phase 6**, alongside sexual reproduction, mate choice, and seasons, where lifespan and
reproductive timing become meaningful life-history tradeoffs rather than an extra Phase 1
survival penalty.

Start with an age-dependent maintenance cost rather than deleting every agent at a hard
maximum age. That keeps death inside the closed energy economy: old age becomes
progressively expensive, and a lineage pays for longevity through continued foraging.
The onset and slope are runtime `SimParams`; any genetically evolvable repair or longevity
trait belongs to the same phase and must trade energy against reproduction. Do not add a
free age reset at reproduction or an unledgered age-death sink.

---

## 6. Observability

Underinvesting here is the most common way these projects die. You cannot tune what you cannot see, and an emergent behavior you didn't notice may as well not have happened.

**There are two deployment profiles with different scale targets, and conflating them causes trouble.**

| | Native shell | Shared web app |
|---|---|---|
| Runs on | Your machine, headless, overnight | A friend's laptop or phone, live |
| Agents | 50k (aspirational, flexible) | Whatever holds 60fps — expect 2–10k |
| Purpose | Experimentation, long runs, sweeps | The shareable artifact |
| Storage | Files, checkpoints | IndexedDB, seed URLs |

The 50k figure is a target for the native shell, not a requirement for the web build. Treat agent count as a runtime config scaled to the device rather than a constant — the web app should sample capability on load and pick a budget. Nothing in the architecture cares, provided the sim core never assumes a population size (§7.2).

Close-up observation of individuals is wanted in both. That imposes three requirements that are easy to retrofit badly:

- **LOD rendering.** At 50k you cannot draw every agent as a full multi-part body. Distant agents render as a single instanced quad or sphere tinted by `signature`; near agents get the full part assembly plus sensor cones and brain overlay. Two viewing modes in practice — *world view*, where you're reading population-level pattern, and *close view*, a few dozen agents at full detail.
- **Viewport culling in the snapshot.** The render snapshot (§2.2) should carry only agents in or near the camera frustum plus a coarse aggregate for everything else. Cheap in-tab, and it's the same mechanism §9.5 would need over a network.
- **Checkpointing.** Determinism means you *can* replay from seed to any tick, but replaying 4M ticks takes as long as generating them. **DECIDED: manual full-world save/load lands in late Phase 2 (§7.10)** so interesting evolved worlds can be preserved and native runs opened in the browser without replay. Phase 7 adds periodic checkpoints and retention scheduling for the longer workflow: run overnight → wake up → a probe flagged something at tick 4.2M → load the nearest checkpoint → fast-forward deterministically → watch it happen at 1×.

Everything else:

- **Agent inspector.** Click an agent: live brain visualization (neurons firing, connection weights), full genome, sensor field-of-view overlay, energy history, lineage.
- **Phylogenetic tree.** Persistent, incrementally built, prunable. This is the single most compelling artifact these sims produce. Build the renderer to tolerate a node with two parents — it's a tree in V1 and a DAG once sex is unlocked (§3.4).
- **Time series:** population by species, trophic level distribution, mean genome size, mean brain size, species count, energy flow by tier.
- **Species browser** with representative genomes and a diff view between two lineages. If §3.6 is built, this is where generated assets surface — a visual phylogeny alongside the structural one.
- **Event log:** speciation events, extinctions, first appearance of a behavior class.
- **Save / load / replay.** World state serialization, and replay from seed + param blob.
- **Seed URLs.** Encode seed, params, and optionally a target tick into the URL fragment, so a link reproduces a world exactly. Determinism (§7.4) already paid for this; it is close to free and it is the single best feature for sharing with people. "Open this link, watch what evolved by tick 3M" is the artifact.
- **Behavior probes.** Scripted detectors that flag when something interesting happens: sustained pursuit, coordinated movement, signal-then-response correlations. Given the overnight-run target these are **load-bearing, not a nice-to-have** — they are the index into a run you were asleep for. Each probe hit records its tick so you can jump to the nearest checkpoint.

---

## 7. Technology

**DECIDED: Rust compiled to WASM for the sim core from day one; Svelte 5 with TypeScript/JavaScript and a WebGL2 renderer on the main thread.** Three.js arrives in Phase 5. No TypeScript sim-core stage — porting later is real work, and a JS implementation quietly bakes in assumptions you'd rather not carry.

### 7.1 Stack

| Layer | Choice | Rationale |
|---|---|---|
| Sim core | Rust → WASM, in a Worker | Manual memory layout, no GC, no collector jitter |
| UI | Svelte 5 (runes) | Low overhead, doesn't fight an external render loop. Svelte owns panels, never the frame loop. |
| Render | WebGL2 instanced quads; Three.js at Phase 5 | One draw call for all agents. Per-instance color/scale via instanced attributes. |
| Charts | uPlot or a canvas renderer | Do **not** use an SVG/DOM chart library for streaming time series |
| Build | Vite + `wasm-pack` | |
| Storage | IndexedDB, plus `postcard` binary export | |
| Later (optional) | WebGPU compute | 100k+ agents, but heterogeneous brain topologies are genuinely awkward on GPU. Only if you need the scale. |

Crates: `glam` (vectors, quaternions), `rand` + `rand_pcg` or `rand_xoshiro` (seedable — **never `thread_rng`**), `serde` + `postcard` (serialization), `libm` (see §7.4), `ts-rs` (generates TypeScript types from the Rust genome definitions, which stops the inspector UI drifting out of sync with the actual genome).

Two standing notes:
- `SharedArrayBuffer` requires COOP/COEP headers, which constrains where you can host a static build. Read §7.7 before choosing a host.
- Skip a physics engine entirely. Sphere overlap resolution is ~30 lines and is all V1 needs. (Rapier enters only at the articulated-morphology step — §9.2.)

### 7.2 One crate, two shells

```
sim-core/          ← pure Rust, no I/O, no wasm-bindgen
  ├── wasm shell   → wasm-bindgen → browser worker
  └── native shell → CLI binary: headless runs, batch sweeps
```

Costs almost nothing if the core is I/O-free from the start, and buys a lot: real debuggers, `perf`, flamegraphs, and `cargo test` on the native target. WASM debugging in browser devtools is genuinely unpleasant, and you will be doing a lot of it otherwise. The native shell is also the batch-sweep path for Phase 7 and, if it ever happens, the server binary in §9.5.

**No `static` mutable state in the sim core.** This is what keeps the native shell able to run many worlds in one process. It has one concrete implication for §3.1: the innovation-ID counter is a field on `World`, not a `static AtomicU32`.

### 7.3 The memory boundary

The current single-threaded WASM build owns ordinary linear memory inside the worker.
Rust exposes snapshot pointers and lengths; the worker constructs typed-array views over
that memory without copying, then copies the narrow snapshot into the selected transport.
The shared path uses a separate leased triple-buffered `SharedArrayBuffer`; the fallback
uses two pooled transferable `ArrayBuffer`s (§7.7). Both paths copy once in the worker.

**Growing ordinary WASM memory detaches existing JS typed-array views.** Pools and the
snapshot are pre-allocated at capacity, but boundary operations such as inspection can
still allocate. The worker must rebuild its snapshot views whenever `memory.buffer`
changes. Main-thread transport buffers do not alias WASM memory and are not detached by
that growth. Direct shared-WASM-memory reads would require the threads-enabled build
considered in Phase 7; cross-origin isolation alone does not make WASM memory shared.

Keep the boundary narrow — per-tick calls, never per-agent. `inspect_agent()` can return a JSON string; it runs for one selected agent at human speed.

### 7.4 Determinism

Rust helps here — explicit integer types, no coercion surprises — but floating point still bites:

**`sin`, `cos`, and `exp` are platform libm implementations and differ between native and WASM.** The CTRNN and oscillator neurons hit these every tick, so a native run and a browser run from the same seed will silently diverge. Use the `libm` crate explicitly rather than platform intrinsics, everywhere in the sim core. Cheap on day one, archaeology later.

**Avoid WASM relaxed SIMD.** Those ops are specified to permit differing results across engines — that is their purpose — which destroys byte-identical replay. Standard `simd128` only.

### 7.5 Performance: what WASM keeps and what it costs

Kept intact: the full LLVM pipeline (inlining, unrolling, monomorphization, DCE) and — mattering more here — complete control of memory layout with no GC. The SoA arrays, arena-allocated brains, and cache-friendly packing behave exactly as they would natively, and no allocation in the hot loop removes a source of frame-time jitter JS can't fully escape. Bounds checking is a non-issue; modern engines use guard pages.

Given up:

- **SIMD width.** WASM has fixed 128-bit vectors — 4-wide f32 — against native AVX2 at 256-bit or better. Vectorized code runs at roughly a quarter to a half of native throughput. Auto-vectorization is also more conservative; build with `-C target-feature=+simd128` and reach for explicit intrinsics where it matters.
- **Threads.** Available via `wasm-bindgen-rayon`, but this *requires* cross-origin isolation (§7.7), so it's unavailable on any deployment using the no-SAB fallback. Worker pool setup is also manual and there's no plain `std::thread`. Skip initially — single-threaded WASM is already a large win, and the perception phase parallelizes cleanly whenever you do want it.
- **No inline asm, prefetch, or cache hints.** Rarely matters.

Expected gain against *well-written* SoA typed-array JavaScript is **3–10×**, not the 20× sometimes quoted — that JS style is unusually JIT-friendly. Against naive object-per-agent JS, 20×+ is fair. By phase:

| Phase | Gain | Why |
|---|---|---|
| Perception (60–80% of tick) | Large | Branchy, indirection-heavy, memory-bound — where the JIT struggles most |
| CTRNN evaluation | Moderate | Vectorizable, so the narrower SIMD costs you here |
| Genetics / mutation | Large | Variable-length and allocation-heavy — no GC is worth a lot |

Build settings:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
```

Plus `RUSTFLAGS="-C target-feature=+simd128"` and a `wasm-opt -O3` pass. Keep `dyn` trait objects out of hot loops — indirect calls through WASM function tables are pricier than native.

**One optimization worth taking early:** §7.4 already forfeits native's hand-tuned transcendentals, so both targets run a software `tanh` regardless. Replace it with a polynomial or rational approximation — deterministic, several times faster, and biologically meaningless as a difference, since the exact activation curve shape has no bearing on what evolves. It runs once per neuron per tick across every agent, so it's a real line item.

Build both targets and measure your own gap rather than trusting these numbers.

**Memory is the constraint on the 50k target, not speed.** Measured at Phase 1 M4: an agent costs ~11.3 KB, of which the **genome is 98%** — everything else, the SoA arrays, the brain arena, the pool and hash, comes to 233 bytes between them. Pools are pre-allocated at `max_agents` and never grown (§7.3), so that cost is committed up front: 55 MB at 5k agents, **553 MB at 50k**.

Two things follow, and the order matters.

The first lever is **not** lazy allocation. A full world is a full world, and 50k *is* the full world — growing on demand only buys headroom for the common case where population sits below the ceiling. The first lever is the genome arena's layout: `Gene` is an enum sized by its widest variant, so the ~85% of genes that are connections pay 40 bytes for a 20-byte payload. Splitting the arena by gene class roughly halves genome memory and needs no new machinery. Lazy growth composes on top of that, and can be made behaviourally invisible — keep `max_agents` as the ceiling the simulation sees and let allocation track live population underneath, so allocation strategy never reaches the golden hash.

And the detach hazard in §7.3 is narrower than it looks. It applies only to what JS actually views, which is the render snapshot at **61 bytes per agent** — about 0.5% of per-agent state and 3.1 MB even at 50k. Pre-allocate that at capacity and stop thinking about it; the 99.5% that is expensive is never viewed from JS at all, since inspector data is pulled per-agent on demand (§2.2b). Keeping those two questions separate is what makes the rest tractable.

### 7.6 Tuning discipline

This is the standing cost of choosing Rust, and it needs an explicit mitigation. Every failure mode in §10 is a *tuning* problem — attack cost versus prey energy, pheromone decay rate, metabolic constants. You will iterate on those numbers hundreds of times, and Rust's edit-compile loop against Vite's instant HMR is a real tax.

**Every tunable constant lives in a config struct settable at runtime from JS. Never hardcode a number you will want to twiddle.** Get this right and tuning happens in the browser without recompiling, which mostly neutralizes the objection. Get it wrong and the Rust decision will feel like a mistake by Phase 3.

---

### 7.7 Deployment: static hosting and cross-origin isolation

**DECIDED: Azure Static Web Apps.** The target is a static app, shared early, with no server component.

`SharedArrayBuffer` requires cross-origin isolation, which requires two response headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp   (or credentialless)
```

These cannot be set from a `<meta>` tag — they must come from the server, which rules out some otherwise-obvious static hosts. Azure handles it through `staticwebapp.config.json`, whose `globalHeaders` section applies a set of headers to every response:

```json
{ "globalHeaders": {
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Embedder-Policy": "require-corp" } }
```

Two Azure-specific traps, both of which present as "cross-origin isolation is broken" rather than "my config didn't deploy":

- The file must live in the folder set as `app_location` in the workflow, or a subfolder within it. A repo-root config with a build subdirectory is silently ignored — the most common failure.
- Header changes may not apply if other files are unchanged ([Azure/static-web-apps#905](https://github.com/Azure/static-web-apps/issues/905)). Touch a file or force a rebuild.

Verify with `crossOriginIsolated === true` in the console rather than reading response headers; that's what `SharedArrayBuffer` actually gates on.

**Keep the non-SAB fallback.** The snapshot handoff has two implementations behind the same transport contract: leased frames in a separate triple-buffered `SharedArrayBuffer` when `crossOriginIsolated` is true, and transferable `ArrayBuffer`s ping-ponged between two pooled buffers when it isn't. Select the transport when creating a run, not inside renderer logic. Neither path exposes live world arenas to the main thread.

This is insurance against the ways isolation can break: embedding the sim in an iframe on someone else's page, `require-corp` interfering with cross-origin resources you later want to load, or a misconfigured deploy. Both current transports pay one snapshot copy per published frame (§7.3); the fallback additionally transfers ownership by message. It means "can my friend open the link" never depends on header configuration being correct.

**Rejected, for the record.** GitHub Pages cannot set custom headers; GitHub has acknowledged this as a scenario they would support, with no ETA. The community workaround is a service worker shim such as `coi-serviceworker`, which works but forces a reload on first visit and conflicts with any service worker of your own. Cloudflare Pages, Netlify, and Vercel are all equally viable if Azure becomes inconvenient — the `SnapshotTransport` indirection means migrating costs nothing.

Two smaller notes: expect people to open the link on phones, so plan touch controls and a lower device budget; and watch WASM binary size, since `lto = "fat"` and `opt-level = 3` from §7.5 favor speed over size, and a slow first load is the most common reason someone closes a shared link before seeing anything.

---

### 7.8 Verification

Most bugs in this codebase are silent. A determinism break, an energy leak, or an allocation in the hot loop produce a sim that still runs and still looks plausible — you find out weeks later when a replay doesn't match.

Three tiers, and the boundaries between them matter as much as the contents.

#### Tier 1 — Unit and property tests

Applies to anything with a clear contract independent of the ecosystem. These should be thorough; they're cheap and they're where most real bugs live.

| Target | Test |
|---|---|
| Spatial hash | **Differential**: neighbor queries match a brute-force O(n²) reference on random populations |
| Genome ops | **Property**: crossover of two valid genomes yields a valid genome; no mutation orphans a sensor's `target` neuron; serde round-trips |
| Genetic distance | `d(a,a) == 0`, `d(a,b) == d(b,a)`, and known-divergence pairs land in expected ranges |
| CTRNN | Hand-built network produces known outputs; an oscillator neuron oscillates at its specified period |
| Chemo field | Diffusion conserves mass minus decay; gradient points up-slope |
| Interactions | A bite transfers exactly the energy it should, and the loss fraction is exactly what's specified |
| Math | Quaternion ops, `libm` wrappers |

Two patterns carry most of the weight:

- **Differential testing against a reference implementation.** For anything optimized — the spatial hash now, SIMD CTRNN evaluation at Phase 7 — keep a slow, obviously-correct version and assert agreement on random inputs. This is what makes optimization safe rather than nerve-wracking, and it's why the reference version should never be deleted once the fast path exists.
- **Property tests over example tests** for genome operations. The space is combinatorial; you will not think of the right examples, and `proptest` will.

#### Tier 2 — Whole-system invariants

Cannot be unit tested; must hold across a real run. The baseline invariants exist from Phase 1, not added after something breaks; later features extend these guarantees.

**Golden hash.** The highest-value test in the project.

The native and WASM suites share
[`sim-core/tests/common/golden_case.rs`](../sim-core/tests/common/golden_case.rs),
including both scenarios and their reference hashes. Phase 1 pins a 300-tick shipped
configuration and a 500-tick reproduction-heavy configuration, each with 200 founders.
The former must remain populated; the latter must actually produce offspring, so an
empty or non-reproducing run cannot satisfy the intended coverage.

Phase 2 M1 additionally shares
[`sim-core/tests/common/storage_case.rs`](../sim-core/tests/common/storage_case.rs):
a hand-expanded genome feeds, reproduces, hits a pooled-neuron limit, and continues
stepping in both heredity modes. This pins the variable-storage path across targets,
not a claim that useful structure evolved.

M2's [`structural_case.rs`](../sim-core/tests/common/structural_case.rs) and M3's
[`organs_case.rs`](../sim-core/tests/common/organs_case.rs) additionally pin neural
and sensor edits, bounded refusals, births, slot reuse, and continued stepping in
both heredity modes. The M3 case starts from a sparse chemo-only founder; it proves
cross-target mechanism agreement, not ecological viability.

M4's [`species_world_case.rs`](../sim-core/tests/common/species_world_case.rs) pins
plant-funded classified births, same-tick retirement/recolonization, unclassified
command admissions, and later storage/slot reuse in both heredity modes.

`state_hash` folds world state, including positions, energies, genomes, recurrent neural
state, future slot-allocation order, variable-arena capacities/free spans and live
handle placement, the retained founder template, RNG state, and tick. Allocator layout
matters because freeing an agent changes fragmentation and later birth success.
Sparse founder wiring is sampled once and reused by later founder commands; current
RNG state cannot substitute for that cached template. M3's separate reference refresh
adds this coverage without changing trajectories or shipped defaults.

Full classifier coverage includes its frozen coefficients/threshold/capacities, next
species ID, active membership counts, owned representative genomes, reservation
handles and allocator metadata, plus World's unclassified population. This is
authoritative future classification state, unlike shell-owned observation counters.
Unused tails of full representative reservations and freed payload remain irrelevant.
M4 separates the initial live-species-label metadata update (which moves the organ
control reference) from the subsequent full classifier-coverage refresh. Neither
changes ecology or RNG. A test-only ecological fingerprint masks classification
metadata while retaining every ecological field and RNG position; several seeds
and both heredity modes must agree with classification enabled or disabled.

M5's separate coverage refresh adds the world-owned birth counter and each live
agent's lifetime ID plus both captured parent IDs. Dead-slot payload remains
irrelevant. The preceding identity-integration commit retains the earlier references;
this refresh records extra authoritative state, not a change to ecology or RNG.
The test-only ecological fingerprint also excludes these lifetime identities.
[`birth_case.rs`](../sim-core/tests/common/birth_case.rs) is shared by native and WASM,
covering plant-funded reproduction, parent death/reuse, failed admission, and continued
stepping with persistent parent references.

Stale free-space payload and
optional observer counters are not authoritative. A behavior-changing refactor
must not silently move the reference; an intended behavior change requires a deliberate,
reviewable update. Strengthening hash coverage can also change references without
changing trajectories, and must be identified as such. Seed, params, and heredity mode
remain experiment configuration recorded alongside the hash, not a substitute for it.

**Energy conservation.** Sum everything — agents, plants, corpses, field — and assert the delta over N ticks equals `input − dissipation` within epsilon. Catches double-counted eating, corpses that resurrect energy, reproduction that mints it. Since §5.1's closed economy is what makes selection real, a leak doesn't crash anything; it quietly makes the simulation uninteresting.

**Cross-target agreement.** Same seed through native and WASM, compare `state_hash`. This is what actually enforces §7.4's `libm` discipline — a platform `sin` passes every single-target test and fails only here.

**No allocation in the hot loop.** Counting allocator, assert zero allocations across a `step()` after warmup.

**Checkpoint continuation (Phase 2).** Saving/loading must preserve the state at the
boundary and reproduce an uninterrupted run after further ticks, in both heredity
modes and in both native/WASM transfer directions. Exercise structural births/deaths,
allocator reuse/fragmentation, species changes, recurrent neural state, energy
residuals, and future queued commands. Successful deserialization or an immediate
hash match alone cannot prove exact continuation.

#### Tier 3 — Not testable

Every phase success criterion in §8 is a judgment about whether something *interesting* evolved. No assertion covers this, and no amount of Tier 1 and 2 passing implies it. Treat any claim that a phase is complete on the strength of green tests as unverified.

A tool that helps: **a random-brain control population.** §10's last failure mode is that humans see intent in moving dots. Run it as a separate world with the same seed and params; putting a control lineage in the evolving world would make it compete for the same energy and perturb the measurement. Founders match exactly, while every control offspring redraws its neural scalars instead of inheriting them. In Phase 1's fixed topology this breaks cumulative neural-scalar inheritance without changing sensors, body, or ecology. Indistinguishable behavior does not establish a benefit from that inheritance. In automated reporting (§7.9), every behavioral metric should be printed alongside the relevant control's value for the same metric.

**DECIDED: the scalar control, protocol `randomized_at_birth_v3`.** Evolving
offspring receive scalar mutation, then organ edits, then neural structural edits.
Scalar-control offspring receive the same organ and structural edits, then a redraw
of all neural scalars on their resulting topology; sensor parameters and bindings are
not redrawn. It can inherit and evolve topology, so it is not a structural null. The
browser's `randomized_at_birth` mode identifier is labelled "scalar control".

Disabling structural rates does not discard evolved topology. Scalar redraw must
remain safe for its retained fan-in, including one-input neurons. Sampling a finite
interval whose width overflows f32 uses bounded interpolation rather than an
infinite intermediate; ordinary interval arithmetic and the single random draw are
unchanged.

**DECIDED: Phase 2 also requires an approved structural-null comparison before
acceptance.** A scalar-heredity control that inherits/evolves topology cannot alone
establish that structural growth is adaptive. Decide the structural-null protocol at
M0 and implement/report it with M8's experiments, rather than discovering the missing
instrument at acceptance. Approval must name the structural contribution or inheritance
disrupted, the preserved quantities, metabolic and sensory confounds, cohort/seed
matching, measurements, and limits on interpretation. Resetting to founder topology
or arbitrary rewiring is not assumed to control those confounds.

**DECIDED (M8): the structural null is donor topology with parent scalars,
protocol `structural_null_v2`.** It is a third heredity mode run as the paired
control world, selected natively with `--control structural-null`; the metrics
header's `control` field names the protocol, and `diagnose` labels the cohort
"structural null". It writes metrics only; history archives and saved runs carry the
scalar control. (An earlier version redrew every neural scalar, collapsed exactly like
the scalar control, and so could not isolate structure; it was retired, which is why
the protocol is versioned.)

- *Birth rule:* each birth draws a donor uniformly from the living agents other than
  the parent, in slot order, including agents born earlier that tick; with no other
  living agent the parent is its own donor and no draw is made. The child starts from
  the donor's neuron, sensor, effector, and connection genes with the parent's body
  and meta genes. Wherever the parent has the same gene (same innovation ID and
  structural fields: connection endpoints, neuron activation), the child takes the
  parent's neural scalars (weight, bias, tau, oscillator period); donor-only genes
  keep the donor's, and enable states and sensor/effector parameters stay the
  donor's as structure. The child then receives the evolving mutation pipeline.
- *What it disrupts:* the coupling between a lineage's structure and its weights,
  while brains stay functional because shared genes carry the lineage's scalars.
- *What it preserves:* founders and seed/params match the other worlds exactly; the
  population's distribution of topologies, costs, and sensory opportunity is
  carried by whichever topologies survive; body, placement near the parent, and
  energy accounting are unchanged.
- *Confounds:* donors are selection-filtered survivors, so useful structure can
  still spread by being copied. Early in a run most genes are shared founder genes,
  so the comparison has power only where structure has diverged.
- *Matching and measures:* the same seeds and params as the evolving world and the
  scalar control, at least three seeds per configuration, reporting neurons,
  connections, sensors, genome sizes, species persistence, population, energy, and
  behavior metrics with their variance, separately from the scalar control.
- *Interpretation and limits:* if evolved structure is useful because it fits its
  lineage's weights, the evolving world should outperform the null; if structural
  change is neutral drift, the two should match. It cannot show that a particular
  circuit is adaptive, nor anything about scalar inheritance, which both worlds have.

Report the structural-null and scalar-heredity comparisons distinctly across
multiple seeds. Early viability experiments need not wait for the structural-null
implementation, but useful-structure claims and Phase 2 acceptance do. Missing or
inconclusive structural evidence cannot be replaced by gene counts or human
impressions alone. The evidence informs, rather than replaces, the human judgment
required by §8.

### 7.9 The experiment loop

Tuning this simulator is a large, tedious, highly parallel search — exactly the work worth automating. The native shell is headless and deterministic, so an agent can run experiments and read results without a human in the inner loop.

**Telemetry, not observation.** Don't have an agent watch a rendering. Have the sim emit structured metrics from headless runs:

```
cargo run -p native -- --seed 42 --ticks 500000 --metrics run.jsonl
```

One JSON line per sample interval: population by species, trophic biomass by tier, mean and max brain size, genome size distribution, energy flow per tier, speciation and extinction events, behavior-probe hits. This runs at 1000× and produces something an agent can actually reason over, which a canvas is not. Emit only fields the current phase can measure: before Phase 2 species clustering exists, report exact genome variants and mark species-based diagnostics unavailable rather than treating the placeholder species ID as data.

**DECIDED for Phase 2 telemetry (native metrics schema 8):** report genome-size distributions as exact nearest-rank order statistics (min, quartiles, max, plus mean) of all genes, neuron genes, connection genes, and enabled connections across living agents, separately per cohort. Also record each cohort's history-capture availability (capacity, retained and dropped events, gaps), or explicit absence when capture is off. The browser reports the same distributions through the same shell code, so both agree exactly at a completed tick. Only the current metrics schema is read.

**DECIDED (M8): report functional wiring alongside size.** Gene counts include structure nothing reads, so each cohort also reports, per living agent, the hidden neurons on an enabled path from an input (a sensor target or an oscillator) to an output (an effector source), the sensors with a target on such a path, and the effectors driven from any input. These describe wiring, not usefulness; they exist because M8 found genomes growing while perception was shed. Files written before them report wiring as unknown.

**Encode §10 as a diagnostic.** Every failure mode in that table is visible in the metrics:

| Signal | Diagnosis |
|---|---|
| Population → 0 early | Energy input too low, or mutation rate past error catastrophe |
| Species count → 1, stays there | World too homogeneous (§5.3) |
| Mean brain size climbing without bound | `k_brain` too low |
| Carnivore biomass → 0 | `attack_cost` too high relative to prey energy |
| Prey biomass → 0, then total collapse | `attack_cost` too low; no refugia |
| Mean speed ≈ 0, population stable | Metabolism too cheap — idling isn't fatal |
| Signal emission uncorrelated with anything | Spatial viscosity too loose (§5.4) |

`cargo run -p native -- diagnose run.jsonl` reporting which of these matches turns the most common debugging session into one command.

**Sweeps.** Grid over the parameters that interact — `attack_cost` × `attack_damage` × plant growth is the Phase 3 one — and report which cells produce stable coexistence. **Require N seeds per cell and report variance.** A single seed can look excellent by luck, and a single good run is the most common way an automated report misleads.

#### Where this goes wrong

**Every metric is Goodhart-able, and an agent will find the degenerate solution faster than you can guard against it.** Maximize species count → drop the speciation threshold to 0.001 and get ten thousand species that are noise. Maximize population → make food free. Maximize brain size → remove the brain cost.

There is an irony worth sitting with: an outer optimization loop with a scalar objective recreates precisely the problem §1 says to avoid on the inner loop. Explicit fitness functions are what kill open-endedness, at both levels. So:

- **No scalar objective.** The loop reports a vector of metrics with the control values alongside. It does not rank configurations, and it does not pick a winner.
- **The agent may tune `SimParams` freely. It may not edit `sim-core` in response to metric outcomes without human review.** This is the important one. "Population is unstable" is fixable by quietly weakening a metabolic cost — every metric improves and the simulation is destroyed. Parameter search and code change must stay separate loops.
- **Behavior probes are the least gameable metrics** because they measure behavior rather than aggregates — sustained pursuit, coordinated movement, signal-then-response correlation. Weight them accordingly, while remembering they're still heuristics.
- **The human gate is on "is this interesting," never on "did it run."** The loop's output is a shortlist of configurations worth looking at, and looking is done by a person watching the sim.

#### The loop, in shape

```
agent proposes a hypothesis about a parameter interaction
  → headless sweep, N seeds per cell, deterministic
  → metrics + diagnose + control comparison
  → agent reports: what happened, across how many seeds, with what variance
  → human watches the promising configurations
  → human decides
```

Determinism is what makes this trustworthy: every result is a reproducible `(seed, params)` pair, so a claim about a run can always be checked rather than believed.

### 7.10 Manual checkpoints (Phase 2)

**DECIDED:** basic manual checkpoints arrive after Phase 2's storage, species, and
ancestry work. They enable checkpoint-assisted inspection but are not a prerequisite
for starting founder experiments; those can begin with configurable founders and
existing telemetry, then acquire richer evidence as observability lands. Manual
checkpoints remain required before Phase 2 is complete. A checkpoint preserves a
complete running world at a between-ticks boundary; an exported phylogeny preserves
history and is not a substitute for resumable state.

**DECIDED save/load packaging:** the normal **Save run / Load run** workflow uses
one portable, versioned saved-run bundle containing the full-world checkpoint and
its available history prefix. Keep resumable state and history as distinct internal
components, but do not require users to manage two unrelated saves.

**DECIDED UI/CLI split:** browser **Save run** downloads the complete saved-run
bundle, and **Load run** uploads it and restores the world paused with its available
history. The browser continues to display history but exposes no standalone
history-file import/export actions, including advanced options. M7 replaces the
interim M5 browser history-file controls with this combined workflow.

Standalone history export and import/readback are CLI-only analysis tools, retained
alongside CLI support for complete saved-run bundles. History-only analysis does not
start or resume a World.

Use one bundle format shared by the native and browser shells. Native file
save/load and browser download/import must interoperate when their simulation
compatibility identities match and the receiving host can accommodate the world.
The identity describes format and simulation compatibility, not a target-specific
binary hash. Reject incompatible versions explicitly; Phase 2 promises no
cross-version migration. Retain the originating seed/run provenance in the
checkpoint metadata and associate the history with its originating run/cohort.

Preserve parameters, heredity protocol, RNG state, tick, authoritative physical and
genetic state, recurrent neural values, compensated energy and its ledger, future
allocation order/availability, innovation and ancestry counters, active species
representatives, and queued commands in their original application order. Derived
caches, scratch, and presentation/transport state may be rebuilt only without
changing continuation; live recurrent neural values are not derivable from a genome.
Saving must not advance simulation time or consume randomness. Continuation must
satisfy §7.8.

Capture the checkpoint and active history at the same between-ticks boundary.
Include only the matching history prefix up to that boundary, never the original
run's later events. Preserve recorded gaps and capture status. If capture was
disabled, stopped, or failed earlier, include whatever earlier prefix is available
with its original end boundary/status, or explicitly declare history unavailable.
Missing or incomplete history does not make a checkpoint unresumable; a saved run
must not claim or reconstruct historical observations it never captured.

The core owns in-memory checkpoint encoding/decoding, not I/O or the history archive.
Shells assemble the saved-run bundle, schedule capture/load outside a step, and own
files and browser interactions. Treat imported bytes as
untrusted: enforce bounded decoding and resource limits, validate structural and
numeric invariants, and validate both components and their run/cohort/boundary
association before replacing a live world. A rejected load leaves the existing world
and displayed history intact. Browser loads restore the world paused together with
its historical context and invalidate old worker responses, selections, and
render-buffer leases.

A resumed run must identify its checkpoint origin and restored history prefix,
then begin a distinct history segment rather than silently appending after the
original run's later events. Loading an older saved run must not attach history from
that later future. Resuming paired experiments requires both worlds at the same
tick with the original control protocol and matching history prefixes; a freshly
seeded control is not a continuation.

**DECIDED core checkpoint encoding (M7):** `World::checkpoint()` writes the 8-byte
magic `SEVCKPT\0`, a little-endian `u32` format number (the simulation-compatibility
identity), and a postcard-encoded record. It stores the construction seed, current
params, heredity mode, RNG, tick, innovation/birth counters, ledger, pool free order
and incarnations, every live agent's physical and identity fields, genome and exact
arena placements, recurrent neuron values, the parts arena, plant stocks, plant
positions and starvation timers, chemo concentrations, active species with their
representatives and blocks, queued commands, the spatial grid's cell count, and the
saved `state_hash`. It does not store derived data: compiled neurons, synapses,
sensors, and effectors are recompiled from validated genomes, and the seed-derived
fertility map and founder topology are regenerated by constructing the world from its
seed; the hash confirms them.

`World::from_checkpoint` decodes untrusted bytes under the host's limits: an encoded
byte ceiling, and a ceiling on the saved params' core construction budget, checked
before any world is allocated. It builds the world through the ordinary budgeted
constructor, validates placements, lengths,
genome coherence and limits, finiteness, species membership, pool order, and
counters before writing any state, and finally requires the restored world to
reproduce the saved hash. A retuned world keeps its construction grid, which may be
coarser but never finer than its current params allow. Encoding is byte-identical on
native and WASM, so saved runs transfer in both directions.

**DECIDED saved-run bundle (M7):** one container shared by both shells: the
8-byte magic `SEVRUN\0\0`, a little-endian `u32` container version (1), a `u32`
manifest length, a strict JSON manifest (at most 1 MiB, unknown fields rejected),
then each cohort's core checkpoint followed by each included history archive. The
manifest records the container and checkpoint formats (checkpoint format 3 since M9
added `grazing_lag` to the encoded params; earlier formats are refused), the originating run's
provenance (kept unchanged across resumes), the build that wrote the bundle, the
save tick, one cohort or both in canonical order with each checkpoint's state hash
and length, and an ordered list of **history segments**. Each segment starts at a
tick (the first at 0), ends where the next starts or at the save tick, and is either
an included archive or explicitly unavailable (`not_recorded`, or `not_retained`
when capture streamed somewhere the shell cannot read back). Loading validates every
checkpoint against the manifest (heredity, seed, tick, hash), requires paired cohorts
to share params, and requires each included archive to belong to the run, cover the
bundle's cohorts, and end exactly at its segment boundary, so later events cannot leak
in. A paired archive may accompany one cohort that a browser loaded from a paired run.
Segment starts are non-decreasing: an archive can end at the tick it began, holding
only that tick's founder origins, and be followed by a gap from the same tick.

**DECIDED browser Save run / Load run (M7):** Save run takes the checkpoint inside the
same worker drain that closes a live recording's snapshot boundary, so both describe
one tick; without a live recording it saves the world's stopped or incomplete archive
as far as it reached, then an explicit gap. Load run validates the whole bundle
(framing, every checkpoint through WASM with a 256 MiB core-budget ceiling, and every
archive) before replacing anything, persists the restored archives, and restores the
world paused; a paired native run loads its evolving cohort. Recording on a loaded
world captures a browser-shaped schema 4 segment from the load tick. A save request
never shares a pending retune boundary, whose checkpoint would carry new params. The
browser's standalone history-file import/export controls are removed.

A resumed run keeps the restored segments as saved and begins a new segment at its
resume tick; saving again appends that segment, included when its archive could be
re-read and otherwise marked. **History schema 4** is that resumed segment, in either
archive shape: its header adds `resumed_from_tick` and may declare representative
staging. Its lineage before that tick is unknown, exactly as after a gap, so it may
record extinctions of species it never saw originate. Its events fall at or after
the resume tick and strictly before its end, it numbers its own sequences from zero,
and, unlike a run that seeds founders, it may legitimately be empty. A bundle's
first segment uses schemas 1-3; later included segments must be schema 4 resuming at
their own `starts_at`. Metrics for resumed runs are not yet produced.

Periodic autosaves and retention scheduling remain Phase 7 work. Cross-version
migration, timeline scrubbing/indexing, compression, and storage optimization are
also deferred, not requirements of the Phase 2 save/load milestone.

---

## 8. Roadmap

**Phase 1 — the loop works.** Fixed-topology CTRNN, hardcoded sensors (vision, chemo, energy), hardcoded effectors (thrust, turn, ingest). Plants, energy economy, death, asexual reproduction with mutation. Render as 2D circles first. *Success: agents evolve to move toward food. Takes minutes of sim time. If this doesn't happen, nothing else will.*

**Because the goal is to share early, the renderer needs to be presentable from Phase 2, not Phase 5.** A clean instanced 2D renderer plus seed URLs makes every phase from 3 onward shareable, which is where the feedback actually comes from. The Three.js pass at Phase 5 then upgrades a working presentation rather than creating one — don't defer *all* visual polish to Phase 5 on the strength of that line item.

**Phase 2 — genetic architecture.** Variable-length genome, innovation IDs, add/remove neuron and connection, connection enable/disable, add/remove sensor, genetic distance, species clustering, phylogenetic tree, and basic manual portable checkpoints (§7.10). Plant ecology also lands here, ahead of the seasons and terrain in Phase 6: stock-dependent regrowth and plant turnover with local dispersal (§5.1), and patchy fertility (§5.3). The success criterion cannot be judged in a world where blind grazing does as well as perceiving. **Revisit founder composition here (§3.3)** once the structural operators exist; the minimal viable founder remains a multi-seed measurement, not an assumed starting configuration. Build order and implementation decisions under review live in [`phase-2-implementation-plan.md`](phase-2-implementation-plan.md). *Success: brains grow in complexity, distinct species appear.*

**Phase 3 — predation.** Bite effector, damage, energy transfer, corpses, decomposition. Tune attack cost. *Success: a carnivorous lineage becomes established without going extinct or eating everything. This will take tuning — the ratio of attack cost to prey energy is the critical parameter.*

**Phase 4 — signaling and sociality.** Pheromone emit/sense, sound, `set_signature`, kin sense. Verify spatial viscosity is tight enough. *Success: signal emission correlates with something — predator presence, food location. Look for warning coloration.*

**Phase 5 — morphology.** Multi-part rigid bodies from the genome (§3.5), bilateral symmetry, sensors and effectors mounted on parts, per-part metabolic cost. Three.js render pass with LOD. *Success: body plans visibly co-vary with trophic role — you can guess what something eats by looking at it. If morphology is uncorrelated with niche, the functional coupling in §3.5 is too weak and parts are cosmetic.*

**Phase 5b — generated appearance (optional).** Per-species assets from an image model, structurally conditioned on the Phase 5 part assembly and inherited via img2img from the parent species (§3.6). *Gated on Phase 5's success criterion — do not start this until body plans provably co-vary with niche.* Best done before the 3D branch, since sprite assets don't survive volumetric rendering.

This phase is genuinely optional: the roadmap is complete and the simulator is finished without it. Consequently **Phase 5's procedural renderer must stand on its own** — genome-driven pigment, part shape, and hash-seeded texture built to look good, not built as a placeholder for something better. If 5b never happens, nothing is missing; if it does, the procedural layer is still what draws at LOD-far and what fills the gap before an asset resolves.

**Phase 6 — depth.** Sexual reproduction and mate choice, senescence and evolvable
longevity tradeoffs (§5.6), seasons, terrain heterogeneity, `grab`/symbiosis, gene
duplication. *Success for the sex unlock specifically: a sexual lineage persists rather
than being outcompeted by asexual cousins. If it doesn't, the Red Queen pressure from
Phase 3 is too weak — that's a predation-tuning problem, not a reproduction bug.*

**Phase 7 — performance and scale.** SIMD in the perception and CTRNN phases, `wasm-bindgen-rayon` if needed, automatic checkpoint scheduling and retention for overnight native runs, batch parameter sweeps. Build on Phase 2's manual save/load rather than introducing checkpoints here for the first time. Push to the 50k target. (The sim core is already Rust/WASM from Phase 1 — this phase is optimization, not a port.)

The roadmap ends here deliberately. What follows is a **branch, not a Phase 8** — volumetric 3D and articulation are alternative directions with different costs and different payoffs, and articulation in particular trades away the 50k scale target rather than building on it. See §9.3.

---

## 9. Long-term direction

Two long-term directions run independently of each other: the **world** getting richer (§9.1–9.4) and the **deployment** becoming a multi-client service (§9.5). Neither depends on the other.

V1 is a 2D plane rendered in 3D. The intended eventual destination is **fully volumetric 3D with non-spherical, multi-part morphology** — agents that are structurally as well as behaviorally evolved. This section exists so that destination shapes the code we write now.

The distinction that matters: some of the gap between here and there is cheap to hedge against and catastrophic to retrofit, and some of it is genuinely new work that no amount of foresight avoids. Hedge the first category, accept the second, and don't confuse them — over-abstracting the second category costs you V1.

### 9.1 Cheap now, catastrophic later — do these in V1

| Hedge | V1 form | Why retrofitting hurts |
|---|---|---|
| 3-component vectors | `N*3`, z pinned to 0 | Touches every math site in the codebase |
| Quaternion orientation | `N*4`, yaw-only | A scalar `heading` makes turn effectors, sensor aiming, and rendering all 2D-native |
| `(azimuth, elevation)` sensor params | elevation clamped to 0 | **Invalidates every saved genome** — you lose accumulated populations |
| Parts indirection | `partOffset`/`partCount`, always 1 | Cashed in at Phase 5 (§3.5), not some distant future — this is the big one |
| Turn effector takes an axis | axis pinned to Z | Yaw/pitch/roll falls out for free |
| Spatial hash as 3D loop | Z range `[0,0]` | 9-cell → 27-cell neighbor iteration is a loop bound, not a rewrite |
| Chemo field as 3D grid | depth 1 | Same |
| Metabolic cost summed per-part | one part | Cost model otherwise assumes agent == body |
| `parentA`/`parentB` lineage fields | `parentB` unused | Phylogeny becomes a DAG under sexual reproduction (§3.4) |

**The parts indirection is the single highest-value hedge.** "An agent is a sphere" is an assumption that leaks into rendering, collision, sensing, metabolism, the genome, and the inspector. If instead an agent is a *list of parts* — each with a body-local offset, a radius, and a parent index — and V1 simply always has exactly one part at the origin, then every one of those systems is already written against the general case. Sensors and effectors bind to a part index (always 0 in V1) rather than to the agent root. The cost today is an extra indirection in a few loops. The cost of adding it later is touching everything.

The genome design in §3.1 already accommodates this: adding a `{ kind: 'part'; id; parentPart; offset; radius; jointAxis }` gene kind to a typed-gene list is additive. That was not an accident.

### 9.2 Genuinely new work — accept the cost, don't pre-build

With rigid morphology pulled forward into Phase 5 (§3.5), what remains genuinely hard is **articulation** — bodies as machines rather than shapes.

- **Rigid-body physics with joints.** Thrust/turn on an assembly is not a special case of articulated dynamics; it's a different model. Don't build a physics abstraction layer now hoping it will generalize.
- **Locomotion via joint torques.** Movement becomes emergent from body dynamics rather than a `thrust` primitive. This resets locomotion evolution to zero — Phase 5 swimmers won't transfer.
- **Recursive body-plan encoding.** Directed graphs with recursion limits, where one gene expresses as a repeated limb segment. Belongs here rather than at Phase 5, where direct encoding plus a mirror flag does the job.
- **Compound-shape contact resolution.** Sphere-set intersection is cheap; contact and constraint solving between articulated bodies is not.

No genome hedge is needed for any of this. Because §3.1 is a list of *typed genes* rather than a fixed-arity parameter tuple, adding a `joint` gene kind later is purely additive — old genomes load unchanged, with zero joints. This is the one place where "don't pre-build" carries no retrofit penalty, which is exactly why the elevation and parts hedges in §9.1 were worth taking and this one isn't.

**On Rapier: not at Phase 5, and think hard even at articulation.** For rigid compound bodies it's straightforwardly the wrong tool — you'd be paying for a broadphase, narrowphase, and iterative constraint solver to do work that sphere-set overlap resolution handles in about thirty lines, at 50k agents.

The sharper problem is determinism, and it's a direct collision with §7. Rapier can be bit-level cross-platform deterministic, but its `enhanced-determinism` feature cannot be enabled at the same time as the `parallel` or `simd-stable`/`simd-nightly` features. So adopting Rapier means choosing between byte-identical replay and the SIMD work in §7.5 — and byte-identical replay is what §9.5's cheap lockstep multiplayer depends on. That's a real architectural fork, not a build-flag detail. Rapier's own guidance also notes that values used to initialize its structures must not come from platform `sin`/`cos`/`tan`, which is the §7.4 discipline extended into the physics layer.

**Worth evaluating first: spring-mass soft bodies.** Parts connected by damped springs with genetically evolvable rest lengths that the brain can oscillate. It's a few hundred lines, fully under your determinism control, far cheaper than a constraint solver, and produces strikingly organic locomotion — it's roughly the Framsticks approach. For evolved swimming and crawling specifically, it may simply be better than rigid joints, not just cheaper.

### 9.3 Ordering, and why articulation is a branch rather than Phase 8

```
V1 (2D, spheres)
  → Phase 5: rigid multi-part morphology, 2D    ← unique bodies, no physics engine
  → Phase 7: performance, 50k target
      │
      ├─→ volumetric 3D, rigid bodies           ← cheap: unclamp the §9.1 hedges
      │                                            new ecology, scale preserved
      └─→ articulation                          ← expensive: new physics (§9.2)
                                                   new bodies, scale sacrificed
```

Putting morphology before either branch is deliberate. It exercises the parts indirection in the cheapest possible setting — you learn whether the parts arena, per-part costs, and part-mounted sensors actually work while you still have 2D to debug in. Doing morphology and dimensionality at once means every bug has two candidate causes.

**Articulation is not scheduled as a phase because it competes with the scale target rather than extending it.** Articulated bodies cost roughly an order of magnitude more per agent, so a world that runs 50k rigid agents runs perhaps 2–5k articulated ones. That is not a scheduling problem you can optimize your way out of; it's a position on a tradeoff curve. Karl Sims ran a few hundred creatures. Framsticks is the same order. Nobody runs 50k articulated organisms, and the reason is arithmetic.

So the honest framing is that this is a **fork in what the simulator is for**:

| | Many simple agents | Few complex agents |
|---|---|---|
| Scale | 50k | 2–5k |
| What you study | Ecology, population dynamics, coevolution | Embodied locomotion, body/brain co-adaptation |
| Emergence you get | Trophic webs, signaling, speciation | Gaits, limbs, novel body plans |
| Reference points | Polyworld, bibites | Karl Sims, Framsticks |

Both are worth building. They are not the same project, and pretending articulation is "Phase 8" hides that. If the sim core avoids assuming a population size, this can be a world-config branch rather than a repository fork — same genome library, same brain, different physics module and different agent count.

**Prerequisites, in order:** Phase 5 morphology proven (body plans co-vary with niche), Phase 7 optimization done, and the determinism decision in §9.2 settled — because choosing Rapier retroactively costs you the SIMD work from §7.5.

**The readiness signal.** Do this when `thrust`/`turn` has become the thing limiting how interesting the sim is — when you look at an evolved organism with an elaborate body and find that how it *moves* is the boring part. If locomotion still feels sufficient, articulation is premature, and the volumetric-3D branch will buy more novelty per unit of work.

**If you do take the articulation branch, do it in 2D first.** Two-dimensional joint dynamics are dramatically simpler than 3D, and the same "don't debug two new things at once" logic that put morphology before dimensionality applies here. Learn torque-driven locomotion, contact stability, and the determinism story in the easy setting; port to 3D after.

### 9.4 What will break regardless

Be honest about this up front rather than discovering it:

- **Seeds won't replay across the transition.** Determinism holds within a version, not across dimensionality changes. Version the param blob and keep old sim cores runnable if replays matter to you.
- **Evolved populations mostly won't transfer.** Genomes will *load* if you follow §9.1 — that's the point of the hedges — but a lineage that solved 2D foraging is not competent in 3D. Expect to re-evolve. Loading is still worth it: seeded starts beat random ones.
- **Performance budget resets.** 27-cell neighbor iteration and per-part collision will cost you roughly an order of magnitude. Treat the Phase 7 optimization work as a prerequisite for volumetric 3D, not as cleanup afterwards.
- **Articulation resets locomotion evolution outright.** Once movement emerges from joint torques rather than a `thrust` primitive, every evolved swimmer and forager is incompetent again. Bodies and brains transfer as genomes; competence does not. Budget for re-evolving from near-scratch, and keep the rigid-body world runnable alongside rather than replacing it.

### 9.5 A second, independent track: centralized multi-client service

Everything above concerns the *world*. This concerns *where it runs*, and the two are orthogonal — either can happen without the other.

**This track is currently deprioritized:** the stated goal is a static app with no server (§7.7). Kept here because the hedges below cost nothing and because the lockstep variant needs only a dumb relay rather than a simulation server — which is the one version of this that stays compatible with static hosting.

The pivot is smaller than it sounds, because §2.1's architecture is already server-shaped by accident: the sim does no I/O, sits behind `step()`/`snapshot()`/`serialize()`, never touches the DOM, is deterministic from a seed, and already runs in a separate execution context the client talks to via a command queue. The client already treats the sim as a remote thing it messages. **The pivot is a transport swap** — `postMessage` + SharedArrayBuffer becomes a socket — and the §7.2 native shell is essentially the server binary already.

**The expensive part is the snapshot pipe.** Zero-copy reads from WASM memory become bytes on a wire, and the arithmetic is unforgiving: 50k agents × ~40 bytes × 60Hz ≈ 120 MB/s per viewer. Infeasible. You'd need viewport-based interest management, quantization (i16 fixed-point positions, not f32), delta encoding against the last acked snapshot, and a lower send rate with client-side interpolation. Well-trodden MMO work, but it is the bulk of the effort. Then: an interpolation buffer in the renderer (snapshots now arrive 50–150ms stale and irregularly), session lifecycle, auth, Postgres instead of IndexedDB, and ops.

**The option determinism buys you.** Because the sim is bit-exact reproducible, you can skip nearly all of that: ship the seed, params, and command log, and let every client run its own copy in WASM. Clients stay in sync because the simulation is deterministic. Bandwidth drops to a trickle of commands. This is the RTS lockstep model and it suits an evolution sim well — worlds are long-lived and command volume is tiny. It demands genuinely bit-exact cross-platform determinism, which is precisely what the `libm` discipline in §7.4 protects. Late joiners still need a state snapshot to catch up, and world size is capped by the weakest client.

**Hedges worth taking now:**

| Hedge | Why |
|---|---|
| **All mutations go through a serde-serializable `Command` enum** — including UI actions like "place food here" | Network transport, replay, and lockstep all fall out of a command log. The biggest one. |
| Tick number on every snapshot; `apply_at_tick` on every command | Costs nothing now; essential for ordering and lockstep |
| Thin `SnapshotReader` in front of the buffer | Otherwise the renderer couples to raw SoA memory offsets |
| No `static` mutable state in the sim core (§7.2) | A server must run many worlds in one process |

**Difficulty by scope:**

| Scope | Effort |
|---|---|
| Each user gets their own server-hosted world | Small — multi-tenancy, no shared state |
| Many spectators watching one world | Moderate — the snapshot pipe is the work |
| Shared world, users interact with it | Significant — authority, ordering, validation |

The real constraint isn't engineering difficulty. Headless evolution sims are CPU-bound by nature, so a hundred concurrent worlds at 60Hz is a genuine hosting bill, and that will shape the product more than any code decision. Lockstep sidesteps it neatly by pushing compute back to the clients.

---

## 10. Known failure modes

Worth knowing in advance, because you will hit most of these:

| Symptom | Likely cause |
|---|---|
| Agents jitter in place, never learn | Selection pressure too weak, or metabolic cost too low to make idleness fatal |
| One clone sweeps the world, diversity → 0 | World too homogeneous; add spatial/temporal niches |
| Lineages drop or disconnect their sensors | Food too dense or too uniform for perception to pay; check plant patchiness and regrowth (§5.1, §5.3) before sensor costs |
| Total extinction in the first minutes | Energy input too low, or mutation rate too high (error catastrophe) |
| Brains bloat, sim slows over hours | No metabolic cost on brain complexity |
| Predators evolve then wipe everything out | Attack too cheap relative to prey energy; add prey refugia |
| Communication never evolves | Almost always insufficient spatial viscosity — offspring aren't near kin |
| Behavior looks smart but isn't | You're pattern-matching. Check against a random-weights control run |

That last one is a real methodological hazard. Keep a random-brain control population available to compare against; humans are extremely good at seeing intent in moving dots.

---

## 11. Open questions

1. ~~2D-sim/3D-render vs. fully volumetric 3D?~~ **Decided:** 2D-sim/3D-render for V1, volumetric 3D as a long-term target (§9).
2. ~~Asexual only for v1, or both reproduction modes from the start?~~ **Decided:** asexual V1, sexual pathway unlocked at Phase 6 with the hedges in §3.4.
3. ~~TypeScript sim core first, or start in Rust/WASM?~~ **Decided:** Rust/WASM from day one, single crate with wasm and native shells (§7).
4. ~~Target scale?~~ **Decided:** two profiles — 50k headless on the native shell (aspirational, flexible), and whatever holds 60fps in the shared web build. Agent count is runtime config, not a constant (§6).
5. ~~When does multi-segment morphology land?~~ **Decided:** Phase 5, before volumetric 3D, as rigid genome-derived bodies (§3.5). Rapier deferred to articulation, and possibly skipped entirely (§9.2).
6. ~~Watch, experiment, or share?~~ **Decided:** experiment *and* share, deployed as a static app early. Consequences: seed URLs are a first-class feature (§6), the renderer must be presentable from Phase 2 (§8), host choice is constrained by COOP/COEP (§7.7), and §9.5 is deprioritized.
7. **What is the minimal viable founder? Open.** Phase 1 hands every founder the full sensory suite because no operator can add one back (§3.3), so the question could not be asked. Once the structural operators exist the default should invert toward the simplest organism that closes the loop — but how simple stays viable against the §5.5 economy is a measurement nobody has taken.
