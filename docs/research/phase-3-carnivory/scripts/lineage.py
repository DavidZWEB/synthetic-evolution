"""Carnivorous lineages: chains of majority-meat species linked by descent.

usage: lineage.py <metrics.jsonl> <history.jsonl> [threshold=0.5]
A species is carnivorous at a sample when its living members' lifetime intake is more
than `threshold` meat and plant-eaters hold biomass beside it (spec §7.9's carnivore).
A carnivorous species at one sample continues a lineage from the previous sample if it
was carnivorous then, or descends (through species-origin parent links) from a species
that was. The span is the longest unbroken chain, ending anywhere; `alive` is whether a
chain reaches the last sample.
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
def ancestors(s):
    seen = set()
    while s in parent and s not in seen:
        seen.add(s); s = parent[s]; yield s
start = {}  # carnivorous species at previous sample -> chain start tick
best = (0, None, None); spans = []
for smp in samples:
    m = smp['evolving']; pred = m.get('predation') or {}
    pops = (m.get('species') or {}).get('populations', [])
    carn = [e['species_id'] for e in pops if (e.get('meat_share') or 0) > cut] if pred.get('herbivore_biomass', 0) > 0 else []
    nxt = {}
    for s in carn:
        if s in start:
            nxt[s] = start[s]; continue
        origin = smp['tick']
        for a in ancestors(s):
            if a in start:
                origin = min(origin, start[a]); break
        nxt[s] = origin
    for s, t0 in nxt.items():
        if smp['tick'] - t0 > best[0]:
            best = (smp['tick'] - t0, t0, smp['tick'])
    start = nxt
end = samples[-1]['tick'] if samples else 0
alive = max([end - t for t in start.values()], default=0)
print(f"{mpath.split('/')[-1][:30]:<30} longest carnivorous lineage {best[0]:>7} ticks "
      f"(from {best[1]} to {best[2]}); alive at end: span {alive}; species linked by descent: {len(parent)}")
