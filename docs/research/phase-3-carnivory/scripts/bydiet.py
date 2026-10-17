"""Population-weighted per-species traits for carnivorous species (> 0.5 meat), mixed
(0.1-0.5) and grazer species (< 0.1) at each sample of each diag file."""
import json, sys
F = ['neurons', 'connections', 'wired_hidden', 'eyes', 'wired_eyes', 'wired_sensors', 'eye_offset', 'genes', 'size', 'gut', 'age']
print(f"{'file':12}{'tick':>8}{'group':>8}{'n':>6}" + ''.join(f"{f[:10]:>11}" for f in F))
for path in sys.argv[1:]:
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') != 'sample': continue
        d = r['data']; pops = (d['evolving'].get('species') or {}).get('populations', [])
        groups = {'carn': [], 'mixed': [], 'graze': []}
        for e in pops:
            m = e.get('meat_share'); t = e.get('traits') or {}
            if m is None or 'neurons' not in t: continue
            g = 'carn' if m > 0.5 else ('graze' if m < 0.1 else 'mixed')
            groups[g].append(e)
        for g, es in groups.items():
            n = sum(e['population'] for e in es)
            if not n: continue
            vals = []
            for f in F:
                ws = [(e['population'], e['traits'][f]) for e in es if e['traits'].get(f) is not None]
                w = sum(p for p, _ in ws)
                vals.append(sum(p * v for p, v in ws) / w if w else float('nan'))
            print(f"{path.split('/')[-1][:12]:12}{d['tick']:>8}{g:>8}{n:>6}" + ''.join(f"{v:>11.2f}" for v in vals))
