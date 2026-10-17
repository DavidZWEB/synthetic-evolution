"""Join a run's metrics with its resumed continuations into one file.

usage: merge.py out.jsonl first.jsonl cont1.jsonl [cont2.jsonl ...]
The last file's header (whose ticks name the whole run) leads; samples follow in
order, each continuation's after the previous file's last tick. Earlier files'
final-state hashes are dropped, since only the end of the whole run is final.
"""
import json, sys
out, files = sys.argv[1], sys.argv[2:]
header, samples = None, []
for path in files:
    for line in open(path):
        r = json.loads(line)
        if r['kind'] == 'header':
            header = r
        elif r['kind'] == 'sample':
            if samples and r['data']['tick'] <= samples[-1]['data']['tick']:
                continue
            samples.append(r)
for r in samples[:-1]:
    r['data']['final_state_hashes'] = None
# History availability counts restart in each continuation's archive, so a joined file
# cannot carry them monotonically; the metrics reader requires that, so drop them.
for r in samples:
    for cohort in ('evolving', 'random_control'):
        if cohort in r['data'] and isinstance(r['data'][cohort], dict):
            r['data'][cohort]['history'] = None
# Event counters (spawn failures, structural mutations, species events) also restart.
for r in samples:
    for cohort in ('evolving', 'random_control'):
        m = r['data'].get(cohort)
        if isinstance(m, dict):
            for key in ('spawn_failures', 'structural_mutations'):
                m[key] = None
            if isinstance(m.get('species'), dict):
                m['species']['events'] = None
with open(out, 'w') as f:
    f.write(json.dumps(header) + '\n')
    for r in samples:
        f.write(json.dumps(r) + '\n')
print(f"{out}: {len(samples)} samples through tick {samples[-1]['data']['tick']}")
