"""Paired comparison, same seed, bite (L10-m100) vs no-bite baseline: evolving-cohort brain and
perception measures averaged over a window of samples. usage: compare.py [lo] [hi]"""
import json, sys, statistics as st
D = '/Users/davidwebster/Dev/Git/synthetic-evolution/.claude/research/phase-3-carnivory/'
lo = int(sys.argv[1]) if len(sys.argv) > 1 else 150000
hi = int(sys.argv[2]) if len(sys.argv) > 2 else 300000
def load(path):
    out = []
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample': out.append(r['data'])
    return out
def measures(m):
    c = m['complexity']; w = c['wiring']; pop = max(m['population'], 1)
    vis = [s for s in m['sensors'] if s['modality'] == 'VisionRay']
    chem = [s for s in m['sensors'] if s['modality'] == 'Chemo']
    found_eye = sum(s['carriers'] for s in vis if s['founding']) / pop
    return {
        'pop': m['population'], 'neurons': c['neurons']['mean'], 'enabled': c['enabled_connections']['mean'],
        'hidden': w['wired_hidden_neurons']['mean'], 'brain_units': m['brain_units']['mean'],
        'wired_sens': w['wired_sensors']['mean'], 'eyes/agent': sum(s['carriers'] for s in vis) / pop,
        'founding_eye': found_eye, 'new_eyes': sum(s['carriers'] for s in vis if not s['founding']) / pop,
        'chemo/agent': sum(s['carriers'] for s in chem) / pop,
        'eye_offset': m['bodies']['eye_offset']['median'], 'size': m['bodies']['size']['mean'],
    }
KEYS = ['pop', 'neurons', 'enabled', 'hidden', 'brain_units', 'wired_sens', 'eyes/agent', 'founding_eye', 'new_eyes', 'chemo/agent', 'eye_offset', 'size']
def window(ss):
    rows = [measures(s['evolving']) for s in ss if lo <= s['tick'] <= hi]
    return {k: st.mean(r[k] for r in rows) for k in KEYS}, len(rows)
print(f"window {lo}-{hi}; per seed: bite / no-bite")
print(f"{'seed':5}" + ''.join(f"{k:>18}" for k in KEYS))
diffs = {k: [] for k in KEYS}
for s in range(1, 8):
    a, na = window(load(D + f'data/L10-m100-s{s}-scalar.jsonl'))
    b, nb = window(load(D + f'brains/nobite-s{s}.jsonl'))
    print(f"{s:<5}" + ''.join(f"{a[k]:>9.2f}/{b[k]:<8.2f}" for k in KEYS) + f"  (n={na}/{nb})")
    for k in KEYS: diffs[k].append(a[k] - b[k])
print(f"{'diff':5}" + ''.join(f"{st.mean(diffs[k]):>+9.2f}{'':9}" for k in KEYS))
print(f"{'up':5}" + ''.join(f"{sum(d > 0 for d in diffs[k])}/7{'':14}" for k in KEYS))
