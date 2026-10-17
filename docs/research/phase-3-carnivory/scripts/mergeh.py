"""Concatenate species-history segments for lineage analysis: mergeh.py out first cont..."""
import json, sys
out, files = sys.argv[1], sys.argv[2:]
with open(out, 'w') as f:
    header = None
    events = []
    for path in files:
        for line in open(path):
            try: r = json.loads(line)
            except Exception: break
            if r.get('kind') == 'header': header = line
            elif r.get('kind') == 'event': events.append(line)
    f.write(header)
    for line in events: f.write(line)
print(out, len(events), 'events')
