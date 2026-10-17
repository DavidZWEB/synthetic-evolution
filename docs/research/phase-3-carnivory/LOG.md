# Phase 3 M6/M7 research log — toward an established carnivorous lineage

Goal (spec §8): a carnivorous lineage becomes established without going extinct or
eating everything. Measured as a species (meat share > 0.5 of its living members'
lifetime intake) persisting at least half a run on several seeds, with plant-eaters
beside it, against both controls. Nothing here is checked in.

## Economics (measured, saturated world, defaults)

- Energy actually absorbed by plants ≈ 87/tick for ≈ 1,100 agents → mean upkeep ≈
  0.078/tick per agent. Grazing on a plant runs at `rate·g²` ≈ 0.6/tick.
- A swing at the spec's cost 8 is ≈ 100 ticks of a typical agent's whole budget; a hit
  keeps 16·g² ≈ 10. Break-even needs a hit rate near 80%; observed rates are 5–26%.
  Biting cannot pay at spec values, which is why every spec-value cell selected it away.

## Chunk 1 (pre-wired bite, 200 founders, 200k ticks, cost 4/8/16, 0–1 eyes)
- No carnivore species in any cell. Best run (cost 4, eye) peaked at 9% meat, then 2%.
- Seed 1 with an eye went extinct in every cell: 200 founders → ~5 survivors (bottleneck).

## Chunk 2 (1,000 founders, 1 eye, 120k ticks; cost × mouthful × damage; dormant vs wired)
- Dormant bite, cost 2 / mouthful 60 / damage 0.25, seed 2: **discovered naturally at
  ≈95k ticks**; by 120k a 55-member species ate 40% meat, swings doubling per 10k ticks.
- Wired bite, cost 2 / mouthful 60 / damage 0.5: stable whole-population omnivory
  (10–15% meat; species at 30–50%), population depressed to 700–800. No species >50%.
- Costlier cells (4/40, 8/60) bite little.
- Reading: with rich meat, biting pays; without a digestive trade-off nothing pushes an
  omnivore toward specialization (Price et al. 2012; Holt & Polis 1997).

## Mechanisms added (local, uncommitted; defaults reproduce the old world bit for bit)
1. **Gut body trait** (`founder.gut`, `body.gut`, `metabolism.k_gut`, `digestion.*`):
   plant assimilation `floor + (1−floor)·γ^curve`, meat assimilation `1 − penalty·γ`,
   upkeep `k_gut·(γ−1)`; unassimilated energy dissipated and ledgered.
   Grounding: expensive gut tissue (Aiello & Wheeler 1995; GI tract ≈ 20–25% of whole-body
   O2 use in ruminants), diminishing extraction with retention time (Penry & Jumars 1987),
   protein needs no fermentation (Karasov & Martínez del Rio 2007), IGP coexistence needs
   the prey to be the better plant competitor (Holt & Polis 1997).
2. **Eye drift** (`mutation.organs.eye_drift_rate/sigma`): an eye's azimuth takes a
   wrapped normal step at birth. Forward eyes for hunters, lateral for prey.
3. **Cryptic dormant bite** (`founder.bite_cryptic`): the dormant bite neuron reads every
   founding source at weight 0; weight mutation varies it neutrally until released.
   Evolutionary capacitance (Rutherford & Lindquist 1998), cryptic variation enriched
   for adaptations (Masel 2006). NEAT protects new structure with fitness sharing, which
   needs a fitness score; this protects it by hiding it.
4. Tooling: gut and eye-offset distributions in native metrics; `resume --metrics` so a
   run can span several sub-2-hour chunks (resumed hash == unbroken hash, verified).

## Chunk 3 (gut with a meat penalty, linear curve; wired and dormant; 150k)
- A meat penalty on the herbivore gut **suppressed** the omnivory bridge: meat share fell
  below the gutless control and guts never moved off 1. Matches Price et al.: diet
  switches pass through omnivory, so omnivory must stay viable.
- → Next: meat equally digestible by every gut, concave plant extraction, larger k_gut.

## Chunk 4 (concave gut k 0.02–0.03, meat penalty 0; wired vs cryptic; 180k)
- **A predator guild diverges.** Wired, k 0.02, seed 2, 135k: seven species (≈185 agents,
  ≈22% of the population) at 38–49% meat with 0.8–2.3 kills per member, bodies larger
  (2.5–3.1 vs 2.2–2.3), guts slightly lower (0.89–0.98); the herbivore species beside them
  eat 1–10% meat with ≈0 kills. Larger predators, as in Loeuille & Loreau 2005.
- Cryptic bite, seed 2: a species at 54% meat branched off at 40k (10 → 76 members),
  killing 10× as often per member as the main species; later lineages at 26–33% meat
  with mouths 1.4–1.6 (vs ≈1.05) and guts 0.93.
- Cryptic bite discovers biting by 25–30k ticks (dormant: ≈95k when at all).
- Guts barely move (σ 0.05 at rate 0.1 is ≈0.016 per generation): the predator guild
  sits just under 50% meat. → Chunk 5: gut σ 0.15, k 0.04; eye drift 0.2 / 0.4 rad.

## Chunk 5 (cryptic, concave gut, k 0.03–0.04, gut σ 0.15, eyes 0.2/0.4; 180k) + two 330k resumes
- No persistent carnivore species. Meat-eating species reach 30–37% meat with guts
  0.84–0.95; herbivores ≈0.96–1.0. Gut selection is weaker than estimated: big mouths
  (1.2–2.0) raise plant intake, and k_gut is small against it.
- **Failure mode, 330k (slow-gut config):** every species converges on the smallest body
  (size 1.5, the floor) with huge mouths (1.8–3.0): gape 1.45 kills in one hit and takes
  ≈126 per bite. A symmetric "everyone bites everyone" society at 14–27% meat, population
  falling 845 → 520. With a meat penalty of 0, every gut profits from a bite equally, so
  nothing separates predator from prey.
- Process note: chunk 6 was first launched with `&` by mistake; killed within a minute,
  outputs deleted, relaunched as a tracked task.
- → Chunk 6: two-sided trade-off (meat penalty 0.5/0.7) with the concave curve, so a
  herbivore's gut makes a bite pay little while the first steps down the gut stay cheap.

## Chunks 6–7 (two-sided gut: meat penalty 0.5–0.7; founders' gut 1, then 0.5–0.7)
- With a meat penalty, biting never starts: 0–3% meat by 50–80k in every seed, whether
  founders start as herbivores (gut 1) or generalists (gut 0.5/0.7, whose guts then rise
  toward herbivory, 0.62–1.0, before the cryptic bite is found). Stopped early.
- Reading: one number does two jobs. Meat's value to a herbivore-gutted grazer decides
  whether biting gets started (needs ≈ mouthful 60 at full value); the penalty that
  separates guilds lowers exactly that value. With penalty 0 biting starts but nothing
  separates predator from prey; with penalty > 0 the guilds would separate but biting
  never starts.
- → Chunk 8: raise the mouthful so a herbivore's discounted bite is still worth ≈ 60
  (mouthful 120, penalty 0.5; 150/0.6), so biting can start while a short gut still doubles
  its value; plus diet-induced gut plasticity (spadefoot model) as the individual-level
  bridge.

## Working model of the gut's selection gradient (for choosing parameters)
Fitness rate of a lineage eating meat share f of intake I, gut γ:
  F(γ) = (1−f)·I·e_p(γ) + f·I·(1 − p·γ) − k·γ,   e_p(γ) = γ^c (c = plant_curve).
- Measured I ≈ 0.13–0.19 per tick in saturated worlds (lifetime intake / age), so
  k_gut must be a sizeable fraction of I for the gut to respond: k ≲ 0.03 is nearly
  neutral once mouths grow.
- Herbivores keep γ = 1 iff c·I > k + 0 (at f = 0): with c 0.5, I 0.15 → k < 0.075.
- A 20%-meat lineage moving γ 1 → 0.5 gains −(1−0.5^c)·0.8·I + 0.5·0.2·p·I + 0.5·k:
  c 0.5, p 0.5, k 0.04: −0.007 (held); k 0.06: +0.003; c 0.3, k 0.04: +0.005.
  So a more concave curve or a larger k lets omnivores shed gut while herbivores keep it.

## Chunk 8 (meat penalty 0.5–0.6 with mouthful 120–150; plastic gut 0.8) — stopped at 75–120k
- Biting stays rare: >90% of individuals eat <10% meat; the plastic gut adds only
  1.6–2.9% meat. The two-sided trade-off does not get started however rich the bite.

## Back to the closest result: chunk 4, wired, damage 0.5, k 0.02, seed 2, at 180k
- A **predator guild persists from ≈50k to 180k**: 5–7 species (≈200 of 838 agents),
  bigger bodies (2.4–2.7 vs 2.0–2.2), 1.2–1.4 kills per member (herbivores ≈0),
  33–52% meat, guts 0.83–0.95; beside ≈600 herbivores at 0–4% meat. Diet is bimodal at
  the species level. Species ids in the guild turn over every 30–60k ticks, so no single
  id is carnivorous for half a run.
- Damage 0.5 makes kills size-structured (damage ∝ gape / victim size), and the guild is
  the larger-bodied one: size-based trophic structure as in Loeuille & Loreau 2005.
  Damage 0.25 drains prey by mouthfuls instead and the guild never forms.
- → Chunk 9: replicate this family (wired seeds 3–5; cryptic seeds 1–3), a faster/stronger
  gut variant, damage 0.75; and extend the guild run itself to 330k.
- Instrumented same-species hits (cannibalism vs predation) from chunk 9 on.

## The guild run extended to 330k (chunk 4 w-c05-k02 seed 2, slow gut, k 0.02, d 0.5)
- **The predator guild persists ≈300k ticks** (≈30k → 330k): 200–350 agents, 30–50% of
  the population, guild meat 31–44%, 0.7–1.4 kills per member, beside herbivores at 1–12%
  meat. Species ids within the guild turn over; the guild itself never disappears.
- Late in the run predators are eyeless (vision organ lost) with mouths ≈2.0 and bodies
  ≈1.9 (herbivores 1.6–1.7): ambush omnivores at plant patches, grazing and biting
  whoever is beside them — hence the mixed ≈40% diet.
- Chunk 9 adds a cannibalism count: 40–70% of hits land on the biter's own species
  (falling from 100% to 39% as a guild forms in one run).
- Species are brain-genotype clusters (body traits, the gut included, do not enter the
  distance), so a species can hold biting and grazing genotypes at once. A clade-level
  measure (carnivorous species linked by descent, from the species history) is now
  recorded: `lineage.py`.

## Chunk 9 (damage 0.5/0.75, k 0.02, mouthful 60, cost 2; cryptic seeds 1–3, wired 3–5)
- **The predator guild replicates in all 10 runs**, cryptic (no pre-wiring) as well as
  wired: at 120–180k a guild of 100–300 agents (15–40% of the population) at 26–43% meat,
  0.65–1.6 kills per member, bodies 2.0–2.6 vs herbivores' 1.8–2.4, beside herbivores
  at 0–15% meat. Cannibalism 34–78% of hits. Guild guts 0.90–0.98.
- No species >50% for long: the guild is a band of predatory omnivores.

## Chunk 10 (guild config with k_gut 0.06/0.08, gut σ 0.15; cryptic and wired)
- The guild forms in every run again (guild meat 26–41%), but guts fall in grazers and
  predators alike (k 0.08: both ≈0.80–0.90), so the gut does not separate them.
  Carnivorous lineages (>50%, linked by descent): ≤20k ticks.
- Descent: about half of new guild species have guild parents, half grazer parents — the
  guild partly breeds true and is partly refilled by grazer lineages that start biting.
- Reading: the predators are patch omnivores, and that is optimal here: grazing is free
  where prey gather, and a bite returns about what grazing does (≈0.1–0.2 per tick).
  A predator's diet goes past half meat only if hunting returns several times grazing.
- → Chunk 11: skill-selecting bites (cost 6–8, mouthful 120–160: an untargeted swing
  roughly breaks even, a skilled hunter's is worth many ticks of grazing), and a mild
  two-sided gut with plasticity (penalty 0.3–0.4).

## Chunk 11 (stopped early except P1)
- Skill-selecting bites (cost 6/8, mouthful 120/160): **no biting at all** by 65–75k —
  an unskilled swing that only breaks even is still enough to keep the cryptic bite
  from ever being tried in earnest. Stopped.
- Mild two-sided gut (penalty 0.3–0.4, mouthful 80–100), with or without plasticity:
  1–5% meat by 70k. P2/P3 stopped; P1 (plastic) left to finish.

## Mechanism added: Holling handling time (`combat.handling_seconds`, default 0)
- After landing a hit the biter's mouth is busy for that long: it cannot graze or
  scavenge. A pure predator loses nothing; a patch omnivore loses the grazing it would
  have done. Targets the diagnosis above (patch omnivory is optimal because grazing is
  free where the prey are) without touching the bite's value, so biting can still start.
- → Chunk 12: handling 1 s and 2 s on the guild configuration (cryptic and wired).
- P1 (penalty 0.3 + plasticity 0.8, mouthful 80), 180k: guild 119–230 agents at 30% meat,
  1.3–1.5 kills per member, herbivores 3–4%; cannibalism 40–48%. No lineage >50%.
- Chunk 12 at 60–75k: with handling 1 s **grazers stop biting** (herbivore meat 0.00–0.04)
  and cannibalism falls to 26–28% (from 40–70%); the guild (≈60–90 agents, 31–41% meat,
  0.9–1.8 kills per member) is cleaner. Handling 2 s delays the guild.

## Chunk 12 (handling 1 s / 2 s; guild config, k 0.04 or 0.02; cryptic and wired), 180k
- The guild forms in all 10 runs; grazers almost stop biting (herbivore meat 0.00–0.06).
- Handling 2 s: guild meat 35–41%, carnivorous lineages (>50%, linked by descent) last
  15–30k ticks, one still alive at the end; handling 1 s: 5–20k. Cannibalism 39–71%.
- Handling pushes in the right direction (grazers stop biting; predators graze less) but
  1–2 s is not enough for a lineage to stay above half meat.
- → Chunk 13: handling 3 s and 4 s; H2 replicated on seeds 4–6; the two H2 runs extended
  to 330k with history.

## Chunk 13 at 100–140k (handling 2/3/4 s, cryptic, k 0.04; under heavy host load)
- Longer handling delays the guild but raises its meat share once formed: handling 4 s,
  seed 2: a guild of 19 at **70% meat**, 2.5 kills per member, a carnivorous lineage
  alive; 3–4 s elsewhere: guilds of 93–167 at 33–45%.
- Flaw found in handling as first built: a predator could not eat the carrion of its own
  kill while handling. Changed: handling blocks grazing only; carrion is meat and allowed.
- → Chunk 14: carrion-allowed handling 2 s / 3 s; 3 s with plasticity and k 0.06; 3 s
  with a 90° bite arc.
- Chunk 13 final (180k): guilds of 30–313 agents at 31–48% meat beside grazers at 0–3%;
  carnivorous lineages (>50%, by descent) last 10–35k ticks (handling 3 s, seed 1: 35k).
- The two handling-2 s runs extended to 330k: the guild never disappears (221–280 agents)
  but drifts to 28–33% meat; lineages >50% at most 20–30k.
- Summary so far: **coexistence of a predator guild with grazers is robust** (every seed,
  whole runs, up to 330k); **majority-meat lineages recur but do not persist** (≤35k).
- Handling 3 s, seed 1, 170k: **a single clean predator species** (78 members, 53% meat,
  2.45 kills per member, body 2.75 vs 1.75–2.2) beside five grazer species at 0–1% meat.
  Handling 4 s, seed 1, 180k: predator species at 42–50% meat with guts 0.71–0.92 (grazers
  0.96–0.97), bodies 2.5–3.0 vs 2.3. The predator species sits right at half meat, and
  its gut is still falling — the trend needs a longer run.

## Chunk 14 (carrion allowed during handling, 2 s / 3 s; 3 s + plasticity/k 0.06; 3 s + 90° arc), 150k
- Same picture as chunk 13: guilds of 81–399 agents at 27–51% meat, 0.8–1.8 kills per member,
  grazers at 0–7%; lineages >50% by descent last up to 15k (two still alive at the end).
  Most hits land on plant-eaters (59–83%); cannibalism 39–77%. Carrion access, plasticity
  and a wider arc do not move the guild decisively above half meat by 150k.
- → Chunk 15: extend the handling 3–4 s runs (chunk 13) to 330k and the carrion-handling
  3 s runs (chunk 14) to 300k, to see whether predator guts keep falling and a
  majority-meat lineage consolidates once given time.

## Chunk 15: the best handling runs extended to 300–330k
- All ten keep a predator guild for the whole run (143–301 agents, 30–49% meat, 0.8–1.5 kills
  per member) beside grazers at 0–2%. Majority-meat lineages by descent last up to 60k
  (carrion handling + 90° arc, seed 2), 40k, 35k, 30k (alive) — far short of half a run.
- Diagnosis: wherever predators can still graze, their diet settles where hunting and
  grazing pay alike (≈40–50% meat). The gut only taxes plants (penalty 0) and evolves
  slowly; nothing makes a better hunter a worse grazer.
- → New mechanism: a **jaw** body trait trading grazing for biting (dentition / mouth form;
  Pristionchus predatory mouth form: Serobyan et al. 2014; jaw force–speed trade-off:
  Westneat 1994).

## Chunk 16 (handling 3–4 s with k_gut 0.06–0.10, gut σ 0.25) — stopped at 80–110k
- Guilds at 27–48% meat, lineages >50% ≤10k: a stronger, faster gut does not change the
  picture. Stopped to give the cores to the jaw experiment.

## Chunk 17 (linear jaw: graze ×(1−jaw), bite ×(1+jaw); founders' jaw 0) — stopped at 110–150k
- Guild jaws stay at 0.00–0.07 and guild meat at 28–49%: with a linear trade-off a lineage
  eating under ~60% meat loses more grazing than its bite gains, so the jaw never leaves 0
  (the same threshold the gut has). Same plateau.
- → Concave/convex jaw (exponents): bite gain ∝ jaw^0.5 (the first changes add most force),
  grazing loss ∝ jaw² (sharpened teeth still crop; carnassials cannot grind). A lineage
  that bites at all then gains from a partly killer's jaw, and the gain feeds back.
  Predicted fixed point for a 40%-meat lineage: jaw ≈0.3–0.5 and ≈55–75% meat.

## Chunk 18 (concave jaw: bite ∝ jaw^0.5, grazing loss ∝ jaw²; gain 1/2; handling 0/2 s), 150k
- Predator jaws now evolve (guild 0.16–0.42 vs grazers ≈0.05–0.11) but guild meat stays
  ≈28–41%; lineages >50% ≤5k. Biting harder per hit does not change how rarely predators
  hit: ≈1–1.7 kills per member over a life.
- Reading: hunting yield, not digestion or bite strength, now limits predators. Most
  predators' single eye points sideways or back (median offset 1–3 rad), so they bite
  whatever bumps into their forward arc.
- New option `sensing.vision_azimuth_spread` (default π, the old draw): new eyes arise
  within ±spread of the heading (cephalization). → Chunk 19: founders' eyes within ±0.3
  rad (one eye) or ±0.6 (two eyes), with the concave jaw at gain 2; controls: forward
  eyes without the jaw; jaw with eyes anywhere.

## Chunk 19 (founders' eyes forward ±0.3 rad, one or two eyes; concave jaw gain 2), 150k
- No change to the plateau: guilds at 27–42% meat, ≈1–1.3 kills per member, lineages >50% ≤5k.
- Across chunks 9–19 the plateau is the same whatever is changed (handling, gut, plasticity,
  jaw, eyes): a predator kills ≈1–1.5 prey in its life, and grazing supplies the rest.
- Reading (Holt & Polis IGP): an omnivore's density is set by both resources, so the guild
  grows on plants until prey per predator are scarce and each kills rarely. Majority-meat
  predators need their density held by prey, i.e. biting and grazing must be close to
  mutually exclusive across a life, not only across a few seconds.
- → Chunk 20: long handling (6 s and 10 s, i.e. 360–600 ticks without grazing per hit) — a
  saturating Holling type II response — with mouthful 60 or 100 so the first biters can
  still pay for the grazing they give up.

## Chunk 20 (long handling, 6 s / 10 s; mouthful 60/100/150) — **breakthrough** (interim, ≈75–105k)
- Handling 10 s, mouthful 100, seed 3: **a carnivorous lineage alive from 20k to 105k and
  counting** (85k by descent). Guild species at 59–91% meat (guild 15 → 66 agents), guts
  falling 0.97 → 0.49–0.65, bodies 3.0–3.3 vs grazers' 2.2–2.9, grazers at 0–1% meat.
  **Individual diets are bimodal**: a cluster at ≥90% meat (33 agents) beside ≈630 grazers
  at <10% — specialists, not a band of omnivores.
- Handling 10 s, mouthful 150, seed 1: lineage alive 25k. Handling 6 s, mouthful 100, seed 3:
  alive 10k. Others not yet.
- Why it works: 600 ticks without grazing per hit makes biting and grazing close to
  mutually exclusive over a life. A grazer gives up ≈90 energy of grazing per bite, so
  grazers stop biting; a lineage that hits often loses nothing, so it commits, sheds its
  gut, and its numbers are held by prey (a saturating Holling type II response).
- Chunk 20 final, 150k: carnivorous lineages by descent in 9 of 10 runs, **7 alive at the end**:
  handling 10 s / mouthful 100, seed 3: **130k of 150k, alive** (single species 70k);
  seed 2: 50k alive; mouthful 150, seed 2: 70k alive; seed 1: 50k; handling 6 s: 15–35k.
  Guilds at 55–62% meat under 10 s handling (≈35% before), grazers 0–1%.
- → Chunk 21: extend the 10 s runs (and the best 6 s run) to 300k; replicate 10 s / mouthful
  100 on seeds 4–7. Then M7 acceptance on that configuration.

## Chunk 21: handling 10 s / mouthful 100 to 300k (seeds 1–3), replicates at 150k (seeds 4–7)
- 300k, by descent (`lineage.py` on stitched segments): seed 1 **130k, alive**; seed 2
  **200k, alive** (single species 130k); seed 3 **235k** (20k → 255k), then a new one alive.
  Guild meat 63–75%; grazers 0–3%; prey populations 820–1150, no collapse.
- Official `summarize` (3 seeds × 300k): carnivore_species at the end 2–5 in every seed,
  coexistence_span 235–295k, first carnivore 20–115k; scalar control 4–9 agents, none.
  **persistent_carnivores = 0**: no single species id stays >50% for 150k — species ids
  turn over every 50–130k (fixed representatives; small predator populations drift fast).
- Replicates at 150k: lineages 75k (30k alive), 55k alive, 20k, 40k alive.
- Running: chunk 22 — vision and chemo knockouts from the 150k boundary (seeds 1–3) and
  structural-null controls (seeds 1–4, 300k).

## Chunk 22: knockouts from the 150k boundary (seeds 1–3) and structural-null controls
- Vision off at 150k: carnivorous lineages continue (85k alive; 140k; 215k ended); guild meat
  0.34–0.67 vs 0.63–0.75 intact. Chemo off: 50k alive; 185k (single species **160k**);
  265k ended; guild meat 0.35–0.74. Carnivory survives either knockout; meat share falls in
  2 of 3 seeds, so the senses help but are not required (predators ambush at patches).
- Structural-null controls (interim 100–135k): viable (≈800–1100 agents), some meat eating
  (3–7%), small guilds (6–50), carnivorous species short-lived (≤10k). Scalar controls
  collapse to 3–9 agents.

## Chunk 23: seeds 4–7 extended to 300k; richer worlds (plant input 30k / 36k)
- Handling 10 s / mouthful 100, **seven seeds at 300k**, longest carnivorous lineage by descent:
  130k (alive), 200k (alive), 235k, 160k, 130k, 40k (alive), 150k (alive) — **≥ half the run
  in 4 of 7**, alive at the end in 4 of 7. Single-species spans 40–130k. Guild meat 45–75%,
  grazers 0–3%, populations 820–1150, scalar controls 4–13.
- Plant input 36k (1.5×), 150k: lineages 95k (alive; single species 95k), 45k (alive),
  40k (alive) — bigger predator populations drift slower, so species ids last longer.
- → Extend the 36k runs to 300k: the per-species proxy (one id >50% meat for 150k) may pass.
- Structural-null controls at 300k (seeds 1–4): control guilds 13–114 at 36–54% meat, but
  carnivorous species last 10–30k at most; the evolving cohort's single species last 70–130k
  and its lineages 130–235k. Scalar controls: 4–13 agents.
- Richer world (plant input 36k) at 300k: lineages 190k, 120k (115k alive), 55k (alive); single
  species ≤110k; persistent_carnivores 0; coexistence 280–290k. Replicates at 150k: 110k, 60k,
  100k, all alive. More prey does not lengthen single-species spans.

## Chunk 24: richer-world extensions and replicates; richer-world structural nulls
- Plant input 36k, seeds 1–3 to 300k (above) and seeds 4–6 at 150k (110k, 60k, 100k, alive).
- Structural-null controls at plant input 36k (seeds 1–2, 300k): guilds ≥ 5 members for
  35k and 100k unbroken (median guild 6 and 35); single carnivorous species ≤ 45k.

## Chunk 25 (analysis + a 20k-tick diagnostic): what the species-id proxy is measuring
- **Grazer species ids last the whole run** (the founder species spans 300k in 6 of 7 seeds; other
  grazer ids up to 235k), so the classifier does not churn everything. Predator ids last ≤ 130k.
- **Predators live 3–12× shorter lives than grazers.** Per-species mean age at 305–320k
  (new `age`/`genes` per-species telemetry, native-gut16): carnivores 490–2,760 ticks,
  grazers 4,200–9,700, in all seven seeds. Mean age is a proxy for generation time: a
  predator lineage passes ≈ 3–12 generations for each grazer generation, so its genomes
  leave a fixed representative's 0.5 radius that much sooner *in ticks*. Measured in
  generations, a 100k-tick predator id (≈ 100 generations) outlasts a 300k-tick grazer id
  (≈ 50). Predator genomes are also larger (48–83 genes vs 48–57).
- **Strict descent** (`lineage_strict.py`: a new carnivorous species joins a chain only
  through ancestors never sampled at ≤ 50% meat): 125k, 200k, 235k, 150k, 130k, 40k, 150k —
  ≥ half the run in 4 of 7, essentially the lenient numbers. Richer world: 190k, 120k, 55k.
- **Guild presence** (`presence.py`: some species > 50% meat with ≥ 5 members, plant-eaters
  beside): first at 20–115k; longest unbroken stretch 130k, 200k, 265k, 160k, 225k, 135k,
  205k (≥ 150k in 5 of 7); coverage after first appearance 0.74–1.00; median guild 45–134.
  Richer world: 230k, 225k, 270k. **Structural nulls: 110k, 25k, 10k, 35k (median guild
  0–6)**; scalar controls: never. Carnivory never vanished for good in any evolving run —
  "lineage ended" in seeds 3–5 means the descent chain broke while the guild went on.
- Knockouts, guild presence: vision off 110k / 200k / 210k; smell off 80k / 200k / 265k.
- Corrections on re-measurement:
  - Chunk 22's knockout guild meat (0.34–0.74) pooled species at ≥ 25% meat. With the
    report's definition (species > 50% meat, ≥ 5 members, 150–300k), guild meat is
    0.60–0.69 whether a sense is knocked out or not.
  - Chunk 20's guts (0.49–0.65) were seed 3 at 75–105k. Over the second half of the 300k
    runs, guild gut medians are 0.71–0.90, falling as low as 0.46, against 0.93–0.96 for
    grazers.
- Cross-target check: an uncommitted native/WASM scenario covers every new mechanism in
  all three heredity modes with a mid-meal checkpoint, and the hashes are identical. The
  browser's *load run* restores the native 300k bundles (seed 2: 1,005 agents, 17 species).
