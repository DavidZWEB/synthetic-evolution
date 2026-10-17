# Phase 3 carnivory research (reference branch, not for merge)

This branch preserves the Phase 3 M6–M7 research exactly as it ran: the uncommitted
mechanisms, telemetry, and tests, with the report and lab notebook. It is a reference for
the evidence cited in the spec. What worked lands on `main` through separate, reviewed
pull requests. This branch is never merged.

| file | what it is |
|---|---|
| `REPORT.md` | outcome, evidence against both controls, mechanisms, judgment calls, and the §11 follow-up on brains, perception, and omnivory |
| `LOG.md` | the chronological lab notebook (25 chunks) |
| `MECHANISMS.md` | each mechanism, its parameters, and its grounding |
| `L10-m100-params.json`, `url.txt` | the final configuration, and the browser URL for seed 2 |
| `scripts/` | analysis and run scripts |

The run data (about 27 MB of metrics, species histories, and saved worlds) is kept out of
git. It is deterministic: every run is a `(seed, params)` pair on this branch's code.

**Scripts.** They hard-code local paths and expect the research layout, so read them as a
record of each computation rather than as tools.

**Defaults.** With every new parameter at its default, the simulation hashes identically
to the base commit (`03ed1381`). Known gaps before any of this could merge:

- checkpoint fingerprints, the saved-run fixture, and the footprint estimates are stale;
- five native history tests fail, because older archives' combat sections lack the new
  params;
- the checkpoint format needs a bump.
