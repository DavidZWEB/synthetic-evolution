"""Final evidence table for REPORT.md: one row per seed of the L10-m100 300k runs."""
import json, subprocess, re
def samples(path):
    out = []
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample': out.append(r['data'])
    return out
def guild_stats(ss, coh='evolving', mn=5):
    first = None; rs = None; best = 0; sizes = []; meat = []; gm = []
    for s in ss:
        m = s[coh]; pred = m.get('predation') or {}
        pops = (m.get('species') or {}).get('populations', [])
        g = [e for e in pops if (e.get('meat_share') or 0) > 0.5] if pred.get('herbivore_biomass', 0) > 0 else []
        n = sum(e['population'] for e in g)
        if n >= mn:
            first = first if first is not None else s['tick']
            rs = s['tick'] if rs is None else rs; best = max(best, s['tick'] - rs)
            meat.append(sum(e['population'] * e['meat_share'] for e in g) / n)
        else: rs = None
        if first is not None: sizes.append(n)
        h = [e for e in pops if e.get('meat_share') is not None and e['meat_share'] < 0.1]
        if h: gm.append(sum(e['population'] * e['meat_share'] for e in h) / sum(e['population'] for e in h))
    sizes.sort()
    return first, best, (sizes[len(sizes)//2] if sizes else 0), (sum(meat)/len(meat) if meat else 0), ss[-1][coh]['population']
rows = []
for s in range(1, 8):
    f = f'long/L10-m100-s{s}-scalar.jsonl'; h = f'long/L10-m100-s{s}-scalar.history.jsonl'
    ss = samples(f)
    first, stretch, med, gmeat, pop = guild_stats(ss)
    ctl = ss[-1]['random_control']['population']
    out = subprocess.run(['python3', 'lineage_strict.py', f, h], capture_output=True, text=True).stdout
    lin, single = int(re.search(r'strict lineage\s+(\d+)', out).group(1)), int(re.search(r'single species\s+(\d+)', out).group(1))
    ages = [l.split() for l in open(f'c25/age-s{s}.jsonl') if False]
    # ages from ages.py output
    a = subprocess.run(['python3', 'ages.py', f'c25/age-s{s}.jsonl'], capture_output=True, text=True).stdout.strip().splitlines()[1:]
    ca = [float(x.split()[3]) for x in a if x.split()[3] != '-']; ga = [float(x.split()[6]) for x in a]
    k = lambda v: f"{v//1000}k"
    print(f"| {s} | {k(first)} | {k(stretch)} | {med} | {gmeat:.2f} | {k(lin)} | {k(single)} | {sum(ca)/len(ca):,.0f} / {sum(ga)/len(ga):,.0f} | {pop} ({ctl}) |")
