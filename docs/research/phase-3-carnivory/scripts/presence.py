"""Guild presence: at each sample, is any species with more than half its intake from
meat alive (with plant-eaters beside it)? Reports the first such sample, the longest
unbroken stretch of samples with one, coverage after first appearance, guild size, and
the share of individuals at >= 90% and < 10% meat.
usage: presence.py file... [--min N]  (N = minimum guild members to count, default 1)"""
import json, sys, os
COH = os.environ.get('COHORT', 'evolving')
argv = sys.argv[1:]; mn = 1
if '--min' in argv:
    i = argv.index('--min'); mn = int(argv[i + 1]); del argv[i:i + 2]
args = argv
print(f"{'run':24}{'end':>7}{'first':>7}{'stretch':>9}{'cover':>7}{'guild med':>10}{'guild min':>10}{'>=90%':>7}{'<10%':>7}")
for path in args:
    ss = []
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample': ss.append(r['data'])
    first = None; run_start = None; best = 0; present = 0; after = 0; sizes = []; hi = []; lo = []
    for s in ss:
        m = s[COH]; pred = m.get('predation') or {}
        pops = (m.get('species') or {}).get('populations', [])
        g = sum(e['population'] for e in pops if (e.get('meat_share') or 0) > 0.5) if pred.get('herbivore_biomass', 0) > 0 else 0
        ok = g >= mn
        if ok and first is None: first = s['tick']
        if first is not None:
            after += 1; present += ok; sizes.append(g)
            h = m.get('diet_histogram') or (m.get('predation') or {}).get('diet_histogram')
            if h and sum(h): hi.append(h[-1] / sum(h)); lo.append(h[0] / sum(h))
        if ok:
            run_start = s['tick'] if run_start is None else run_start
            best = max(best, s['tick'] - run_start)
        else: run_start = None
    end = ss[-1]['tick']; sizes.sort()
    med = sizes[len(sizes) // 2] if sizes else 0; low = sizes[0] if sizes else 0
    f = lambda xs: f"{sum(xs)/len(xs):.2f}" if xs else '  -'
    name = path.split('/')[-1].replace('-scalar.jsonl', '').replace('.jsonl', '')
    print(f"{name:24}{end:>7}{str(first):>7}{best:>9}{(present/after if after else 0):>7.2f}{med:>10}{low:>10}{f(hi):>7}{f(lo):>7}")
