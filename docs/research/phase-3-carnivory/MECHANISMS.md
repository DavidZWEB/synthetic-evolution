# Mechanisms added in the research worktree (all off by default)

Each reproduces the previous world bit for bit when off (verified by state hash on the
same seed and params), carries `SimParams` fields with validation, and has unit tests.

## 1. Cryptic dormant bite — `founder.bite_cryptic`
The dormant bite neuron reads every founding source through a connection of weight 0,
with its bias at `combat.dormant_bias`, so no founder swings. Weight mutation varies the
circuit neutrally until some lineage's drive crosses the gate in some context.
- Why: NEAT protects new structure with fitness sharing, which needs a fitness score;
  this simulation has none. Hiding structure behind a threshold is how biology stores
  variation it has not yet used: evolutionary capacitance (Rutherford & Lindquist 1998,
  Hsp90), and cryptic variation that is partly exposed to selection is enriched for
  adaptations once revealed (Masel 2006).
- Effect: in the final configuration, sustained biting (≥ 50 hits per 5k ticks) begins at
  10–30k in all seven seeds, against ≈ 95k (or never) for the unwired dormant bite.
- Cost: one zero-weight connection per founding source; founders' drawn genes and the
  RNG stream are unchanged (the wiring takes no draw and sorts after drawn connections).

## 2. Gut body trait — `founder.gut`, `body.gut`, `metabolism.k_gut`, `digestion.*`, `mutation.gut_sigma`
A fraction γ in [0, 1]. Plant assimilation `floor + (1−floor)·γ^curve`, meat assimilation
`1 − penalty·γ`, upkeep `k_gut·(γ − 1)`. What a gut fails to assimilate is dissipated
and ledgered.
- Why: herbivores pay for fermentation with long, metabolically expensive guts (the
  gastrointestinal tract is ≈20–25% of whole-body O2 use in ruminants; Aiello & Wheeler
  1995 for the expensive-tissue trade-off); extraction shows diminishing returns with
  retention time (Penry & Jumars 1987); protein needs no fermentation. Intraguild
  predator and prey coexist when the prey is the better plant competitor (Holt & Polis
  1997). Guts follow a diet shift within tens of generations (Herrel et al. 2008).
- Effect: alone it does not separate guilds (it moves grazers' guts as much as
  predators'); with a meat penalty it stops biting from starting.

## 3. Diet-induced gut plasticity — `digestion.plasticity`, `acclimation`, `diet_memory`
The live gut tracks `set_point · (1 − plasticity · recent meat share)`.
- Why: guts remodel within days (Piersma & Lindström 1997); the spadefoot toad's
  carnivore morph, with its shorter gut, is induced by eating shrimp (Levis & Pfennig
  2016) — plasticity-first evolution.

## 4. Eye-direction drift — `mutation.organs.eye_drift_rate`, `eye_drift_sigma`
An eye's azimuth takes a wrapped normal step at birth.
- Why: eye placement is a classic predator/prey divergence; without it an eye looks
  where its founder's draw pointed it forever.

## 5. Holling handling time — `combat.handling_seconds`
After landing a hit, the biter's mouth is busy: it can eat carrion but cannot graze.
- Why: handling time is the parameter that limits every predator's intake (Holling
  1959) and the cost that makes hunting and grazing compete for one mouth (optimal
  foraging; Stephens & Krebs 1986). A specialist predator loses nothing; a patch
  omnivore loses its grazing.
- Effect at 2–3 s: grazers stop biting (their meat share falls to 0–3%), cannibalism
  falls, and the predator guild's meat share rises to 40–55%.
- Effect at 10 s (the final configuration): biting and grazing become nearly exclusive
  over a life. Individual diets turn bimodal, guild meat reaches 0.60–0.69, and a
  carnivorous guild persists to the end of all seven 300k runs.

## Telemetry (native shell only)
Per-species mean gut, size, mouth, muscle, kills, and eye offset; gut and eye
distributions; individual diet histogram; same-species and plant-eater hit counts;
`resume` with metrics, species history, and knockout retunes.
