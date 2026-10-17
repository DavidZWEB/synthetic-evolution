"""Brain size of the biggest-brained species (pop >= 10): no-bite worlds over 250-300k vs bite worlds'
carnivorous species at 305-320k (diag). Brain measures are over the world's own founders: neurons - founder
neurons, wired hidden (founders 0)."""
import json
D = '/Users/davidwebster/Dev/Git/synthetic-evolution/.claude/research/phase-3-carnivory/brains/'
def samples(p, lo, hi):
    for line in open(p):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample' and lo <= r['data']['tick'] <= hi: yield r['data']
def best(p, lo, hi, base, pick):
    top = None
    for d in samples(p, lo, hi):
        for e in d['evolving']['species']['populations']:
            t = e.get('traits') or {}
            if e['population'] < 10 or 'neurons' not in t or not pick(e): continue
            k = (t['neurons'] - base, t['wired_hidden'], t['connections'], e['population'], e.get('meat_share') or 0, d['tick'])
            if top is None or k[0] > top[0]: top = k
    return top
print(f"{'seed':5}{'no-bite biggest species (d-neurons, hidden, enabled, pop)':>58}{'bite: biggest carnivorous species':>42}{'bite: biggest grazer species':>36}")
for s in range(1, 8):
    nb = best(D+f'nobite-s{s}.jsonl', 250000, 300000, 13, lambda e: True)
    bc = best(D+f'diag-s{s}.jsonl', 305000, 320000, 14, lambda e: (e.get('meat_share') or 0) > 0.5)
    bg = best(D+f'diag-s{s}.jsonl', 305000, 320000, 14, lambda e: e.get('meat_share') is not None and e['meat_share'] < 0.1)
    f = lambda t: f"+{t[0]:.1f}n {t[1]:.1f}h {t[2]:.1f}c pop{t[3]}" if t else '-'
    print(f"{s:<5}{f(nb):>58}{f(bc):>42}{f(bg):>36}")
