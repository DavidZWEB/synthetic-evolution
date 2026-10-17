"""Carnivorous lineages by strict descent: like lineage.py, but a new carnivorous species
joins a chain only through ancestors that were never seen eating mostly plants.

usage: lineage_strict.py <metrics.jsonl> <history.jsonl> [threshold=0.5]
A species continues a chain if it was carnivorous at the previous sample, or if walking
up its species-origin parents reaches a species carnivorous at the previous sample
before reaching any species that was ever sampled at or below the threshold (a grazer
ancestor breaks descent, so carnivory re-evolved in a grazer clade starts a new chain).
Also prints the longest single-species carnivorous run for comparison.
"""
import json, sys
mpath, hpath = sys.argv[1], sys.argv[2]
cut = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5
parent = {}
for line in open(hpath):
    try: r = json.loads(line)
    except Exception: break
    if r.get('kind') != 'event': continue
    d = r['data']
    if d.get('cohort') != 'evolving': continue
    e = d['event']
    if e.get('kind') == 'species_origin':
        p = e.get('parent_a') or {}
        if p.get('status') == 'observed' and p.get('species_id') is not None:
            parent[e['species_id']] = p['species_id']
samples = []
for line in open(mpath):
    try: r = json.loads(line)
    except Exception: break
    if r.get('kind') == 'sample': samples.append(r['data'])
grazed = set()   # species ever sampled at or below the threshold (with diet data)
start = {}; best = (0, None, None); single = {}; best_single = 0
for smp in samples:
    m = smp['evolving']; pred = m.get('predation') or {}
    pops = (m.get('species') or {}).get('populations', [])
    plants_beside = pred.get('herbivore_biomass', 0) > 0
    carn = []
    for e in pops:
        share = e.get('meat_share')
        if share is None: continue
        if share > cut and plants_beside: carn.append(e['species_id'])
    nxt = {}; nsingle = {}
    for s in carn:
        nsingle[s] = single.get(s, smp['tick'])
        best_single = max(best_single, smp['tick'] - nsingle[s])
        if s in start:
            nxt[s] = start[s]; continue
        origin = smp['tick']; a = s; seen = set()
        while a in parent and a not in seen:
            seen.add(a); a = parent[a]
            if a in start: origin = min(origin, start[a]); break
            if a in grazed: break
        nxt[s] = origin
    for e in pops:
        share = e.get('meat_share')
        if share is not None and share <= cut and e['species_id'] not in nxt: grazed.add(e['species_id'])
    for s, t0 in nxt.items():
        if smp['tick'] - t0 > best[0]: best = (smp['tick'] - t0, t0, smp['tick'])
    start = nxt; single = nsingle
end = samples[-1]['tick'] if samples else 0
alive = max([end - t for t in start.values()], default=0)
name = mpath.split('/')[-1].replace('-scalar.jsonl', '')
print(f"{name:<22} strict lineage {best[0]:>7} ({best[1]}->{best[2]}); alive span at end {alive:>7}; longest single species {best_single:>7}")
