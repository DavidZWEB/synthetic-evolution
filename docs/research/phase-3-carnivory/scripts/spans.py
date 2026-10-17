"""Species-id spans by diet, to tell classifier churn apart from carnivore instability.
usage: spans.py file... ; per run: longest span of any species id, of a grazer id
(lifetime meat < 0.1), of a carnivore id (majority-meat at every sample, pop >= 1),
and how many ids alive at the end were first sampled at least half the run earlier."""
import json, sys
def load(path):
    out = []
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample': out.append(r['data'])
    return out
print(f"{'run':34} {'end':>7} {'any':>7} {'grazer':>7} {'carniv':>7} {'persist':>7}  longest-lived ids (span/meat/maxpop)")
for path in sys.argv[1:]:
    ss = load(path)
    if not ss: continue
    end = ss[-1]['tick']; first, last, meat, maxpop = {}, {}, {}, {}
    run = {}; best_c = 0
    for s in ss:
        pops = (s['evolving'].get('species') or {}).get('populations', [])
        nxt = {}
        for e in pops:
            k = e['species_id']; first.setdefault(k, s['tick']); last[k] = s['tick']
            m = e.get('meat_share')
            if m is not None:
                a = meat.setdefault(k, [0.0, 0]); a[0] += m * e['population']; a[1] += e['population']
            maxpop[k] = max(maxpop.get(k, 0), e['population'])
            if m is not None and m > 0.5:
                nxt[k] = run.get(k, s['tick'])
                best_c = max(best_c, s['tick'] - nxt[k])
        run = nxt
    span = {k: last[k] - first[k] for k in first}
    mm = {k: (meat[k][0] / meat[k][1] if k in meat and meat[k][1] else 0) for k in first}
    big = [k for k in first if maxpop[k] >= 10]
    g = max([span[k] for k in big if mm[k] < 0.1] or [0])
    persist = sum(1 for k in first if last[k] == end and end - first[k] >= (end + 1) // 2)
    top = sorted(big, key=lambda k: -span[k])[:4]
    name = path.split('/')[-1].replace('-scalar.jsonl', '')
    print(f"{name:34} {end:>7} {max(span[k] for k in big):>7} {g:>7} {best_c:>7} {persist:>7}  " +
          ' '.join(f"{span[k]//1000}k/{mm[k]:.2f}/{maxpop[k]}" for k in top))
