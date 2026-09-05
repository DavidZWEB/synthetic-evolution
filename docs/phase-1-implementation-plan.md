# Phase 1 — Implementation Plan

Working breakdown of `phase-1-build-plan.md` into ordered milestones. The build plan lists
*deliverables*; this lists *build order*. They differ because the golden hash — the highest-value
test in the project (spec §7.8) — only becomes possible once the tick exists, so several earlier
deliverables cannot be verified end-to-end until M8.

Each milestone ends in a state that compiles and has a test proving it. Milestone numbers are
referenced in commit messages.

## Status

| Milestone | State |
|---|---|
| M0 Scaffold | done |
| M1 Foundations | done |
| M2 Pools and arenas | done |
| M3 Spatial hash | done |
| M4 Genome, mutation, crossover | done |
| M5 CTRNN | done |
| M6 Sensors and effectors | done |
| M7 World and economy | done; larder fixed, `k_sensor` unit open — see the budget pass |
| M8 The tick | done |
| M9 WASM shell and renderer | shell, worker, transport and renderer done; sim does not hold 1× at 5k |
| M10 Instrumentation | done |
| M11 Headless telemetry | done |
| M12 Acceptance | not started |

## Cross-cutting rules for this phase

- The forward-compatibility checklist in `phase-1-build-plan.md` is **not** a milestone. Each item
  lands in the milestone that creates the struct it lives in, with a comment naming spec §9.1.
  The whole checklist is audited as one pass at M8.
- The golden hash is not pinned until M8. M4–M8 all change behavior by construction. Every update
  after M8 is deliberate, in the same commit as the behavior change, with a note on why.
- `sim-core` invariants (AGENTS.md) apply from M0. M0 ships a lint test for the two that fail
  silently: no I/O, no platform transcendentals, no `thread_rng`.

---

## M0 — Scaffold

Build-plan task 1.

- Cargo workspace: `sim-core`, `shells/native`, `shells/wasm`.
- Dependencies pinned: `glam`, `rand` + `rand_pcg`, `serde` + `postcard`, `libm`. (`ts-rs`
  arrives at M10 with the inspector that consumes it.)
- Vite + Svelte 5 in `web/`.
- `staticwebapp.config.json` with COOP/COEP `globalHeaders`, in `web/public/` so Vite copies it
  into the `dist/` that Azure deploys — a repo-root config with a build subdirectory is silently
  ignored (spec §7.7).
- Guard test: greps `sim-core/src` for `thread_rng`, `std::time`, `std::fs`, `std::net`, and bare
  `.sin()` / `.cos()` / `.exp()` / `.powf()`. Invariants 1 and 2 fail silently weeks later;
  a short test is cheap insurance.

**Done when:** `cargo test` and `npm run dev` both run clean.

## M1 — Foundations

- `ids.rs` — newtype `AgentId`, `NeuronId`, `InnovationId`, `PartId`.
- `math.rs` — `libm` wrappers, yaw-only quaternion helpers.
- `rng.rs` — PCG wrapper with serializable state (the golden hash folds RNG state).
- `params.rs` — every constant from spec §5.5, serde, `Default`.

**Done when:** params serde round-trip, quaternion op tests, libm wrapper tests pass.

## M2 — Pools and arenas

Build-plan task 2.

- SoA world arrays exactly per spec §2.2a, including the dead fields (`parentB`, `partOffset`,
  `partCount`).
- Fixed capacity, free list.
- Gene and brain arenas with per-agent `(offset, len)`.

Phase 1 has fixed topology, so arena slots are a fixed stride and recycle through the same free
list. Access still goes through `(offset, len)`, so Phase 2's variable-length genomes are a change
of allocator rather than a change to every caller.

Counting-allocator test harness lands here.

**Done when:** spawn/despawn 10k agents in a loop with zero allocations after warmup.

## M3 — Spatial hash

Build-plan task 3.

- Uniform grid, cell size = max sensing radius, counting-sort rebuild.
- Triple-nested neighbour loop with the Z range pinned to `[0,0]` (spec §2.3).
- Brute-force O(n²) reference committed permanently — it is not deleted once the fast path passes
  (spec §7.8).

**Done when:** `proptest` differential parity against the brute-force reference on random
populations.

**The world is a torus.** It wraps in x and y, so distances are minimum-image and cell walks
wrap. The spec does not state a boundary condition either way; wrapping was chosen because a
bounded world hands out edge and corner geography for free, and an artificial refugium is hard
to tell apart from evolved anti-predator behaviour later. The choice is contained in
`spatial.rs` and the movement integrator, so it stays reversible.

## M4 — Genome, mutation, crossover

Build-plan task 4, first half.

- Typed-gene list per spec §3.1, serde. (`ts-rs` moved to M10: nothing consumes the
  generated TypeScript until the inspector exists, and derives with no consumer are
  exactly the pre-building AGENTS.md warns against.)
- Innovation counter as a field on `World`, never a `static`.
- Fixed-topology builder: sensor→input neurons, hidden, oscillators, effector-source neurons.
- Mutation operators: weight perturbation and weight reset only.
- Crossover written and unit-tested by innovation alignment, called by nothing (spec §3.4).
- The `meta` gene kind exists and serializes into its slot, with its mutation operator disabled —
  the same treatment sensor `elevation` gets. Evolvable mutation rates unlock in a later phase.

**Done when:** genome serde round-trips; property test confirms no mutation orphans a sensor's
`target` neuron; crossover of two valid genomes yields a valid genome.

**Decisions taken here.**

*A sensor binds one target neuron per channel.* Spec §3.1 writes the binding as a single
`target`, but §2.2c has modalities returning two to four values — a `vision_ray` writes
distance plus signature RGB. Binding by id rather than by index is the principle that
indirection exists to serve, and one id per channel is what that means when a modality is
wide. `SensorGene::targets` is a fixed array; only the first `modality.channels()` entries
are read.

*`InnovationId::default()` is `NULL`, not 0.* A gene with a field left unset then fails
validation as a dangling reference, rather than silently binding to whichever neuron
happens to sort first.

*The founding topology is fully connected.* Phase 1 has no add-connection operator, so any
connection absent from the founder can never appear in any descendant. Dense means evolution
can reach any wiring by tuning weights toward zero; sparse would forbid some permanently. It
is also why a default genome is 284 genes, 240 of them connections.

*`k_brain` charges neurons and connections, `k_sensor` charges sensors.* Spec §5.2 keeps the
two coefficients apart so eyes can be made expensive without making brains expensive.
`genome::brain_complexity` and `genome::sensor_load` are the two quantities; nothing may be
counted by both. Their *values* are an M7 problem — see the budget note there.

## M5 — CTRNN

Build-plan task 4, second half.

- Euler integration, one step per tick, evaluated from previous activations.
- Evolvable `tau`; always-present oscillator neurons with evolvable period.

**Scale founder weights by fan-in — check this first.** M4 draws initial connection weights
uniform over ±`weight_limit` (±4), and the default topology gives every output and hidden
neuron 24 incoming connections. Summed input is then order ±20 before the activation
function ever sees it, so every sigmoid in every founder saturates on tick one and stays
pinned at 0 or 1. A saturated brain does not respond to its sensors, and weight mutation
moves it far too slowly to recover, so the population looks alive and evolves nothing —
which reads as a tuning failure rather than an initialisation bug.

The standard fix is to scale initial weights by fan-in (draw from roughly
`±weight_limit / sqrt(fan_in)`, or normalise per neuron). Two things this needs: the
initialisation range has to become a `SimParams` field rather than reusing `weight_limit`,
which is a *bound* and not a starting scale; and there should be a test asserting that a
freshly instantiated founder's activations are not all saturated. Do this before judging
anything about M5's dynamics.

**Done when:** a hand-built network produces known outputs, an oscillator neuron oscillates at
its specified period, and a founder brain's activations are distributed rather than pinned.

**Decisions taken here.**

*A brain is compiled at birth, not read from the genome each tick.* The genome binds
neurons by innovation id, and resolving one is a binary search; at 240 connections per
agent per tick that lookup costs several times the arithmetic it feeds. `brain::compile`
resolves every endpoint to a slot once, and the tick is a flat scan over two slices.
The cost is memory: a compiled brain is 3.6 KB per agent and took the default world
from 57 MB to 71 MB, against a 96 MB ceiling. The two levers for getting it back are
recorded on `arena`'s module doc, in the order they should be pulled.

*The fan-in check found the initialisation bug this milestone predicted.* Measured over
64 founders, drawing weights from the full ±`weight_limit` bound pins 52% of wired
neurons against a rail of the sigmoid and cuts the mean activation slope to 0.064 of a
possible 0.25. With `weight_init_scale` at ±2 over √fan-in it is 0% and 0.217.
`brain`'s `without_fan_in_scaling_a_founder_saturates` holds that second measurement so
the first cannot quietly become vacuous.

*The Euler gain `dt/tau` is clamped at 1.* Mutation floors tau at 1e-3 against a dt of
1/60, and an unclamped gain of 16 makes the state alternate and diverge within a few
dozen ticks. The clamp is not a fudge: a neuron faster than the timestep can only be
observed tracking its input exactly.

*An oscillator's state is a phase, and its bias is a phase offset.* A free-running
neuron ignores its input, so it has no potential to integrate and the state slot is
free for the one thing it does need. Feeding `phase + bias` through the activation
makes the bias gene a phase offset rather than dead weight — the second thing an
oscillator can evolve, after its period. Output is ranged to [0, 1] to match the
sigmoid neurons it feeds, so one weight scale is right across a whole brain.

*`dt` is not folded into the compiled brain.* `SimParams` is settable at runtime, and a
baked-in timestep would silently ignore the new value. The reciprocals stored per
neuron — `inv_tau`, `inv_period` — are pure functions of the genome for the same reason.

## M6 — Sensors and effectors

Build-plan task 5.

- Perception (`perceive.rs`): `vision_ray` (spatial-hash raycast → distance + signature RGB),
  `chemo` (concentration + gradient), `interoception` (energy). Writes `sensorScratch`.
- Directional params stored as `(azimuth, elevation)` pairs, elevation clamped to 0, its mutation
  operator absent (spec §4.1).
- Effectors: `thrust`, `turn` (rotation axis pinned to Z), `ingest`, brain-gated `reproduce`.
  All write to an intent buffer; none mutate the world.

**Done when:** an agent with hand-written weights demonstrably climbs a food gradient.
Met — `sim-core/tests/steering.rs`. Read that file's header before treating it as more
than it is: the weights are hand-written, so it is a **wiring** test. It says the
sensorimotor chain closes with every sign convention agreeing, and says nothing about
whether food-seeking *evolves*, which is spec §8's criterion and M12's job. What it
measures is closest approach from eight starting headings including the one pointing
directly away, against a field that diffuses and decays every tick, with a control agent
whose single steering connection is cut.

**Scope: this milestone is wider than its bullet list.** The acceptance criterion needs
three things the bullets do not name — a field to carry a gradient, a way to sample it,
and movement to climb with. So M6 also lands the chemo field and the movement
integrator. Both are *systems*; M8 still owns assembling systems into the normative
11-step tick, along with the `Command` enum and the state hash. That is the seam M5 set:
`brain::step` and `World::step_brains` exist, `tick.rs` does not.

Split across two changes, because it is four modules: **perception** (compiled sensors,
`chemo.rs`, `perceive.rs`) and then **action** (intent buffer, effectors, movement, and
the gradient-climbing test).

**Decisions taken in the perception half.**

*Diffusion is part of the sensor working, not part of the economy.* A deposit lands in
one cell, so with no spreading the gradient is zero everywhere except inside that cell —
an agent one cell away smells nothing and there is no slope to climb. Diffusion is the
mechanism that turns food into a signal, so it lands here rather than at M7. M7 keeps
what it was actually for: plants as the deposit source, the `emit_chemo` effector, and
the conservation ledger.

*Explicit diffusion carries a stability limit, and 1.0 is past the useful part of it.*
The checkerboard mode is scaled by `1 - 2·diffuse` per tick: damped hardest at 0.5,
damped less above it, and at exactly 1.0 it flips sign forever without shrinking, so a
point deposit leaves a permanent grid artefact a nose would chase as if it were food.
Validation still allows [0, 1] — that is where the arithmetic stays bounded — and the
reasoning lives on `ChemoParams::diffuse`, where the next person tuning it will be.

*`Neuron` grew a fourth state field, and M5's shared scratch buffer is gone.* Perception
runs for every agent before any brain steps, so a tick's sensor input has to survive
between two passes; a single buffer cannot hold it. Per-neuron `input` costs 0.6 MB,
which is what a second arena would have cost, and removes a buffer whose clearing was
the caller's problem. `brain::step` consumes and clears it in one move.

*An eye reports nearness, not distance.* 1 at the eye, 0 at the limit of range, and 0
for an empty view. A raw distance would make "nothing there" and "something 60 units
away" different numbers, and would saturate any neuron it reached before the fan-in
scaling of M5 ever got a say.

**Decisions taken in the action half.**

*Every sensor channel is bounded, and directional ones are egocentric.* Designing the
acceptance test found the chemo sensor unusable: it returned a gradient in **world**
coordinates, which an agent cannot act on without knowing its own heading, and Phase 1
has no proprioceptor to tell it. It is also a global fact handed to a local organ, which
is the omniscience spec §2.2c exists to prevent. The gradient is now a unit vector in the
agent's own frame — `+x` ahead, `+y` left — which is what spec §4.1's "gradient
*direction*" asks for anyway. Magnitude went the same way: a raw gradient is a spatial
derivative of order 0.01 and would never move a neuron, a raw concentration accumulates
without limit and would saturate one permanently, and raw energy of 100 saturates on the
first tick. Strength now saturates as `c / (1 + c)` and energy is reported in tanks. The
rule is the one `vision_ray` already followed: **a sensor returns a value in the range
the brain can use**, because the fan-in scaling of M5 is calibrated for inputs near ±1
and an unbounded channel walks straight through it.

*Thrust is unsigned, turn is signed.* Spec §4.2 says thrust is a forward force, and an
agent that can reverse has less reason to evolve a turn. An angular velocity that can
only go one way is a permanent circle, so turn maps a sigmoid's neutral 0.5 to straight
ahead and swings either side of it.

*Intents are struct-of-arrays, not a queue.* All four Phase 1 effectors are
self-directed, so one slot per agent is the whole story and agent-index order is free
rather than something a sort has to restore. `bite` and `grab` name another agent and
will need a real queue; they can have one when they arrive.

*Turn is applied before thrust within a tick.* Either order is deterministic, but this
one makes a turn take effect immediately — otherwise a tick of sensing buys nothing and
steering always lags the thing it is steering at.

## M7 — World and economy

Build-plan task 6.

- Plants as simple non-brained entities.
- Chemo field as a 3D grid of depth 1; per-channel deposit, diffuse, decay.
- Metabolic costs per spec §5.2, including the brain-complexity term.
- Death at zero energy.
- Asexual reproduction with spatial viscosity — offspring spawn near the parent, parent energy is
  split (spec §5.4).
- Explicit `input` / `dissipated` ledger accumulators, so conservation is measured rather than
  inferred.

**The metabolic budget needs a pass here — it is the first point at which it is measurable.**
Two of the constants were left deliberately unresolved at M4, because guessing at three at
once, with no running population to check against, makes the result unfalsifiable.

`k_brain` was settled: 0.00005 rather than spec §5.5's 0.001, because that table's two
statements about it cannot both hold — a 200-unit brain at "~20% of base" wants the term near
0.01 when `base` is 0.05, but 200 × 0.001 is 0.2. At 0.00005 the default topology's 268
neurons and connections cost 0.013/tick, 0.27× base, which is where §5.5 asks for it.

What remains, at the default topology and body:

| term | per tick | vs base |
|---|---:|---:|
| `base` | 0.050 | 1.0× |
| `k_brain` × 268 units | 0.013 | 0.27× |
| `k_sensor` × 16 units | 0.160 | 3.2× |
| `k_size` × size² | 0.180 | 3.6× |
| total | 0.403 | ~250 idle ticks |

§5.5 wants ~2000 idle ticks on a full tank. Both overshooting terms come from defaults chosen
in this repo rather than from the spec: `k_size` is quadratic in `body.size`, which defaults
to 3 (at size 1 that term is 0.4× base), and `k_sensor` is multiplied by channel count rather
than sensor count — this codebase's reading of "0.01 each, weighted by modality", which makes
the default sensor set 16 weighted units instead of 5.

Change these against a running population and several seeds, not by arithmetic. The live
values and their reasoning are on `MetabolismParams::base` and `::k_brain`; update those
comments in the same commit.

**Decided: agents can see plants as well as smell them.** Recorded on
`PlantParams::signature`, where the next person asking will be looking.

Smell was already settled — `scent_rate` deposits into chemo channel 0, and M6's field
and chemo sensor consume it. Sight was not, and the falling-by-default answer was the
expensive one: `perceive::vision_ray` walked only the *agent* grid, and plants are a
separate pool. `k_sensor` charges by channel, so the default sensor set is 12 units of
eye against 4 of everything else; there is no predation until Phase 3 to make seeing
another agent worth anything, and Phase 1 has no remove-sensor operator, so selection
could not have shed the useless organs for the whole of the phase §8's criterion is
judged in. It would have surfaced during the tuning pass above looking like a `k_sensor`
problem rather than a missing query.

An emptied plant stays visible: the site persists and regrows, so blinking it out would
be stranger than leaving it, and telling a fat plant from a bare one now needs the nose —
a selective pressure worth having rather than a defect.

**Plants are fixed sites that regrow in place.** Spec §5.1's "get eaten, and reseed" is
read as regrowth, the weaker of the two meanings and the one Phase 1 needs. Positions
never change, so the plant neighbour grid is built once rather than every tick. The risk
is a static food map that rewards camping over foraging; if an M12 run shows that,
relocating a depleted site is a small change, and `plants.rs` names it.

**At carrying capacity the world absorbs less than its input rate.** The nominal rate is
shared evenly and each plant is capped, so the surplus never enters rather than being
stored anywhere. That is the honest behaviour of a saturated ecosystem, and it is why the
ledger records what `Plants::grow` returns rather than `energy_input_rate * dt` —
conservation has to be measured, not inferred.

**Decisions taken in the metabolism half.**

*The §5.2 constants are charged per tick, not per second.* §5.5 writes them that way
("0.05 /tick") and states its one relationship in ticks: idling fatal within ~2000, which
is exactly `start_energy / base`. Read per second instead, every lifetime in that table
is out by a factor of sixty. It does leave metabolism as the one system whose rate is per
tick while `movement`'s drag is per second, so changing `world.dt` rescales lifetimes but
not coasting — that is the spec's calibration rather than a preference, and re-deriving
the table onto a per-second footing is not something to do by arithmetic.

*`k_brain` charges for disabled connections too.* Settled on `genome::brain_complexity`.
What the term bounds is *genome* growth, not tick cost: AGENTS.md's reason for it is that
genomes bloat and the sim crawls, and what crawls is the 11 KB copied on every birth. A
disabled gene costs all of that and saves only a multiply-add, so charging only for
enabled ones would let a lineage accumulate thousands for free. The cost is that
disabling buys behaviour and no energy back — the right trade while §3.3 has no
remove-neuron or remove-connection operator at all and `k_brain` is the only brake.

*A founder's tank is input; an offspring's is a transfer.* The conservation test caught
this on tick 0 — 200 founders appearing with 20,000 joules the ledger knew nothing about.
Seeding a world is a boundary condition, not an ongoing source, so `spawn_founder`
records it as input. Reproduction will record nothing, because the offspring's energy
comes out of its parent. `World::spawn` itself records neither and says so, since the two
callers account differently.

*The budget still overshoots, and is pinned rather than fixed.* At the default body and
topology an idle agent lasts ~250 ticks against §5.5's ~2000, for the two reasons already
recorded: `k_size` is quadratic in a `body.size` defaulting to 3, and `k_sensor` is
charged per channel. `metabolism`'s `the_default_budget_is_still_the_known_overshoot`
pins the figure so a change is deliberate. It is not fixed here because the plan is
explicit that these move against a running population and several seeds, and reproduction
does not exist yet — that is the next change.

**Decisions taken in the transfer half.**

*An agent eats the nearest plant in reach, not every plant at once.* The more physical
reading of §4.2's "absorb food in contact radius", and it stops a crowded patch being
worth more per tick than a single plant — otherwise a population could feed faster by
standing where plants happen to overlap, which is geography rather than behaviour. Empty
sites are skipped, since a depleted plant stays in the world and would otherwise block a
fuller one beside it.

*A parent is charged only after the birth succeeds.* At the population ceiling a spawn is
refused, and deducting first would destroy energy on exactly the busiest tick of a run.

*`world.rs` is over the size limit and the split belongs to M8.* It is 616 lines of code
before this change, past the 500 that AGENTS.md calls a split. The new logic went into
`feeding.rs` and `reproduction.rs` so it only grew by wiring, but the real fix is M8
lifting the eleven tick-step methods into `tick.rs`, which is that milestone's job
anyway. Noted so it is a decision rather than a drift.

**The budget pass ran. Generation 0 is not viable, and one parameter will not fix it.**
Measured over 20k-tick runs, three seeds per configuration, 150 founders. Every
configuration tried went extinct; what follows is the vector, not a winner.

*The default world cannot feed 150 agents.* They dissipate 0.403/tick each — 3,600/s
between them — against an `energy_input_rate` of 600/s. Carrying capacity is about **25
agents**, and the run starts with six times that. Plants sat at 25% fill while the
population starved, so the larder was never the limit: supply was. Raising input to
2,400/s takes plants to 100% fill and peak agent energy from 112 to 156.

*`reproduction.maturity_ticks` (300) exceeds the default idle lifetime (248).* Not a
tuning preference — two shipped defaults that contradict each other. At the default
budget an agent cannot be old enough and rich enough at the same time, in any world,
however much food there is. This is why config A produced zero births even with plants
at 100%.

*`feeding.rate` is not a lever.* Above about 5 it changes nothing: total energy ingested
was byte-identical at 15 and at 40. Encounters bound intake, not extraction speed.
`feeding.reach` does matter — total ingested scaled 147 → 540 → 1218 as reach went
0 → 6 → 15.

*The metabolic overshoot is real but secondary.* `body.size` 3 → 1 and `k_sensor` per
sensor rather than per channel triples idle life, 248 → 750 ticks. Necessary for anything
else to matter, sufficient for nothing on its own.

*The world is too slippery to hold station, and fixing that is not enough either.* At
`drag` 0.9/s an agent that cuts thrust coasts about 90 units before slowing; a plant's
reach is 5. Making it viscous — `drag` 0.2, `max_thrust` 13, `k_move` 0.003, which holds
top speed and movement cost fixed — lets agents drain a plant instead of skimming it, and
peak energy rises 151 → 200. Births barely move and extinction time does not move at all.
Coast distance is `top_speed / ln(1/drag)`, so the three parameters have to move together.

*What actually remains.* With every lever above applied, a founder produces about **0.13
offspring in its lifetime**. Replacement needs 1.0. That is an eight-fold gap, and no
single parameter tested closes it: random-brain agents simply do not forage well enough
to pay for themselves, which is the behaviour selection exists to produce and cannot
produce until something survives to be selected. Whether Phase 1's defaults should make
generation 0 marginally viable — and how — is a judgment call, not a measurement.
Untested directions worth a look: many more founders, far denser plants, a
`reproduction.threshold` closer to `start_energy`, or a lower `start_energy` so a full
tank is cheaper to reach.

No parameter has been changed on the strength of any of this. The values are a human's
call, and the plan is explicit that a metric improving is not a reason on its own —
weakening a metabolic cost makes every number look better and is how a simulation quietly
stops selecting for anything (spec §10). The known figure remains an idle agent's ~250
ticks against §5.5's ~2000, pinned by `metabolism`'s
`the_default_budget_is_still_the_known_overshoot`.

**The budget pass was measuring an empty larder.** `Plants::new` seeded every site at
zero energy, and filling one takes `max_energy · max_plants / energy_input_rate`
seconds — 400 s, or 24,000 ticks, at the defaults — against a founder lifetime of about
250. Generation 0 lived and died in a world that had received, across the whole of its
existence, roughly 0.6 joules per plant: a tick and a half of upkeep. Everything above
about foraging was therefore measuring the absence of food, and the honest reading of
"0.13 offspring per lifetime" is that there was almost nothing to forage.

That is fixed rather than tuned — `PlantParams::initial_fill`, defaulting to a full
larder, with `World::new` opening the ledger against the stock. An empty world was never
a choice anyone made; it is the state no ecology passes through. The reasoning lives on
the field.

**What the three levers do once the larder is stocked.** Measured over 20k ticks, three
seeds each, 150 founders. `W` warm larder, `M` slower dissipation (`body.size` 3→1.5,
`k_sensor` 0.01→0.0025), `F` denser food (12k plants, input 1800), `V` more friction
(`drag` 0.9→0.3 with `max_thrust` and `k_move` moved together to hold top speed and
movement cost fixed).

| config | births | extinct@ | plant fill |
|---|---:|---:|---:|
| defaults, cold | 0, 0, 0 | 237–242 | 1% |
| `W` warm only | 0, 0, 0 | 384–467 | 99% |
| `WM` warm+slow | 0, 0, 0 | 844–1228 | 99% |
| `WF` warm+dense | 0, 0, 0 | 532–851 | 99% |
| `WV` warm+visc | 0, 0, 0 | 464–662 | 99% |
| `WMFV` warm+all | 12, 24, 16 | 4748, 8475, survived | 99% |
| `MFV` cold+all | 0, 0, 0 | 637–661 | 3% |

No single lever produces a birth. All four together do, and `MFV` — every lever except
the warm larder — produces none, which is what makes the larder necessary rather than
merely helpful.

**The binding constraint is geometric, not behavioural.** Instrumenting the gates found
them innocent: about 47% of agents want to eat and 48% want to breed at any moment, which
is what a fan-in-scaled sigmoid should give. What is scarce is being *near* food. The
fraction of agents within reach of a plant matches the fraction of world area covered by
plant capture discs — `max_plants · π·(radius + feeding.reach)² / size²`, which is 5.0% at
the defaults and measured 4.7–5.3%. A population that senses nothing would score exactly
that, and the population scores exactly that.

So `feeding.reach` is the highest-leverage knob for encounters and `radius` is quadratic
in it, which is also why `feeding.rate` measured inert: rate is extraction speed, reach is
coverage. At `reach` 8, 72–77% of agents are within reach of a plant.

**Decided: the founder keeps everything for Phase 1, and gets simpler at Phase 2.**

The question was whether to give founders fewer sensors so their metabolism is cheaper.
It is worth taking seriously — the sensor term is the largest in the budget after body
size, and `sensing.vision_rays` is already a `SimParams` field, so reducing it is tuning
rather than a code change.

The arithmetic, at default body and topology:

| founder | sensor units | brain units | cost/tick | idle ticks |
|---|---:|---:|---:|---:|
| `vision_rays` 0 (blind) | 4 | 136 | 0.277 | 361 |
| `vision_rays` 3 (default) | 16 | 268 | 0.403 | 248 |
| `vision_rays` 6 | 28 | 400 | 0.530 | 189 |
| default, `body.size` 1.5 | 16 | 268 | 0.268 | 373 |
| default, `k_sensor` per *sensor* | 5 | 268 | 0.293 | 341 |

Blinding the founder entirely buys 361 idle ticks. Halving `body.size` buys 373 — more,
and nothing about it is permanent.

**The eyes cost 12 of the 16 sensor units and currently earn nothing measurable.**
Enrichment — agents within reach of a plant, over the geometric coverage that a
sense-less population would score — sits at 0.8–1.1× for every variant from zero rays to
six. Blind founders find food exactly as often as sighted ones. (The metric is only
meaningful while coverage is well below 1; at `reach` 8 the discs overlap and the ratio
stops meaning anything.)

That measurement is the trap, not the answer. The eyes look free to remove precisely
because no brain has evolved to use them, and evolving one is the phase's success
criterion (spec §8). Removing them makes that permanently unreachable: `mutate` takes
`&mut [Gene]`, so it cannot add or remove a gene at all, and it explicitly no-ops on
`Gene::Sensor`. There is no add-sensor operator in Phase 1 and no remove-sensor operator
either, so the founder's organs are every descendant's organs for the whole phase. This is
the M4 fully-connected-topology decision restated in a different organ: dense is what keeps
the search space reachable, and sparse forbids permanently.

The measurement that settles it, all at `reach` 8 and `body.size` 1.5, three seeds:

| founder | births | extinct@ |
|---|---:|---:|
| 3 eyes, `k_sensor` per channel | 0, 0, 0 | 638–1418 |
| 3 eyes, `k_sensor` per sensor | 1, 2, 3 | 1990–4852 |
| blind, per channel | 4, 2, 5 | 2270–3713 |
| **6 eyes**, per sensor | 1, 2, 1 | 922–3844 |
| 3 eyes, per channel, `body.size` 1.0 | 0, 0, 0 | 959–1531 |

Keeping all three eyes and charging per sensor performs like blinding and charging per
channel. Correcting the cost model even affords *twice* the default eyes and still breeds,
while shrinking the body alone — the largest single term — does not. The sensor budget is
what binds, and how it is charged is the part that is wrong.

`genome::sensor_load` weights by channel count, making the default set 16 units for five
organs. Spec §5.5 says `k_sensor` is "0.01 each, weighted by modality", which per sensor
reads as 5. The channel reading is this codebase's, recorded at M4 as a known gap.

**Nothing about sensors changes in Phase 1.** Not the founder's organs and not how they are
billed: the measured case for either is real, but both are `sim-core` edited on the strength
of a metric, and a phase whose success criterion is still unmet is the worst moment to spend
a capability to buy a joule. The larder was a bug and is fixed; the sensor budget is a
judgment call and is deferred.

**The real answer is that founders should not have been maximal in the first place, and
Phase 1 could not have had it either way.** Density is forced here — with no add-sensor
operator, an organ missing from the founder is unreachable for every descendant, so granting
everything is the only choice that keeps the search space whole. That constraint lifts at
Phase 2, and the default should invert then: the simplest organism that closes the loop,
with complexity earned rather than issued. Recorded in the spec rather than here, at §3.3
next to the operators that unlock it, with a pointer from the Phase 2 roadmap entry and
question 7 of §11 for how simple is still viable — which is a measurement, not a principle.

One thing that pointer surfaced: the Phase 2 roadmap line lists add/remove *neuron and
connection*, not add/remove *sensor*, and a sensor is its own gene kind rather than a
neuron. So the phase that unlocks a simpler founder is not yet named in the roadmap. The
spec now asks Phase 2 to decide it rather than assuming it.

**Done when:** energy conservation holds over 10k ticks within epsilon. Met —
`sim-core/tests/conservation.rs`, which also holds it across four seeds, through a
population starving to nothing, at carrying capacity, across both transfers, and over a
10k-tick run of a population that both eats and breeds.

## M8 — The tick

Build-plan task 7.

- The 11 steps of spec §2.4, in that order. Intents buffered; births and deaths deferred to step 10
  and resolved in agent-index order.
- `Command` enum, serde-serializable, carrying `apply_at_tick`, as the only route into the world.
- `state_hash` folding positions, energies, genomes, RNG state, and tick. Floats hashed via
  `to_bits`, fixed iteration order, never a `HashMap` walk.
- Forward-compatibility checklist audited here.

**Done when:** the golden hash is stable across two in-process runs, and native and
`wasm-pack test --node` agree on the same seed. Met — `sim-core/tests/golden.rs` and
`shells/wasm/tests/cross_target.rs`, which share one scenario file so that "native and
wasm agree" cannot decay into each target agreeing with itself.

**Decisions taken here.**

*`World`'s fields are `pub(crate)`, and that is what the split cost.* Rust needs crate
visibility to write an `impl` across two files, so lifting the eleven steps out of
`world.rs` meant opening its fields to sibling modules. Nothing outside `sim-core` gains
anything — the shells and the integration tests still go through the accessors — and the
alternative was leaving a 1194-line file that AGENTS.md calls a split at 500.

*The steps stay `pub` alongside `step()`.* `steering.rs` drives perception through
movement without the economy on purpose, so that what it measures is the sensorimotor
chain rather than a population's luck with food. A shell should call `step`.

*Three files held their own copy of the tick order.* `conservation.rs`, `no_alloc.rs`,
and now `tick.rs` itself. The first two were rewritten to call `step()`: an acceptance
test that assembles its own tick can go on passing against an order that no longer
ships, which is the failure a golden hash exists to prevent and would have been immune to.

*FNV-1a rather than `DefaultHasher`.* Not for hash quality — `DefaultHasher` is
explicitly not stable across Rust releases, so a golden value built on it moves when the
toolchain moves, and every such move looks exactly like the behaviour change the test
exists to distinguish. Bytes fold little-endian rather than native, because native and
wasm agreeing is the whole point.

*Every variable-length run folds its length first.* Without it, one gene followed by a
plant can produce the same bytes as no genes followed by a differently-placed plant, and
the hash calls two different worlds equal. That is the only failure a golden test cannot
survive, so it was worth the hash update that fixing it cost.

*Two golden runs, each guarded against becoming vacuous.* The shipped defaults at 300
ticks, inside the ~400 an idle founder now survives: run past extinction and the hash
pins an empty world and a decayed field, which is a stable number that has stopped
covering agents. And a configuration that reproduces, because the defaults produce no
births at all and a hash over a population that only starves never reaches
`resolve_births`. Both assert the guard before comparing the value — the birth guard
caught its own test pinning a run where every offspring had already died.

*A command stamped for a tick already past runs rather than being dropped.* Dropping
would make the result depend on how far the sim had got when the message arrived, which
is wall-clock timing leaking into a deterministic system: the same run replayed on a
slower machine would diverge. Commands apply before step 1, so an agent placed this tick
gets a whole one instead of a fragment whose size depends on where the queue was drained.

*The forward-compatibility checklist is executable.* Ticking boxes in a document does not
stop the next person deleting a field that is always `NULL`; every hedge on that list
*is* unused code today, which is exactly why it reads as removable.
`sim-core/tests/forward_compat.rs` covers the five that had nothing pinning them and
names where the other six already are.

*`llvm-tools` is a required toolchain component, not an optional one.* Nothing in the
source calls it, but it ships `libLLVM.dylib` into `lib/rustlib/<host>/lib/`, which is
where `rust-lld` looks. Without it, linking anything for wasm32 dies with `Library not
loaded: @rpath/libLLVM.dylib` and the cross-target test cannot run at all. The toolchain
ships another copy elsewhere that `rust-lld` cannot see, so the failure reads as a broken
install rather than a missing component. Now pinned in `rust-toolchain.toml`, which
`setup.sh` already reads.

## M9 — WASM shell and renderer

Build-plan task 8.

- WASM shell: `step_many`, snapshot pointer/length, `inspect_agent` → JSON, `set_params`,
  `push_command`. Agent pool pre-allocated at max capacity so WASM memory never grows and JS
  typed-array views never detach (spec §7.3).
- Web: sim in a worker; `SnapshotTransport` interface with a SAB implementation and a transferable
  implementation, selected once at runtime from `crossOriginIsolated` (spec §7.7).
- Renderer: WebGL2 instanced quads, not canvas2d. The build plan allows either; canvas2d will not
  hold 5k agents with the sim at 100×, and would be discarded at Phase 5 regardless. Built to be
  presentable — sharing starts at Phase 2 (spec §8).

**Keep the snapshot's sizing separate from the world's.** These two look like one decision and
are not. The snapshot is 61 bytes per agent — about 0.5% of per-agent state, 0.31 MB at the
default 5k and 3.1 MB even at Phase 7's 50k. World state is ~11.3 KB per agent, almost all of it
genome. Pre-allocating the snapshot at capacity is therefore free and should just be done.

The reason that matters: §7.3's detach hazard is the argument for pre-allocating *everything*,
but it only bites what JS actually views, which is the snapshot alone — inspector data is
pulled for one agent on demand through the command queue (§2.2b), and nothing else crosses the
boundary. Wiring the transport so it reads the world arenas directly would couple the cheap
buffer to the expensive one and make the pool impossible to grow later without breaking every
view. Spec §7.5 has the measurements.

**Done when:** 5k agents render at 60fps with the sim at 1× and at 100×. **Half met, and
the half that is not is `sim-core`, not the renderer.** Measured in Chrome on an M-series
Mac, release wasm (`opt-level = 3`, `lto = "fat"`), 1000×1000 world, default params:

| live agents | sim ticks/s at 1× | render fps |
|---:|---:|---:|
| 585 | 60.8 | 120 |
| 1406 | 59.2 | 120 |
| 5000 | **18.0** | 120 |

The renderer holds 120fps at every population and at every speed, drawing all 5000 slots
each frame — it is nowhere near its limit, and the criterion's rendering half has real
headroom. What cannot hold 1× is the tick: somewhere between 1400 and 5000 agents it
falls off 60/s, and at 5000 it manages 18. Spec §2.2c predicts where that goes —
perception is 60–80% of tick cost and the only phase that touches the spatial hash — but
predicting is not measuring, and nothing here has profiled it.

Left as a measurement rather than an optimisation. Phase 7 is the performance milestone
and owns SIMD and `rayon`; reaching for either now would be optimising `sim-core` against
a number, which is the loop CLAUDE.md keeps separate from tuning for good reason. The
population also cannot *stay* at 5000 under the shipped economy — it starves in ~400
ticks — so 5k is a stress figure rather than a steady state anything currently reaches.

**Decisions taken here.**

*Both transports copy once, in the worker.* Spec §7.3's zero-copy read is a view straight
onto WASM linear memory, which the main thread can only take if that memory is *shared* —
and that needs a threads-enabled build (`--shared-memory`, atomics) that arrives with
`wasm-bindgen-rayon` at Phase 7. Until then the worker owns its memory alone and one copy
per frame is the floor rather than a shortcut. What the shared transport still buys is
not the copy: the renderer never blocks on a message, and the writer drops a superseded
frame instead of stalling if every slot is briefly busy. Both paths were exercised — the
fallback by forcing it, since a fallback nobody has run is a guess.

The shared path uses three state-tracked frames rather than an unleased double buffer.
The renderer holds one frame until the next animation frame; the worker cycles through
the other two and publishes frame-local metadata with the payload. That lease is what
prevents a second publish from overwriting arrays while WebGL is still uploading them.

*The snapshot is slot-indexed, so the renderer draws capacity, not population.* A dead
slot has `alive = 0` and the vertex shader multiplies the radius by it, collapsing the
quad to no area. That keeps the draw one call over a fixed instance count rather than a
per-frame compaction pass on the CPU, and it is why the render cost does not fall as the
population does.

*A throw inside the worker's timer loop used to vanish.* The loop reschedules at its end,
so one exception stopped the clock while the renderer went on drawing the last frame at
full rate — a frozen world that looks exactly like a paused one. It now stops
deliberately and reports, and `client.js` listens for `onerror` and `onmessageerror` as
well, because a worker-level throw reaches neither the page console nor any handler by
default.

*Catch-up is time-sliced rather than unbounded.* At 5k agents the sim cannot meet 1×, so
elapsed-time debt grows while a batch is running. Turning all of that debt into one
`step_many` call made pause and commands wait behind tens of seconds of work at high
speeds. The scheduler now measures tick cost, caps both debt and batch size, and yields
between batches. Requested speed is best-effort when throughput is lower; worker
responsiveness is not.

*Plants are in the snapshot.* Spec §2.2b includes their position and current stock so
Phase 1's food-seeking criterion is observable. An emptied site stays visible at low
brightness because it persists and regrows (spec §5.1).

*Plant positions travel every frame even though sites never move.* Sending them once
would be cheaper and is what "fixed sites" invites, but relocating a depleted site is
already named in `plants` as a change an M12 run might call for, and a renderer holding
cached positions would draw food where none is.

*The renderer is built from the world's own hints, not from constants in the client.*
`world.size` and the two capacities were duplicated in `App.svelte`, which draws a
correct picture of the wrong world the moment either moves — and `set_params` exists
precisely so params can move. `Sim::render_hints` reports them, and the renderer is
rebuilt on every reseed because a reseed can carry different ones.

*The camera wraps but does not repeat.* The world is a torus, so both vertex shaders place
each instance at its nearest image to the camera centre — the same minimum-image rule
`spatial` measures every distance with. Panning across the seam is then continuous rather
than hitting a wall the picture invented, which is the case that matters, because it is
the one you meet while actually watching something.

Tiling the draw to fill a wide viewport with the wrapped copies a torus strictly has was
tried and removed. It is more faithful to the geometry and worse for the only question
this view exists to answer: the same agent appears two or three times, which makes a
population look larger than it is and a cluster look like several. Minimum image alone
puts every agent in one world-sized band around the camera, so each is drawn exactly once,
and the zoom floor stops where the whole world is on screen. The cost is empty margins on
the long axis of a window whose shape the world does not match — visible, and honest about
what is there.

*The `alive` attribute is not normalized.* It cost an hour: a `UNSIGNED_BYTE` attribute
declared normalized divides by 255, and the flag is 1 rather than 255, so every agent's
radius became four thousandths of its size. The world renders as empty — no error
anywhere, and nothing to grep for.

## M10 — Instrumentation

Build-plan task 9.

- Population and mean-energy time series, canvas or uPlot — never an SVG/DOM chart library.
- Agent inspector: click an agent, see genome and live neuron activations, pulled on demand for the
  one selected agent (spec §2.2b).
- `ts-rs` derives on the genome types, generating the TypeScript the inspector reads. Deferred
  here from M4: the point of generating them is to stop the inspector drifting from the genome,
  and until the inspector exists there is nothing to drift.
- Seed URL encoding. Speed control including pause.

**Done when:** you can watch a run, pause, click an agent, and read its brain.

**Implemented.** The browser samples population and mean live-agent energy four times per
wall-clock second into a bounded canvas chart. Mean energy is computed only when the worker
samples it; it is not added to the per-frame snapshot and costs nothing in headless runs.

Clicking uses the renderer's toroidal nearest-image rule and minimum displayed radius, then
requests one agent's inspection JSON from the worker. The selected slot is highlighted and
the panel refreshes its live neuron activations at human speed while keeping the full genome
collapsed until requested. A per-slot incarnation in the snapshot prevents selection from
silently following a recycled slot to a different agent. No genome or brain state is streamed
for unselected agents.

`ts-rs` derives on the Rust genome types generate the committed contract in
`web/src/generated/`. The typed inspector model imports `Gene` from that output, and CI
regenerates the bindings and rejects drift.

The URL fragment carries the exact decimal `u64` seed, founder count, and canonical params
JSON. It is updated only for the active world, so editing an input does not make the copied
link claim to represent a run that has not been reseeded yet. A target tick remains optional
and unset until checkpoint/load support can make opening one practical.

## M11 — Headless telemetry

Build-plan task 10.

- `--metrics run.jsonl` on the native shell, one sample line per interval (spec §7.9).
- `diagnose` subcommand mapping metric signatures to the spec §10 failure modes.
- Random-brain control as a **separate world with the same seed and params**, not a marked lineage
  inside the live world. A control lineage sharing the world competes for the same energy and
  perturbs the thing it is measuring.

**Done when:** a 500k-tick headless run produces a metrics file, and `diagnose` correctly
identifies a deliberately induced extinction and a deliberately induced monoculture.

**Implemented.** A run writes a versioned header with the binary version, full params,
seed, and control protocol, followed by interval samples. Every sample contains evolving
and random-control vectors side by side: population, exact genome variants, descendant
count, agent and plant energy, speed, age, brain/genome size, mean absolute connection
weight, and cumulative energy-ledger values. The final sample includes both state hashes.
Metrics are sampled by the native shell after completed ticks; no counters or callbacks
were added to the hot loop.

The control is a second `World` built from the same seed and `SimParams`. Founders are
therefore identical. At each control birth, weights, biases, time constants, and
oscillator periods are freshly randomized while topology, sensors, body traits, spatial
viscosity, and the energy economy remain inherited. This breaks neural heredity without
letting the control compete with the population it measures.

`diagnose` reports early or late extinction, exact-genome monoculture, energy drift,
brain bloat, stable idling, and aggregate similarity to the random control. Phase 1 has
no species clustering, so a species-based monoculture diagnosis is explicitly
unavailable until Phase 2; exact genome variants are the narrower signal available now.
Predation and signaling are likewise reported unavailable until those systems exist.
Mean absolute connection weight is a drift descriptor, not evidence of adaptation:
neutral mutation alone can move it.

The mechanical criterion was exercised with a 500,000-tick, same-seed paired run using
a deliberately cheap 32-slot, 8×8-field fixture. Both cohorts reached and retained the
32-agent ceiling, each ended with 24 living descendants, and their final state hashes
diverged (`ddcbd7e206890fbb` evolving, `7a4526c29488e1cb` control). It produced a header
plus six samples in about 65 seconds in release mode:

```bash
cargo run --release -p native -- \
  --seed 42 --ticks 500000 --founders 8 --sample-every 100000 \
  --params shells/native/tests/fixtures/sustaining.json --metrics run.jsonl
```

Separate committed CLI fixtures induce and identify early extinction and stable
exact-genome monoculture. The monoculture fixture is a real no-mutation simulation:
seed 8 begins with two distinct founder genomes and ends at tick 4,000 with 22 agents,
21 living descendants, and one surviving genome variant.

The sustaining fixture sets metabolic costs to zero and is intentionally not evidence
of ecological viability; it exists to exercise 500k ticks, births, randomized control
heredity, paired sampling, and final hashes. Shipped defaults still go extinct before
selection can accumulate, which remains the M12 tuning question. Brain-inheritance mode
is experiment configuration like `SimParams`, not mutable world state, so it is not
folded into `state_hash`; the existing golden constants and default behavior are unchanged.

The header embeds the package version and Git revision (`-dirty` when the runtime Rust
sources, manifests, lockfile, or pinned toolchain do not match that revision), and the
parser rejects incomplete streams or missing final hashes. Random-control comparisons
require at least three samples in which both cohorts still contain living descendants;
identical founders or two extinct worlds are reported as unavailable rather than as
evidence. Eligible comparisons always report both cohorts' tail means and relative gaps,
whether or not those gaps trigger the indistinguishable-control warning. This remains a
single-seed comparison with no variance estimate — M12 acceptance owns the required
multi-seed judgment.

## M12 — Acceptance

**Mechanical:** golden hash, energy conservation, no-alloc, cross-target agreement, brute-force
neighbour parity — all passing.

**Judgment:** three or more seeds run side by side against the random-brain control, watched by a
human. Not self-certifiable (AGENTS.md, spec §7.8 tier 3).

If food-seeking does not emerge, check spec §10 before changing code. It is almost always
metabolic cost too low, energy input too high, or mutation rate past error catastrophe — all
tuning, not bugs.
