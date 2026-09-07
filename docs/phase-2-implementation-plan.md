# Phase 2 - Implementation Plan

Ordered implementation of **genetic architecture** from
[`synthetic-evolution-spec.md`](synthetic-evolution-spec.md) section 8. Phase 1's human
acceptance and tuning evidence remain in
[`phase-1-implementation-plan.md`](phase-1-implementation-plan.md).

**Status: implementation started with the approved M1 allocator foundation.**
`World` integration, concrete memory budgets, and the remaining M0 choices are still
pending. The agreed scope includes add/remove sensors, basic manual checkpoints,
and a structural-null comparison before acceptance. Unapproved implementation
choices in M0 remain proposals, not additions to the normative spec.

## Scope and status

The success criterion is **brains grow in complexity and distinct species appear**,
judged by a human with reproducible, multi-seed evidence and a clearly described
scalar-heredity control and an approved structural-null comparison alongside. More
genes or more cluster labels alone are not evidence that useful complexity evolved.

| Milestone | Deliverable | Depends on | State |
|---|---|---|---|
| M0 Design decisions | Approve contracts, including the structural-null protocol, and update the spec | Human review | allocator approved; budgets and other contracts open |
| M1 Variable-length storage | Bounded arenas and transactional birth storage | M0 allocator approval; budgets before world integration | in progress: allocator foundation; world integration pending |
| M2 Neural structural mutation | Connection/neuron operators and topology-safe control | M1; M0 mutation/control decisions | not started |
| M3 Sensors and founders | Sensor operators and configurable founder composition | M2 | not started |
| M4 Distance and species | Deterministic clustering with stable species identities | M3; M0 distance/species decisions | not started |
| M5 Phylogeny | Stable ancestry and bounded, exportable history | M4; M0 history decision | not started |
| M6 Observation and sharing | Species telemetry, browser views, and protocol integration | M4-M5 | not started |
| M7 Manual checkpoints | Portable save/load with exact continuation | M5-M6 | not started |
| M8 Founder experiments | Multi-seed viability and structural-evidence comparisons | M3 to start; M4-M6 and M0 structural-null protocol for full evidence | not started |
| M9 Acceptance | Mechanical evidence and human judgment | M7 and completed M8 evidence | not started |

Each milestone can span several PRs. Keep allocation, mutation, classification,
history, and presentation as separate concepts rather than expanding `World` into
their implementation. Systems still take the slices they need.

Milestone numbers identify deliverables, not a strictly serial schedule. M8's
founder-viability work can start after M3 and overlap M4-M7. Species conclusions need
M4-M6's measurements, and useful-structure claims need M0's approved structural-null
protocol implemented in M8. M7 enables checkpoint-assisted inspection when available;
it does not block experiments, but remains required for M9.

Each core milestone wires its parameters, validation, control-protocol metadata, and
changed contracts through the existing native/WASM/browser boundaries in the same
change. M6 adds the richer observation UI; it is not permission to leave earlier
milestones with stale telemetry headers, broken controls, or unshareable parameters.

**In scope:** variable-length genomes and compiled brains; add/remove neuron and
connection; connection enable/disable; add/remove sensors using the existing
modalities; genetic distance; species assignment; phylogeny; the telemetry and
presentable 2D UI needed to observe these; manual checkpoint save/load in native and
browser shells; simpler-founder experiments and the required structural-null
comparison.

**Not in scope:** sex or reproductive isolation enforcement, new sensory modalities,
new effector kinds or effector-structure mutation, predation, signaling, body
mutation/morphology, gene duplication, evolvable meta-genes, Three.js, periodic
autosaves/retention scheduling, checkpoint migrations, timeline scrubbing/indexing,
compression/storage optimization, automatic sweeps, or the Phase 7 performance
rewrite. The mutation table in spec section 3.3 is not a requirement to enable every operator in this phase. Gene
duplication remains Phase 6; meta-gene scheduling remains a later explicit decision.
Existing serialized hedges stay intact.

## What already exists, and what actually needs to change

| Existing surface | Phase 2 seam |
|---|---|
| `sim-core/src/genome.rs` | Typed genes, innovation references, canonical kind/ID ordering, validation, and complexity accounting already exist. Structural edits must preserve their contracts. |
| `sim-core/src/arena.rs` | Handles already carry `(offset, len)`, but allocation is fixed-stride. Increasing a founder-sized stride is not variable-length allocation. |
| `sim-core/src/world.rs` | Variable-data arena strides and genome scratch currently derive from `FounderPlan`. Compiled neurons, synapses, and sensors must grow with genomes, not just the gene buffer. |
| `sim-core/src/tick.rs` | Births copy a parent into reusable scratch, mutate, spawn, then transfer energy only on success. Preserve that ordering and make partial capacity failure recoverable. |
| `sim-core/src/founder.rs` | Founders share one innovation template. Control randomization indexes a founder-sized fan-in table, which cannot handle a structurally mutated descendant. |
| `sim-core/src/brain.rs`, `perceive.rs`, `effectors.rs` | Compilation already resolves innovation IDs to local slots; tick systems consume slices. Reuse this instead of evaluating genes in the tick. |
| `sim-core/src/agents.rs`, `pool.rs`, `snapshot.rs` | Species storage and slot incarnations exist, but a recycled parent slot is not persistent ancestry. The snapshot already carries species IDs. |
| `shells/native/src/metrics.rs`, `metrics_reader.rs`, `diagnose.rs` | Telemetry measures exact genome variants, not species. The reader explicitly requires Phase 1 and the current control protocol. |
| `shells/wasm/src/lib.rs`, `web/src/inspect/model.ts` | On-demand inspection already carries variable-length genes/activations and parent slots. Add stable ancestry without confusing it with selection identity. |

Keep the accepted Phase 1 configuration reproducible during the foundation work.
There is no need to rebuild the worker, renderer, command queue, or sensorimotor loop.
The known 5k-agent simulation-throughput limit remains a recorded baseline, not a
reason to promise Phase 7 scale here.

## M0 - Decisions to approve before their implementation

**Including sensor addition/removal, basic manual checkpoints, and a structural-null
comparison before acceptance in Phase 2** is settled by this planning discussion.
The approved checkpoint and evidence requirements are in spec sections 7.10 and 7.8;
the actual structural-null protocol still needs M0 approval. Recommendations in this
section deliberately expose the choices that
would otherwise become accidental selection pressures or misleading measurements.
Approve them before the dependent milestone, and record the resulting contracts in
the relevant spec sections in the same change.

### D1 - Storage budgets and failure semantics

**Decided for the allocator foundation:** preallocated variable-length arenas with deterministic
address-ordered first-fit allocation and adjacent-free-span coalescing. Keep handles
stable, with no compaction, relocation, or backing-buffer growth in the tick. Size
allocator metadata for the maximum simultaneous blocks; it must not allocate either.
Keep fixed-size parts storage simple.

The approved first M1 slice adds `arena::VariableArena` and its coverage without
replacing `World`'s existing fixed-stride arenas or changing defaults. Its fallible
constructor bounds both data and metadata buffers; allocation reports block-limit,
total-space, and fragmentation failures explicitly. World integration is the next
slice, after the budgets below are approved.

Separate per-genome limits from aggregate arena capacity. Both are runtime config:
maximum genes, neurons, connections, sensors/rays, and total storage budgets. Derive
scratch and compiled-buffer requirements with checked arithmetic. Validate the
whole eager core footprint, not only one gene array, and account separately for
shell snapshots/transports and paired evolving/control worlds when selecting a
profile. A configured ceiling is not a guarantee that the host has that much free
memory. Concrete web/native budgets need measured footprints before approval.

The pre-integration native baseline at `00988c1`, using the existing `footprint`
test's allocation tally, requested 77,869,988 bytes (74.3 MiB) while constructing the
default 5,000-slot world, and 31,330,984 bytes (29.9 MiB) at 2,000 slots. These are
cumulative construction requests, including temporary allocations, not retained
heap, process RSS, or whole-browser memory. The existing 96 MiB test ceiling remains
a regression alarm, not approval of a runtime budget.

An operator that would exceed a per-genome limit should be declined atomically;
it must not leave half a split connection or half a sensor. If the completed child
cannot claim all its arena blocks, refuse the birth without charging its parent.
Release every partial claim. Specify RNG/innovation consumption on failed attempts
so identical failures replay identically.

First-fit can fail from fragmentation even when total free space is sufficient.
Expose declined operators, refused births, capacity use, and fragmentation through
opt-in diagnostics. This allocator outcome is approved; concrete world resource
pressure remains part of budget/integration approval. Never silently drop genes or
mint energy to conceal it.

### D2 - Structural mutation semantics

**Recommendation:** bounded per-birth attempts in a fixed operator order, using
world-owned innovation IDs and reusable scratch. Keep scalar mutation behavior
unchanged initially. Zero structural rates must consume no extra random draws and
must preserve the Phase 1 trajectory.

| Operator | Proposed contract requiring approval |
|---|---|
| Add connection | Join existing neurons; recurrence/self-connections are legal. Do not add duplicate endpoint pairs; a retained disabled edge is re-enabled with its existing ID. |
| Remove connection | Physically delete a selected connection. This is distinct from disabling it, which retains its innovation and storage/metabolic cost. |
| Enable/disable connection | Toggle retained wiring without changing its ID. |
| Add neuron | Split an enabled connection: retain it disabled, add one neuron and two fresh-ID connections atomically. Set initialization through runtime parameters; a CTRNN split is not assumed behavior-neutral. |
| Remove neuron | Remove an eligible non-oscillator neuron plus incident connections. Protect sensor targets and effector sources; removing an organ is a separate operator. Preserve the always-present oscillator scaffold in spec section 3.2. |
| Add sensor | Add a supported modality plus one fresh target neuron per channel atomically. New channels become useful through the connection operator; they must be reachable from a sensor-poor founder. |
| Remove sensor | Remove the sensor gene, leaving its target neurons coherent and available to the neural operators (spec section 2.2c). |

**Physical deletion and D4 must be approved together.** Deleting an edge discards its
innovation marker; re-adding the same endpoints gets a fresh ID, unlike re-enabling a
retained edge. That can increase historical distance and create cluster labels
without a corresponding change in functional wiring. Removing/rebuilding a neuron
and its connections raises the same measurement question. This is not just a choice
of removal rates.

**Recommendation:** keep physical neuron/connection removal rates at zero in shipped
defaults until M4's deletion/recreation comparisons inform joint D2/D4 approval.
Implement and exercise deletion with explicit test/experiment parameters in M2; a
zero shipped rate must not become an excuse to leave the operator unimplemented.

Use fresh monotonic IDs for new structural origins, retain IDs for inherited genes,
and do not add a NEAT-style global event-deduplication registry implicitly. Define
ID exhaustion without wrapping into `NULL_ID`. Canonical ordering is by kind and ID,
not a global ID-sorted slice; all operators, distance, and crossover must agree.

Approve rates for neuron/connection removal, which section 3.3 does not currently
supply, and the exact enable/disable sampling rule. Rates, initialization distributions, and meaningful
growth limits belong in `SimParams`. Preserve the existing cost accounting, including
charging disabled connections; changing that policy is a separate design decision.

### D3 - What the random-brain control means with evolving topology

**Recommendation:** retain a scalar-heredity control. Control offspring undergo the
same structural mutation rules, inherit non-neural genes and topology, and redraw
all neural scalars using fan-in derived from their actual genome. Founders still
match the evolving world's founders exactly for the same seed/params.

This control can evolve topology and sensors. It tests cumulative **neural scalar**
inheritance, not the absence of all evolution. It cannot alone establish that
structural growth is adaptive. Label that limitation in reports and UI.
Do not reset every child to the founder topology: that changes both complexity
costs and sensory opportunity, confounding the comparison.

**Decided: a structural-null comparison is required before Phase 2 acceptance.**
M0 must approve its protocol, not leave the presence of a null model to M9. Record
which structural contribution or inheritance it disrupts, what quantities it
preserves, how it controls or accounts for metabolic costs and sensory opportunity,
the seed/cohort matching, the measured outcomes, and the claims it cannot support.
An arbitrary rewiring or founder reset is not automatically a valid null.

M8 implements and reports that experiment alongside the scalar-heredity comparison,
with distinct protocol identities and results. No implementation algorithm is
approved by this requirement alone. Early founder-viability runs may proceed
without it, but they cannot establish useful structural adaptation or complete M8's
evidence. A missing or inconclusive comparison is not a pass based on human
impressions alone; human judgment remains necessary once the evidence is available.

Replace founder-position-dependent randomization before enabling structural
mutation. Version the control protocol in telemetry rather than silently changing
the interpretation of Phase 1's `randomized_at_birth` records.

### D4 - Genetic distance over typed genes

**Recommendation:** retain spec section 3.4's formula, counting disjoint/excess genes over
innovation-bearing kinds, aligned within each kind. Normalize by the larger total
number of innovation-bearing genes, with a denominator of at least one. Average
weight differences over matching connections, including disabled ones; use zero
when there are no matches. Body/meta genes have no innovation ID and do not
participate in that structural term.

Approve this interpretation and the coefficients/threshold before implementation:
connection-only distance is another reasonable choice, but it under-reports organ
gain/loss. This proposal does not add bias, tau, sensor-parameter, or enabled-state
distance terms. Similarity by this formula is not full genome equality. Avoid an
implicit small-genome normalization heuristic; any such policy must be explicit.

**D2's physical deletion changes how this distance should be interpreted.** Matching
surviving IDs still marks shared origin, but a deleted then independently recreated
gene no longer matches its ancestral counterpart. Compare retention/toggling with
deletion/recreation of otherwise equivalent wiring, including repeated churn and
threshold crossings, before approving nonzero shipped removal rates. Record which
new species labels reflect marker turnover rather than functional divergence. Do
not silently recover old IDs by endpoint matching or add a registry to hide the
effect; either would be another mutation/ancestry design choice.

### D5 - Stable species, not labels that change every frame

**Recommendation:** classify founders and newborns against immutable representative
genomes of active species. Select the nearest compatible representative with
`distance < threshold`, breaking ties by ascending species ID; otherwise create a
fresh species ID. Process births/deaths in the existing agent-index order and do
not consume simulation RNG for representatives or colors.

Membership stays fixed for an individual's lifetime, consistent with its immutable
genome. Representatives must outlive the original representative agent. Retire an
active species when its last member dies; historical IDs are never reused. Specify
same-tick extinction/recolonization and root-species creation explicitly.

Representatives need their own bounded storage; never borrow a live agent's arena
handle after death. Since this is observational in the asexual phase, running out
of classification storage must not prevent reproduction. Approve either a capacity
guarantee covering every active species or an explicit unavailable-classification
state; silently merging incompatible species is not acceptable.

Coefficients, threshold, and resource limits are configuration, not adaptive knobs
that chase a desired species count. Define whether they require a new run; the
recommendation is construction-time configuration for stable historical meaning.

### D6 - Persistent ancestry without unbounded tick allocation

**Recommendation:** world-local, monotonic birth identities with two stable parent
references, distinct from reusable pool slots and their UI incarnations. Preserve
the existing `parent_a`/`parent_b` fields and serialized hedges; do not silently
reinterpret old parent-slot payloads as persistent IDs. Represent wide IDs exactly
across JSON/JS, as is already done for ticks and seeds.

The primary viewer should be a **species-origin graph**, with the birth of a new
species linked to its founding parent's species and tick. It is not an archive of
every organism or a claim that all members have one species ancestor. The data/view
must tolerate two parents without enabling sexual reproduction.

Keep active classification state separate from an opt-in bounded history/event
buffer. Native and browser shells drain events between step batches and own archival
I/O; browser history can persist in IndexedDB, native history in exported records.
This is history persistence, not full-world checkpointing or resumption. M7 owns
that separate capability; exporting M5's ancestry graph cannot resume a world.

Approve retention limits, pruning semantics, and overflow handling before M5.
Recommended overflow behavior is an explicit gap/truncation marker, never altered
ecology or a silently complete-looking tree. Pruning must preserve referenced
ancestor identities or mark them as unavailable. A missed intermediate snapshot
must not erase a speciation or extinction event. Observer attachment, drain timing,
and pruning must not change simulation RNG or trajectories.

## M1 - Variable-length storage and birth transactions

**First slice:** `arena::VariableArena` supplies the approved first-fit/coalescing
backend with the existing `Block` handle. Differential tests compare it with an
elementwise occupancy model, and allocation-counting covers reuse, coalescing, all
refusal kinds, and reset without invoking an allocating element default. The old
`Arena` and all `World` callers remain unchanged. This does not complete M1.

Implement D1 in `arena`, then integrate genome, neuron, synapse, and sensor arenas,
scratch sizing, and the spawn/despawn lifecycle in `world`. Audit effectors even
though their structural operators are deferred. Keep slice-based compilation and
all zero-length-handle behavior. Reject invalid external sizes at the boundary;
internal capacity exhaustion is a defined outcome, not a release-build panic.

**Done when:** allocation/free differential tests cover mixed sizes, fragmentation,
coalescing, full capacity, zero-length blocks, and reuse without stale data.
Allocation counting covers successful and failed claims and real birth/death churn.
World-level tests force failure in each constituent arena and prove no leaked blocks
or lost parent energy. A world hosts genomes both larger and smaller than its founder
and compiles/steps them correctly.

Reassess state hashing: raw offsets may remain irrelevant, but free-span state that
affects future birth success is not. If hash coverage changes, commit that separately
from dynamics changes with its own references and explanation. Preserve trajectories
for the Phase 1 configuration even if its strengthened hash changes.

## M2 - Neural structural mutation and topology-safe control

Implement D2's neuron/connection operators and D3's topology-safe scalar-heredity
control together with integration into births. D3's separate structural-null
experiment is implemented in M8 under its M0-approved protocol. Extract the mutation
pipeline as a slice/buffer-based system rather than adding each operator to
`tick.rs`. Extend validation to generated IDs, endpoint uniqueness, sorting, capacity
bounds, and finite compiled parameters.

**Done when:** property tests run long edit sequences over variable topologies and
assert coherent references, inherited IDs, fresh IDs, bounded size, and deterministic
output. Every enabled operator has a test proving it actually fired; a no-op cannot
pass merely by leaving a valid genome. Smaller offspring claim less storage and have
recomputed complexity costs; the parent's genome remains unchanged. Control
randomization handles changed lengths and fan-in without changing non-neural
content, and fixed-topology control behavior remains covered. Update telemetry's
control protocol and its reader at this milestone, before emitting changed controls.

Pin a structural-mutation scenario shared by native and WASM, including actual
births, removals, and capacity failures. Keep structural rates disabled in shipped
defaults until the operator/control contract is approved and integration is ready;
enable them deliberately, not as an incidental consequence of a new default field.
Physical deletion also retains D2/D4's joint approval gate; its forced scenarios
exist even while shipped deletion rates remain zero.

## M3 - Sensor mutation and configurable founders

Implement D2's sensor operators for vision, chemo, and energy interoception only.
Constrain sensor parameters to supported channels and the configured spatial-query
envelope; retain zero elevation. Sensor count/ray caps must be separate from the
founder's starting counts.

Make founder sensor counts, hidden/oscillator counts, and initial connectivity
runtime configuration. Reuse the current fan-in scaling and shared founder IDs.
Retain thrust, turn, ingest, and brain-gated reproduce: "minimal" must not remove
reproduction simply because section 3.3's illustrative list abbreviates the loop.
Keep body/meta hedges. The current dense founder remains a reproducible baseline;
switching the shipped founder is M8's measured decision, not this milestone's guess.

**Done when:** removing a sensor leaves a valid runnable brain; adding every supported
modality binds the correct number of channels and changes perception and metabolic
load. Deterministic edit sequences demonstrate that a sensor-poor founder can acquire
an eye and connect it to effectors. This proves reachability, not evolved usefulness.
Counts, scratch, compiled sensors, inspection, and parameter validation all agree
for sparse, dense, and zero-vision founders.

## M4 - Genetic distance and species assignment

Implement D4/D5 as separate systems with world-owned state. Reuse innovation
alignment conventions without coupling classification to crossover. No mating,
fitness score, species protection, or species-based energy allocation is introduced.

**Done when:** distance is finite, nonnegative, self-zero, symmetric, and matches
hand-worked disjoint/excess/weight cases, including empty genomes, missing matches,
and disabled connections. Test exact threshold behavior and deterministic tie
breaks. Species membership survives representative death and pool reuse; extinction
is emitted exactly once. Totals by active species sum to population, or explicitly
account for unavailable classification under the approved capacity policy.

Include repeated multi-world and native/WASM agreement with real species creation.
Measure classification and representative-storage cost under high species churn;
do not add an all-pairs population distance pass every tick.

Include D2/D4's controlled deletion/recreation cases in distance and classification
coverage. Report the distance and cluster changes against retained-ID counterparts
so approving removal rates accounts for their measurement effect, not only whether
the resulting genomes validate.

## M5 - Stable ancestry and retained history

Implement D6's identities, species-origin events, history retention, and shell-side
archival. Link births using persistent parent identities at birth time, never by
looking up a potentially recycled slot later. Preserve both parent positions in the
record and keep the second empty for asexual births.

**Done when:** a parent dies, its slot is reused repeatedly, and old lineage links
still identify the correct parent. Species history survives extinction and can be
exported/read back after a reload. Pruning and buffer overflow are visible, bounded,
and cannot create dangling links presented as valid ancestry. Exercise a synthetic
two-parent graph in data/view tests without changing reproduction.

Keep authoritative IDs/classification counters covered by deterministic hashing;
optional observation buffers and shell archive timing must not determine evolution.
History-disabled headless runs pay no archival/instrumentation cost.

## M6 - Telemetry, browser observation, and sharing

Extend native metrics with population by species, species count, separate neuron and
connection sizes, genome-size distribution, structural changes, and capacity/history
availability. Keep exact genome variants as a distinct metric and preserve paired
control values. Update header phase/control/schema, reader, human/JSON diagnosis,
and fixtures together. Approve explicit compatibility or rejection of older schemas;
never interpret Phase 1 data as measured species diversity.

Use existing snapshot species IDs for a species-color mode. Do not alter the
genetically visible signature to paint clusters. Add species selection, representative
genome comparison, and an incrementally updated, prunable phylogeny view. Pull detail
on demand rather than widening every snapshot with genomes or history.

Wire new parameters and control semantics through native CLI/config, WASM validation,
worker/client messages, inspection decoding, run controls, and seed URL round-trips.
Preserve selection incarnation guards and shared/transferable frame ownership.

**Done when:** native reports and browser counts agree at the same completed tick;
events are retained between rendered/sample frames; representative comparisons
correctly display structural differences. Both transports support pause/step,
reseed, inspection, species views, and ancestry with no stale-world responses.
The visual-check skill confirms usable desktop/narrow layouts and a responsive
canvas. Seed URLs reconstruct the intended configuration without lossy IDs.

## M7 - Manual portable checkpoints

**Approved scope:** bring basic save/load forward from Phase 7, after variable-length
storage, species, ancestry, and their browser surfaces settle. This is separate from
M5's history export and M6's observation UI, and can proceed alongside M8 experiments.
Checkpoint-assisted inspection is an enabler, not a prerequisite for gathering
evidence; completing M7 is still required before M9 declares Phase 2 complete.

Implement spec section 7.10's shared, versioned full-world checkpoint format. The
native shell reads/writes files; the browser offers download/import through the
worker/WASM boundary. Native and WASM builds with the same simulation-compatibility
identity must exchange checkpoints within the receiving host's resource limits.
Retain the originating seed/run provenance. Reject incompatible versions explicitly;
no migration support is required.

Capture a consistent between-ticks boundary. Core encoding/decoding operates on
memory; all file/browser I/O and scheduling belong to the shells, never the tick.
Preserve every future-relevant value, including parameters and heredity mode, RNG,
tick, recurrent neural state, energy residuals and ledger, genomes, pool/arena
allocation state, innovation/identity counters, species representatives and ancestry
metadata, and queued commands with their application order/ticks. Rebuild derived
caches, scratch, and render/transport buffers without reseeding or resetting
authoritative state. Saving must not advance the world or consume RNG.

Validate format/compatibility, lengths, resource limits, handles/free spans, genome
references, and numeric invariants before accepting restored state. Stage loading so
a rejected file leaves the current world intact. Browser imports open paused with a
fresh snapshot and invalidated old selections, responses, and scheduling state; they
must not execute a command twice or let the old worker publish into the new world.

Keep external history archives separate. A resumed world starts a clearly identified
history segment linked to its checkpoint origin, not silently appended after events
from the old world's later future. Do not claim the checkpoint contains a complete
historical archive. For resumed paired experiments, restore both evolving and control
worlds at the same tick with their original protocol; never substitute a new control.

**Done when:** uninterrupted and save/load/continue runs agree at the save boundary
and after further ticks, for both heredity modes and in both native/WASM transfer
directions. Cases must actually exercise structural mutation, slot reuse, fragmented
arenas, species creation/extinction, nonzero recurrent state, compensated energy,
and future queued commands. A successful deserialize or an immediate matching hash
alone is insufficient. Reject truncated, malformed, incompatible, and oversized
files without replacing the live world. Browser save/import works on both transports
and remains usable on narrow layouts; a native-produced checkpoint can be inspected
in the browser and continued without replaying from its seed.

Periodic autosaves, retention scheduling, cross-version migration, timeline
scrubbing/indexing, compression, and storage optimization remain deferred. This
milestone provides manual exact continuation, not the Phase 7 overnight-run manager.

## M8 - Founder and complexity experiments

Start founder-viability experiments after M3 using the existing native
paired-telemetry workflow. Compare a small, declared set of founder compositions,
including the accepted dense baseline and chemo-led sparse candidates. Report at
least three seeds per configuration with ranges and variance. Measure viability,
reproduction, neurons/connections/sensors, capacity pressure, energy flow, and
simulation throughput alongside the scalar-heredity control. Before M4-M6 land,
mark species-based evidence unavailable rather than infer it from genome variants.

Complete the evidence with species persistence and M0's approved structural-null
experiment once the required measurements are available. Implement that protocol
in this milestone, update affected reporting/UI contracts, and publish its
multi-seed results separately from the scalar control's. State confounds and
uncertainty; structural-null evidence must address useful inherited structure, not
just total gene counts. M8 is not complete with founder viability alone.

When M7 is available, use it to retain selected moments for inspection without
replaying the entire run. Before then, use live observation and seed replay.
Record checkpoint provenance and retain matching control checkpoints when an
experiment will be resumed or compared from that tick.

Tune existing/new parameters rather than changing mechanisms in response to outcomes.
Retain the current energy accounting and disabled-connection cost policy unless a
separate human-reviewed design change is justified. Capacity saturation is a warning
that a growth result may reflect allocator limits rather than metabolic selection.

Produce an unranked shortlist for a human to watch; do not maximize species count or
brain size. A simpler shipped founder requires observed viability and human approval.
Document any changed default's rationale on its `SimParams` field, retaining prior
tuning notes, and update golden references with the deliberate behavior change.

## M9 - Acceptance and landing

Run checks covering each changed surface from `AGENTS.md` and
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). In particular, structural
birth/death runs must cover energy conservation, no hot-loop allocation, deterministic
hashing, and native/WASM agreement; founder-only scenarios are insufficient.
Generated contracts and real-browser checks are required when their surfaces change.

Human review must distinguish useful inherited complexity and persistent species
from transient mutations, deletion-driven marker turnover, threshold-created labels,
or a misleading scalar-only control comparison. Require M8's completed
structural-null evidence under the M0-approved protocol, alongside its scalar
control results; no structural comparison means acceptance is blocked, not waived.
Record seeds, complete params, source revision, both control protocols, duration,
metric variance, limitations, and what was observed. M7 must also be delivered even
though experiments did not depend on it. Mechanical success alone cannot mark this
phase complete.

Every implementation PR lands on a branch and receives a post-opening diff review.
Keep independent golden-hash causes in separate commits, each with its own reference
update and explanation. Review fixes after opening a PR are new commits. Update the
status table only for delivered work; an allocator-only slice does not complete M1,
and passing its mechanical checks does not complete the phase.
