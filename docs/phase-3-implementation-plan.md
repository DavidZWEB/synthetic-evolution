# Phase 3 - Implementation Plan

Phase 3 adds **predation and evolvable bodies** from
[`synthetic-evolution-spec.md`](synthetic-evolution-spec.md) section 8:

- a bite that founders carry dormant and lineages must evolve into use, with a mouthful
  of energy per hit, health, kills, corpses, and decomposition;
- evolvable body size, muscle, and mouth;
- new sensors that arrive wired;
- attacks drawn in the browser.

Design decisions live in the spec: §2.2b combat in the snapshot, §3.3 wired sensors,
§3.5 body traits, §4.1 knockout switches, §4.2 the bite, §5.1 corpses, §5.2 costs, and
§5.5 constants. This plan tracks status and order. Phase 2's record is in
[`phase-2-implementation-plan.md`](phase-2-implementation-plan.md).

**Status: M1 and M2 done; M3's bite landed, with its diet and kill telemetry next.** The success criterion is **a carnivorous lineage becomes
established without going extinct or eating everything**. It is measured as a species
whose members take most of their energy from other agents (bites and corpses),
persisting for at least half a run on several seeds while plant-eaters persist beside
it, against both controls (spec §7.8). Like every phase, it is judged by a human watching
the sim, with multi-seed evidence alongside; mechanical tests cannot certify it.

| Milestone | State |
|---|---|
| M0 Design | Recorded in the spec; revised 2026-10-04 from the research below |
| M1 Corpses | Done: pool, decomposition, eating, sight, hashing, checkpoints, snapshot, and drawing |
| M2 Evolvable bodies | Done: muscle and mouth genes, mass, gape, upkeep, the mutation operator at rate 0, and trait telemetry |
| M3 The bite | In progress: the dormant bite, the two-pass swing, health, and kills landed; diet and kill telemetry next |
| M4 Protecting innovation | Not started |
| M5 Seeing predation | Not started |
| M6 Calibration | Not started |
| M7 Acceptance | Not started |

## Why the design looks like this

The design was revised after a review of how predation arose in real life and how NEAT
handles new structure. The decisions it rests on:

- **Grazers come first.** At tick 0 the world has one population and one kind of food,
  the situation of the Ediacaran mat grazers such as *Kimberella* before predators
  appeared. Model food webs grow out of a single ancestor in the same way
  ([Loeuille & Loreau 2005](https://pmc.ncbi.nlm.nih.gov/articles/PMC556288)).
- **Predators reuse parts they already have.** Hawaiian *Eupithecia* caterpillars descend
  from flower and seed eaters and became ambush predators
  ([ESA abstract](https://esa.confex.com/esa/2001/techprogram/paper_2034.htm)). Diet
  switches are rare, and in mammals they mostly pass through omnivory
  ([Price et al. 2012](https://pmc.ncbi.nlm.nih.gov/articles/PMC3345017)). Hence the
  dormant bite: founders carry it, and lineages discover its use.
- **The first payoff has to be immediate.** Nearly all predators also scavenge
  ([DeVault et al. 2003](https://digitalcommons.unl.edu/icwdm_usdanwrc/269)), and the
  oldest predators known fed from prey without killing it
  ([Porter 2016](https://news.ucsb.edu/2016/016842/tiny-vampires)). Hence the mouthful:
  a bite repays itself before any kill.
- **Predators need to find prey.** The light-switch hypothesis ties the rise of
  predation to eyes, and Phase 2 found evolved lineages shed perception. Hence founder
  vision is a calibration axis, and the senses knockout is repeated with predation on.
- **Size, speed, and gape decide encounters.** Upkeep grows with mass (Kleiber). Top
  speed peaks at intermediate size
  ([Hirt et al. 2017](https://www.idiv.de/?p=8904)). Gape limitation gives prey a size
  refuge ([Urban 2008](https://ecoevolutionlab-eeb.media.uconn.edu/wp-content/uploads/sites/3715/2023/09/Urban2008Oikos.pdf)).
- **Grazing is biting.** An herbivore's intake is bite size times bite rate, and bite
  area grows with the square of mouth width
  ([foraging efficiency in grazing ruminants](https://researchonline.jcu.edu.au/42625/)).
  Hence one mouth trait scales grazing, scavenging, and the bite. Grazing and attacking
  stay separate actions, so a grazer does not bite its own offspring by accident.
- **New structure needs a chance to work.** NEAT's authors found that new structure
  usually lowers fitness at first, and that unconnected structure may never join the
  network ([Stanley & Miikkulainen 2002](https://www.cse.unr.edu/~sushil/class/gas/papers/NEAT.pdf)).
  NEAT's fix, fitness sharing, needs a fitness score this simulation does not have.
  Hence new sensors arrive wired, and their survival is measured.

## Working rules

Phase 2's rules carry over (`AGENTS.md`).

- **Each mechanism lands reproducing Phase 2 dynamics.** Founders sit at default
  traits, `founder.bite` is off, body-trait mutation is at rate 0, and
  `wired_weight_scale` is 0. Calibration (M6) then switches mechanisms on, with
  multi-seed evidence, as its own golden change.
- **Golden references move only where a change requires it:** a widening of hash
  coverage, new genes, or a storage change. Independent causes go in separate commits,
  each with its own explanation.
- **Re-prove the invariants with each mechanism active, not only with it off:** energy
  conservation, no hot-loop allocation, determinism, and native/WASM agreement.

## M1 - Corpses (done)

A fixed pool of corpses (`corpses.max_corpses`), each a position and an energy pair, all
discs of radius `corpses.radius` for both eating and sight.

- Every death leaves `corpse_energy_fraction` of the agent's energy as a corpse and
  dissipates the rest. A death that finds the pool full dissipates the whole share, and
  is counted.
- Starved agents die empty, so nothing visible changes until the bite exists.
- Corpses decay into dissipation and are removed below `corpse_min_energy`.
- `ingest` takes from the nearest food, plant or corpse. Vision sees corpses.
- Corpses are hashed, checkpointed, sent in the snapshot, and drawn.

## M2 - Evolvable bodies (done)

Spec §3.5 and §5.2.

**Mechanism**

- Muscle and mouth body-gene kinds. Founders carry size `body.size`, muscle 1, and mouth
  1, with no random draws.
- Mass: acceleration is force divided by `s²`, where force is the thrust drive times
  `max_thrust` times muscle. The Phase 5 mass note in `movement.rs` moves here.
- Grazing and scavenging intake: `feeding.rate · g²`.
- Upkeep: the `k_muscle` and `k_mouth` terms, which are zero at default traits.
- A body-trait mutation operator: rate, log-normal step, and ranges, with colour
  drifting by an additive step. A zero rate consumes no draws.
- Telemetry: trait distributions per cohort in native metrics and `summarize`.

**Tests**

- At default traits, cost, intake, and acceleration equal Phase 2's bit for bit.
- At non-default traits, each rule matches a hand-computed value: mass slows a large
  body, muscle scales the thrust force, intake grows with gape², and upkeep adds
  exactly `k_muscle · (muscle² − 1)` and `k_mouth · (mouth² − 1)`.
- Mutated traits stay inside their ranges.
- Conservation and no allocation hold with traits mutating.
- Native/WASM agreement on a trait-mutating case.

**Golden:** founders gain two inert genes, and the state hash folds muscle and mouth.
Each moves hashes without changing dynamics, in its own commit.

**Landed** with body-trait mutation at rate 0, so Phase 2's dynamics are unchanged.
`body.size` is fixed for a world's life, since every body is measured against it
(spec §3.5). History and metrics files written earlier read one-point trait ranges at
the founders' values, which is exactly a run whose bodies never evolved.

## M3 - The bite

Spec §4.2 and §2.4.

**Mechanism**

- `Action::Bite`, with intents carrying drive, azimuth, and reach, and a per-agent
  cooldown.
- `CombatParams`: gate, reach, arc, cost, damage, cooldown, regeneration, dormant bias,
  mouthful, and assimilation.
- `founder.bite`, which is part of the founding topology and fixed for a world's life.
- The founder's bite neuron is excluded from founder wiring and starts at
  `combat.dormant_bias`.
- The per-genome effector limit rises from 4 to 5 in the storage defaults, in its own
  commit.
- Step 7 swings before ingest, in two passes.
  - The first pass fixes every eligible swing, its target, and its paid cost from the
    state at the start of step 7. The second applies hits in agent-index order.
  - The agent grid is rebuilt for the swing pass only on ticks when someone swings,
    since agents moved in step 5.
  - Damage is `attack_damage · g / s_victim`.
  - Each hit takes a mouthful of `mouthful · g²`, of which the biter assimilates
    `assimilation` and the rest is dissipated.
- Step 9 regenerates health above 0 and counts down cooldowns. Agents at health 0 join
  step 10's deaths and leave corpses.
- Diet and kill telemetry:
  - per agent, lifetime energy eaten from plants and from other agents (bites and
    corpses), so diet can be measured per species;
  - per cohort, swings, hits, kills, and corpse stock;
  - `summarize` and `diagnose` report diet fractions and the spec §7.9 predation
    signals.

**Tests**

- A hit moves exactly the mouthful, and the dissipated share is exact (spec §7.8 tier 1).
- Index order cannot decide who swings or who dies.
- Cooldown and cost.
- Arc, reach, and size-ratio geometry.
- Regeneration stops at 0.
- Dormant founders never swing until their bite neuron is wired.
- Conservation and no allocation with combat on.
- Checkpoint continuation through kills.
- Native/WASM agreement.

**Landed (mechanism)** with `founder.bite` off, so Phase 2's dynamics are unchanged.
`storage.max_effectors` is 5, so a world with the bite needs no storage edit, while
`effectors_per_slot` stays 4: 5,000 slots hold 4,000 biting founders. The gate is
non-negative, since an agent without a bite drives it at 0. A cooldown counts whole
ticks, never shorter than `cooldown_seconds`, and a `world.dt` retune keeps each running
one's remaining time. Two bites on one agent sum their drives,
and the stronger aims. An imported bite must aim on the plane and reach no further than
half the world. The random control redraws the bite's bias with every other neural
scalar, so once calibration turns the bite on, its children bite at random from birth;
M6 should weigh that when comparing against it. History and metrics files written
before the bite read with founders that cannot bite and every combat number zero,
since today's defaults are checked against each run's own timestep and body ranges
and could refuse a run that never swung.

## M4 - Protecting innovation

Spec §3.3 and §4.1.

- **Wired sensor addition** under `mutation.organs.wired_weight_scale`. At 0 it
  reproduces Phase 2; M6 decides the shipped value.
- **Innovation-survival telemetry.** For each sensor innovation, when it first and last
  appeared in the population, compared between the evolving world and both controls.
  `summarize` reports how long sensor innovations persist.
- **Knockout switches** `sensing.vision_gain` and `sensing.chemo_gain`, so a retune can
  silence existing eyes or noses. The Phase 2 tooling could only stop new scent.
- How often sensors are removed relative to added is tuned in M6, not changed here.

## M5 - Seeing predation

Spec §2.2b.

**Snapshot**

The combat fields: health, ticks since the last swing, ticks since the last hit, and
where the latest swing landed (NaN on a miss, so the swing's age always matches it).
That is 72 bytes per agent, pinned on both sides as the current 61 is.

**Renderer**

A newly seen event becomes a wall-clock animation, like the reseed glow, so it stays
visible at 100× speed.

- **Swing:** a quick forward lunge plus a translucent wedge showing the bite arc.
- **Hit:** the victim flashes red and shakes, with red specks at the point of contact,
  and a line runs briefly from biter to victim.
- **Wound:** a red rim that fades as health regenerates.
- **Kill:** the body shrinks into its corpse with a ring burst.

**Inspector and colouring**

- The inspector shows health, the three traits, the diet split, and kills.
- A diet colour mode runs from green (plants) to red (other agents).

**Tests:** byte budgets on both sides, both transports, and the animation state
machine, as the reseed glow is tested.

## M6 - Calibration

The spec §7.9 sweep, with `founder.bite` and body-trait mutation on. Its axes:

- attack cost × mouthful × plant input;
- founder vision (no eye vs one ray);
- sensor wiring (unwired vs wired).

Several seeds per cell and both controls. Each cell reports:

- time to the first lineage that keeps biting;
- kills, and diet fractions per species;
- trait distributions;
- how long predators and grazers coexist;
- wired sensors, and sensor-innovation lifetimes.

A **pre-wired bite diagnostic** gives founders a connected bite, to tell "biting cannot
pay here" apart from "evolution cannot find it". The output is an unranked shortlist.
Ship defaults with their reasons on the fields; that is a golden change.

**Named fallbacks** are not built unless no cell sustains coexistence:

- a digestion trade-off gene for plants against meat;
- carcasses from every death, meaning a body energy store separate from the tank.

## M7 - Acceptance

Run every check, and record seeds, params, revision, controls, durations, variance, and
what was observed.

Repeat the scent and vision knockouts with predation on, to answer the question Phase 2
left open: does perception pay once there is something to hunt or flee?

A human decides whether a carnivorous lineage established.

## Scope

**In scope:**

- the dormant bite, mouthful, health, kills, corpses, and decomposition;
- scalar body traits (size with mass, muscle, mouth, colour drift);
- wired sensor addition, innovation-survival telemetry, and knockout switches;
- diet and kill telemetry;
- attack animations and the inspector views they need.

**Out of scope:**

- multi-part bodies (Phase 5);
- adding or removing effectors by mutation;
- carrion scent, and corpse nutrients feeding plants;
- armour or body-plan defences (Phase 5);
- signalling (Phase 4);
- crossover, and NEAT's explicit fitness sharing, which needs a fitness score;
- the M6 fallbacks unless calibration calls for them.
