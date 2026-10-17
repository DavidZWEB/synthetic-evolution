"""Species meat shares over time: one row per sample, a column per species that ever
reached `min_pop` members with more than `min_meat` meat. usage: species.py file [min_pop] [min_meat] [every]"""
import json, sys
path = sys.argv[1]; min_pop = int(sys.argv[2]) if len(sys.argv) > 2 else 10
min_meat = float(sys.argv[3]) if len(sys.argv) > 3 else 0.3
every = int(sys.argv[4]) if len(sys.argv) > 4 else 1
samples = []
for line in open(path):
    try: r = json.loads(line)
    except Exception: break
    if r.get('kind') == 'sample': samples.append(r['data'])
keep = set()
for s in samples:
    for e in (s['evolving'].get('species') or {}).get('populations', []):
        if e['population'] >= min_pop and (e.get('meat_share') or 0) > min_meat:
            keep.add(e['species_id'])
keep = sorted(keep)[:14]
print('tick    pop  ' + ' '.join(f"sp{k:<9}" for k in keep))
for n, s in enumerate(samples):
    if n % every: continue
    by = {e['species_id']: e for e in (s['evolving'].get('species') or {}).get('populations', [])}
    cells = []
    for k in keep:
        e = by.get(k)
        cells.append(f"{(e.get('meat_share') or 0):.2f}/{e['population']:<5}" if e else ' ' * 10)
    print(f"{s['tick']:<7}{s['evolving']['population']:>4}  " + ' '.join(cells))
