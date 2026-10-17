# Phase 3 M6–M7 research report: a carnivorous lineage, established naturally

Nothing here is checked in. The code is uncommitted in the `research/phase-3-carnivory`
worktree; every mechanism defaults to off, and with defaults the sim hashes identically to
its base on `main` (`03ed1381`). The chronological lab notebook (25 chunks, ≈ 260 runs) is `LOG.md` beside this file.

## 1. Outcome

**In every seed, a carnivorous lineage evolves from a dormant bite and specializes. It holds
its niche beside thriving grazers to the end of the 300k-tick run. Neither control does
this.** Founders are grazers whose bite is dormant: its connections carry zero weight.

The plan's strict proxy, one species *id* above half meat for half the run, reads 0. The
reason is that predators live 3–12× shorter lives than grazers, so their genomes leave a
species' fixed representative that much sooner in ticks (§1.1).

Configuration `L10-m100` (§4), 1,000 founders, 300k ticks, scalar control paired:

| seed | carnivores from | longest unbroken guild | median guild | guild meat | lineage, strict descent | longest single species id | mean age, carnivores / grazers | final pop (scalar control) |
|---|---|---|---|---|---|---|---|---|
| 1 | 115k | 130k | 45 | 0.60 | 125k | 80k | 2,241 / 8,752 | 1152 (4) |
| 2 | 60k | **200k** | 134 | 0.64 | **200k** | 130k | 1,282 / 5,418 | 1005 (4) |
| 3 | 20k | **265k** | 80 | 0.68 | **235k** | 70k | 638 / 6,764 | 817 (9) |
| 4 | 35k | **160k** | 83 | 0.64 | **150k** | 70k | 763 / 4,324 | 971 (5) |
| 5 | 60k | **225k** | 95 | 0.64 | 130k | 45k | 884 / 6,812 | 821 (6) |
| 6 | 55k | 135k | 57 | 0.66 | 40k | 40k | 980 / 9,663 | 1093 (7) |
| 7 | 95k | **205k** | 94 | 0.69 | **150k** | 100k | 996 / 5,332 | 926 (13) |

Terms used in the table:

- **Carnivorous species**: one whose living members took more than half their lifetime
  intake from other agents (bites and carrion), with plant-eaters alive beside it (spec §7.9).
- **Guild**: every carnivorous species together, counted when it has at least 5 members.
  "Unbroken" means the guild was present at every 5k sample. Guild meat is the guild's
  pooled meat share.
- **Lineage, strict descent** (`lineage_strict.py`): a chain of carnivorous species, each
  descended through the species-origin records from one that was carnivorous at the
  previous sample. The chain may not pass through any ancestor that was ever sampled at
  50% meat or below, so carnivory that re-evolves in a grazer clade starts a new chain.
- **Mean age** is per-species, sampled at 305–320k with telemetry added for this purpose.

What the runs show:

- **Never extinct, never eating everything.**
  - The guild is alive at the end of all 7 runs, with 23–166 members.
  - It is unbroken for at least half the run in 5 of 7 seeds, and its lineage by strict
    descent spans at least half the run in 4 of 7.
  - Prey are never eaten out: once a guild exists, populations stay at 745–1,158. The
    exception is seed 3, whose guild appears at 20k while the population is still
    recovering from the founders' early crash (351).
  - Carnivores hold 3–12% of living biomass (second-half means per seed).
- **Mostly grazers and hunters, with a minority of mixed diets** (see §11).
  - Individual diets are bimodal: 1–4% of agents eat at least 90% meat and 88–97% eat under
    10%. Grazers' meat share is 0–3%. Between them, 3–9% have mixed lifetime diets, and
    half to two-thirds of predator-species members also took more than 10% from plants.
  - Predators' guts are smaller (second-half medians 0.71–0.90, falling as low as 0.46,
    against 0.93–0.96 for grazers).
  - Predators' bodies are larger (median ratio 1.12–1.46).
  - 66–80% of hits land on plant-eaters.

Controls, same seeds and configuration:

| cohort | longest unbroken guild (≥ 5) | median guild | longest single carnivorous species id |
|---|---|---|---|
| evolving (seeds 1–4) | 130–265k | 45–134 | 70–130k |
| structural null (seeds 1–4) | 10–110k | 0–6 | 10–30k |
| scalar (seeds 1–7) | never | 0 | none; the population collapses to 4–13 agents |

### 1.1 Why the single-species proxy reads 0

- **What the proxy requires.** `persistent_carnivores` needs one species id to be above
  50% meat at every sample for 150k ticks.
- **Grazer ids do last that long.** The founder species spans the whole run in 6 of 7
  seeds. Predator ids last at most 130k (160k once, in a smell-knockout run).
- **The cause is the generation-time effect.**
  - Spec §3.4 gives each species an immutable representative and a distance threshold of
    0.5. A lineage keeps its id until its genomes drift 0.5 away from that representative.
  - Predators' mean age is 640–2,240 ticks, against 4,300–9,700 for grazers. Taking mean
    age as a proxy for generation time, a predator lineage passes 3–12 generations for
    each grazer generation, so it crosses that distance 3–12× sooner in ticks.
  - Measured in generations, a 100k-tick predator id (≈ 100 generations) outlasts a
    300k-tick grazer id (≈ 50).
  - This is the generation-time effect on rates of genetic change: short-lived lineages
    change faster per unit of time (Wu & Li 1985).
- **Threshold flicker.** A species near 50% meat sometimes dips below the line at a single
  sample, which restarts the every-sample streak.

## 2. What stood in the way, in the order it was found

1. **Spec-value bite economics cannot pay.** A swing at cost 8 is ≈ 100 ticks of a typical
   agent's whole budget (measured upkeep ≈ 0.08/tick); a hit keeps ≈ 10. Every spec-value
   cell selected biting away.
2. **An unwired dormant bite is found late or never** (≈ 95k when found).
3. **With rich meat, everyone becomes an omnivore.** Long runs also produced a degenerate
   society of tiny, big-mouthed agents biting each other.
4. **A meat penalty on herbivore guts separates the guilds but stops biting from
   starting**: one parameter was doing two jobs (chunks 3, 6, 7, 8).
5. **Predators settle as patch omnivores at ≈ 30–45% meat**, whatever else is changed
   (gut, plasticity, jaw, forward eyes; chunks 9–19).
   - Intraguild-predation theory explains this (Holt & Polis 1997).
   - An omnivore's numbers are set by both foods. The guild therefore grows on plants until
     prey per predator are scarce, and each predator kills about once in its life.
6. **The fix: a long Holling handling time** (10 s without grazing after each hit).
   - Biting and grazing become nearly exclusive across a life.
   - A grazer gives up ≈ 90 energy of grazing per bite, so it stops biting.
   - A lineage that hits often loses nothing, so it commits and sheds gut. Its numbers are
     then held by prey: a saturating (type II) functional response.

## 3. Mechanisms (each off by default; tests added; defaults hash-identical to `main`)

| mechanism | params | grounding | role in the result |
|---|---|---|---|
| Cryptic dormant bite | `founder.bite_cryptic` | evolutionary capacitance (Rutherford & Lindquist 1998); cryptic variation is enriched for adaptations (Masel 2006). NEAT protects new structure with fitness sharing, which needs a fitness score this sim lacks; this protects it by hiding it. | sustained biting (≥ 50 hits per 5k ticks) from 10–30k in every seed |
| Holling handling time | `combat.handling_seconds` | Holling 1959; Stephens & Krebs 1986 | **the decisive change** |
| Gut body trait | `founder.gut`, `body.gut`, `metabolism.k_gut`, `digestion.{plant_floor, plant_curve, meat_penalty}`, `mutation.gut_sigma` | expensive tissue (Aiello & Wheeler 1995); diminishing returns in gut reactors (Penry & Jumars 1987); guts follow diet in ≈ 30 generations (Herrel et al. 2008) | lets predators shed gut upkeep they no longer use |
| Diet-induced gut plasticity | `digestion.{plasticity, acclimation, diet_memory}` | Piersma & Lindström 1997; spadefoot toad carnivore morph (Levis & Pfennig 2016) | tested; not used in the final configuration |
| Jaw body trait (bite vs graze) | `founder.jaw`, `body.jaw`, `feeding.jaw_graze_{cost,exponent}`, `combat.jaw_bite_{gain,exponent}`, `mutation.jaw_sigma` | trade-offs between dentition and mouth form (Serobyan et al. 2014; Westneat 1994) | tested; not used in the final configuration |
| Eye-direction drift | `mutation.organs.eye_drift_{rate,sigma}` | eye placement in predators and prey | used at a slow rate |
| Eye placement at birth | `sensing.vision_azimuth_spread` | cephalization | tested; not used |

The native shell's telemetry gained:

- per-species gut, jaw, size, mouth, muscle, kills, eye offset, mean age, and genome length;
- gut, jaw, and eye distributions;
- an individual diet histogram;
- counts of hits on the biter's own species and on plant-eaters;
- `resume` with metrics, species history, and knockout retunes.

## 4. The configuration (`L10-m100`)

```json
{"founder": {"bite": true, "bite_cryptic": true, "gut": true},
 "combat": {"attack_cost": 2, "mouthful": 100, "attack_damage": 0.5, "handling_seconds": 10.0},
 "metabolism": {"k_gut": 0.04},
 "digestion": {"plant_floor": 0.0, "plant_curve": 0.5, "meat_penalty": 0.0},
 "sensing": {"vision_rays": 1},
 "mutation": {"body_trait_rate": 0.1, "body_trait_sigma": 0.05, "gut_sigma": 0.15,
              "organs": {"wired_weight_scale": 0.25, "eye_drift_rate": 0.1, "eye_drift_sigma": 0.1}}}
```

The runs use 1,000 founders. These departures from spec §5.5 would need recording on the
fields:

- attack cost 8 → 2;
- mouthful 20 → 100;
- damage 0.25 → 0.5;
- handling 10 s (new);
- founders' bite on and cryptic;
- gut on;
- body-trait mutation 0.1 (the spec value);
- wired sensors 0.25.

## 5. Knockouts and controls

**Knockouts** (seeds 1–3, sense switched off at 150k, run to 300k):

| | lineage spans, strict descent | longest unbroken guild | guild meat, 150–300k |
|---|---|---|---|
| intact | 125k; 200k; 235k | 130k / 200k / 265k | 0.60 / 0.66 / 0.64 |
| vision off | 85k (alive at the end); 140k; 215k | 110k / 200k / 210k | 0.62 / 0.60 / 0.63 |
| smell off | 50k (alive at the end); 185k (one species id for 160k); 165k | 80k / 200k / 265k | 0.69 / 0.62 / 0.63 |

Knocking out either sense does not end carnivory or change the guild's diet. Lineages by
strict descent run shorter after 5 of the 6 knockouts (for example 50k against 125k), so
the senses may help predators persist, but three seeds cannot separate that from chance.
The predators hunt where prey gather, at the plant patches. On Phase 2's open question
(does perception pay once there is something to hunt?), this is weak evidence for a
small yes.

**Structural-null control** (seeds 1–4; topology from a living donor, with the parent's
weights on shared genes):

- It stays viable (750–1110 agents) and it does bite.
- Its carnivorous guild is small (median 0–6) and intermittent: unbroken for 10–110k,
  present at 30–62% of samples.
- Its carnivorous species ids last 10–30k.
- At plant input 36k the null gives 35k and 125k unbroken (median guild 6 and 42), against
  230–270k for the evolving cohort.
- Heritable brains matter. When structure is decoupled from weights, the null's guilds are
  at most a seventh the size and its carnivorous species ids are 3–7× shorter-lived. Its
  guild stays unbroken about as long as the evolving cohort's in only one seed (seed 1:
  110k against 130k).

**Scalar control**: collapses to 4–13 agents and is never carnivorous.

## 6. Spec changes this would need (none made)

| spec section | change |
|---|---|
| §4.2 | cryptic wiring of the dormant bite; handling time |
| §3.5 | gut and jaw body traits; plasticity |
| §4.1 | eye drift and eye placement |
| §5.2 | gut upkeep |
| §5.1 | digestion waste as a dissipation flow |
| §5.5 | the new constants and the departures in §4 |
| §7.9 / §8 | the Phase 3 measure (see §7, item 1) |
| checkpoint format | new per-agent fields (format 12 → 13) |

## 7. Judgment calls for you

1. **Which measure establishes "a carnivorous lineage".** There are three candidates:
   - the single species id: fails, reading 0;
   - lineage by strict descent: at least half the run in 4 of 7 seeds;
   - guild presence: unbroken for at least half the run in 5 of 7, and alive at the end in
     7 of 7.

   A fourth option is a species-id measure scaled by generation time. The spec's sentence is
   "a carnivorous lineage becomes established"; the plan measures it as one species id.
   Adopting any other measure is a §7.9/§8 wording change.
2. **Handling time of 10 s** is long against a 0.5 s cooldown: a predator eats a meal
   slowly. It is the parameter that makes the result, so it deserves a hard look.
3. **Mouthful 100 and attack cost 2** make a bite a large, cheap prize; the spec's 20 and 8
   cannot pay.
4. **Plant input.** A 1.5× input gives bigger guilds but no longer-lived species ids (§8),
   so the default input stands.
5. **Short predator lives** (mean age 640–2,240 ticks, against a maturity of 300) may
   strike you as odd when watching. Predators live fast: one bite can bring up to half the
   energy needed to breed, and many die young.

## 8. Richer world (plant input 36k)

Seeds 1–3 at 300k:

- lineages by strict descent of 190k, 120k (115k alive at the end), and 55k (alive);
- an unbroken guild for 230k, 225k, and 270k, with a median guild of 103–212;
- single species ids of at most 110k;
- coexistence for 280–290k, with populations of 1090–1290.

Seeds 4–6 at 150k: lineages of 110k, 60k, and 100k, all alive at the end. More prey makes
bigger guilds but does not lengthen single-species spans. The richer input is therefore
not part of the recommended configuration.

## 9. How to watch it

Start the research web client with `npm run dev --prefix web` in the worktree, or with the
`research-web` launch config. Then do either of the following:

- **Jump to the end.** Choose *history → load run* and pick a saved world, such as
  `runs/seed2-300k.sevrun` in the durable copy. Loading was tested: the browser restores the
  native run exactly, paused at tick 300,000 with 1,005 agents and 17 species, and it runs on.
- **Watch it evolve.** Open the URL in `url.txt` (seed 2, 1,000 founders, the configuration
  above) and run at 100×. It reached 150k ticks in about 50 minutes here, with other runs
  competing for the CPU.

Then choose *species → color by → diet: plants to meat*. At 155–174k, red and orange
predators sit among the green grazers at the densest plant patch. They are visibly larger
dots, and 15 species are alive. This matches the native run.

## 10. Code state

29 files are changed (1,267 insertions) and three test files are added, all uncommitted.

Passing:

- `cargo test -p sim-core` (522);
- `cargo fmt` and `clippy -D warnings`;
- `npm test` (162) and `svelte-check`;
- 118 of 123 native tests;
- a native/WASM agreement scenario for every new mechanism (cryptic bite, gut and jaw curves,
  digestion waste, plasticity, handling time, eye drift, trait mutation), in all three
  heredity modes, including restore from a mid-meal checkpoint. It lives in
  `sim-core/tests/common/carnivory_case.rs`, uncommitted. Native (release) and
  `wasm-pack test --node` (debug) give identical hashes.

Expected failures in a PR, because params and per-agent arrays gained fields:

- the checkpoint cross-target fingerprints;
- the saved-run fixture;
- the footprint estimates;
- five native history tests. Older archives' recorded combat sections lack the new combat
  params. A PR must teach the history reader those params' defaults, so old archives stay
  readable, rather than regenerate the fixtures.

A PR would also bump the checkpoint format.

## 11. Follow-up: brains, perception, omnivory, founders

This adds a no-bite baseline: the same 7 seeds and parameters with the bite removed. It also
adds per-species brain and eye telemetry, and two verification passes. Data and scripts are
in `brains/`.

- **Brains grow slowly everywhere**: from 14 neurons at founding to 15.4–18.1 at 300k.
  - Once each world is measured from its own founders, the bite worlds grew 2.5 more brain
    units in 6 of 7 seeds. The difference is mixed for wired hidden neurons (4 of 7), and
    seed 3 dominates it.
  - Bite founders carry 10 extra units: the bite neuron and its 9 zero-weight links.
- **Predator lineages carry clearly bigger brains** than grazers and than any no-bite
  species: +4.9 to +14.1 neurons over founders, against +2.7 to +6.1, in 7 of 7 seeds at
  population ≥ 10.
  - This is most likely not selection. Predators breed 5–15× faster (parent-to-offspring
    gap 431–1,176 ticks, against 5,838–10,091). Their neuron gain *per generation* is no
    higher than grazers'.
  - Much of the gain is oscillators, which can never be removed, so they count generations.
  - Neutral splits of the zero-weight bite links add hidden neurons that only feed the
    bite.
  - In one seed-7 lineage the bigger brain came before the carnivory.
- **Perception is kept but not elaborated.**
  - The bite has no consistent effect on eyes per agent; a no-bite world (seed 1) also lost
    most of its sight.
  - New sensors rarely spread beyond 10–16%, no more than in the structural null.
  - The vision knockout does not speed eye loss, so eyes are not visibly maintained by
    selection.
  - Predators do not consistently look more forward (3 of 7 seeds).
  - Seed 2's predators are the exception: they evolved a second, forward eye, carried by
    about 65% of them.
  - "Wired sensors" counts are inflated by the zero-weight bite links.
- **Omnivores exist but are a minority.**
  - In the second half, 2.7–9.4% of fed agents have a 10–90% lifetime meat share. That is a
    ramp toward meat, not a separate omnivore peak.
  - Species in the 0.1–0.5 meat band come and go, lasting 10–80k ticks.
  - The structural null shows as many mixed diets in 3 of 4 seeds, so mixed diets alone are
    not evidence of an evolved generalist.
  - "Meat" includes carrion throughout.
- **All founders start as grazers.**
  - Within a seed they share one topology: 14 neurons, 4 drawn connections and 9 zero-weight
    links into the bite. All have gut 1.0, no jaw and size 3.
  - Their weights, biases, time constants, eye direction and colour vary.
  - With bias −1 and zero-weight inputs, the bite's output is σ(−1) = 0.27, below the 0.5
    gate, so no founder can bite in its lifetime.
  - Descendants first swing within 5–10k ticks in every seed. A weight reset on a cryptic
    link wakes the bite, with probability ≈ 0.5% per birth.
  - One seed-7 founder lived all 300k ticks as a pure plant-eater.
