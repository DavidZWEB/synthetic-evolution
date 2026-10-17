"""Predator guild over time: species whose members eat >= `meat` meat, pooled.
usage: guild.py file [meat=0.25] [every=2]"""
import json, sys
path = sys.argv[1]; cut = float(sys.argv[2]) if len(sys.argv) > 2 else 0.25
every = int(sys.argv[3]) if len(sys.argv) > 3 else 2
print(f"{'tick':>7}{'pop':>5}{'gPop':>5}{'gSp':>4}{'gMeat':>6}{'gGut':>6}{'gJaw':>6}{'gSize':>6}{'gKill':>6}{'hMeat':>6}{'hGut':>6}{'hJaw':>6}{'hSize':>6}{'cann%':>6}  diet histogram")
n = 0; prev = None
for line in open(path):
    try: r = json.loads(line)
    except Exception: break
    if r.get('kind') != 'sample': continue
    n += 1
    s = r['data']; m = s['evolving']; p = m.get('predation') or {}
    if (n - 1) % every:
        prev = p; continue
    pops = (m.get('species') or {}).get('populations', [])
    g = [e for e in pops if (e.get('meat_share') or 0) >= cut]
    h = [e for e in pops if (e.get('meat_share') or 0) < cut]
    def wavg(es, key):
        tot = sum(e['population'] for e in es if e.get('traits'))
        return sum(e['traits'].get(key, 0) * e['population'] for e in es if e.get('traits')) / tot if tot else float('nan')
    def wmeat(es):
        tot = sum(e['population'] for e in es)
        return sum((e.get('meat_share') or 0) * e['population'] for e in es) / tot if tot else float('nan')
    dh = p.get('hits', 0) - (prev or {}).get('hits', 0)
    ds = p.get('same_species_hits', 0) - (prev or {}).get('same_species_hits', 0)
    cann = 100 * ds / dh if dh else float('nan')
    hist = p.get('diet_histogram')
    print(f"{s['tick']:>7}{m['population']:>5}{sum(e['population'] for e in g):>5}{len(g):>4}{wmeat(g):>6.2f}{wavg(g,'gut'):>6.2f}{wavg(g,'jaw'):>6.2f}{wavg(g,'size'):>6.2f}{wavg(g,'kills'):>6.2f}"
          f"{wmeat(h):>6.2f}{wavg(h,'gut'):>6.2f}{wavg(h,'jaw'):>6.2f}{wavg(h,'size'):>6.2f}{cann:>6.0f}  {hist if hist else ''}")
    prev = p
