"""Per-species mean age and genome length, carnivores (> 0.5 meat) against grazers
(< 0.1), population-weighted, at every sample of each file (native-gut16 metrics)."""
import json, sys
print(f"{'file':14}{'tick':>8}{'carn n':>8}{'carn age':>9}{'carn genes':>11}{'graz n':>8}{'graz age':>9}{'graz genes':>11}")
for path in sys.argv[1:]:
    for line in open(path):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') != 'sample': continue
        d = r['data']; pops = (d['evolving'].get('species') or {}).get('populations', [])
        acc = {'c': [0, 0.0, 0.0], 'g': [0, 0.0, 0.0]}
        for e in pops:
            t = e.get('traits') or {}; m = e.get('meat_share')
            if m is None or 'age' not in t: continue
            k = 'c' if m > 0.5 else ('g' if m < 0.1 else None)
            if not k: continue
            n = e['population']; a = acc[k]; a[0] += n; a[1] += n * t['age']; a[2] += n * t['genes']
        f = lambda a: (f"{a[0]:>8}{a[1]/a[0]:>9.0f}{a[2]/a[0]:>11.1f}" if a[0] else f"{0:>8}{'-':>9}{'-':>11}")
        print(f"{path.split('/')[-1][:14]:14}{d['tick']:>8}" + f(acc['c']) + f(acc['g']))
