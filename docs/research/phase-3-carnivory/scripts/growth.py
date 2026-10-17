"""Growth over each world's own founders (mean over 150-300k minus tick 0), bite vs no-bite, same seed,
and the biggest-brained species (pop >= 10) in each world over 250-320k."""
import json, statistics as st
D = '/Users/davidwebster/Dev/Git/synthetic-evolution/.claude/research/phase-3-carnivory/'
def load(p):
    out = []
    for line in open(p):
        try: r = json.loads(line)
        except Exception: break
        if r.get('kind') == 'sample': out.append(r['data'])
    return out
def vals(m):
    c = m['complexity']
    return {'neurons': c['neurons']['mean'], 'enabled': c['enabled_connections']['mean'],
            'connections': c['connections']['mean'], 'hidden': c['wiring']['wired_hidden_neurons']['mean'],
            'brain_units': m['brain_units']['mean']}
K = ['neurons', 'enabled', 'connections', 'hidden', 'brain_units']
print('founders (tick 0) bite vs no-bite:', vals(load(D+'data/L10-m100-s1-scalar.jsonl')[0]['evolving']), vals(load(D+'brains/nobite-s1.jsonl')[0]['evolving']))
print(f"{'seed':5}" + ''.join(f"{k:>22}" for k in K))
d = {k: [] for k in K}
for s in range(1, 8):
    row = []
    for path in (D+f'data/L10-m100-s{s}-scalar.jsonl', D+f'brains/nobite-s{s}.jsonl'):
        ss = load(path); f = vals(ss[0]['evolving'])
        w = [vals(x['evolving']) for x in ss if 150000 <= x['tick'] <= 300000]
        row.append({k: st.mean(v[k] for v in w) - f[k] for k in K})
    for k in K: d[k].append(row[0][k] - row[1][k])
    print(f"{s:<5}" + ''.join(f"{row[0][k]:>+10.2f}/{row[1][k]:<+10.2f}" for k in K))
print(f"{'diff':5}" + ''.join(f"{st.mean(d[k]):>+10.2f} ({sum(x>0 for x in d[k])}/7 up)  " for k in K))
