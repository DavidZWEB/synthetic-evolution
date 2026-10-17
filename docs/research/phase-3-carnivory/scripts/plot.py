"""Small-multiples SVG: carnivore and plant-eater populations over time per seed."""
import json, sys
files = sys.argv[2:]; out = sys.argv[1]
W, H, pad = 760, 110, 38
rows = []
for f in files:
    xs = []
    for l in open(f):
        try: r = json.loads(l)
        except Exception: break
        if r.get('kind') != 'sample': continue
        m = r['data']['evolving']
        pops = (m.get('species') or {}).get('populations', [])
        carn = sum(e['population'] for e in pops if (e.get('meat_share') or 0) > 0.5)
        guild = sum(e['population'] for e in pops if (e.get('meat_share') or 0) >= 0.25)
        xs.append((r['data']['tick'], m['population'] - guild, guild, carn))
    rows.append((f.split('/')[-1].replace('-scalar.jsonl', ''), xs))
T = max(x[0] for _, xs in rows for x in xs)
svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{(H+12)*len(rows)+40}" font-family="Helvetica,Arial,sans-serif" font-size="11">',
       '<rect width="100%" height="100%" fill="#ffffff"/>',
       f'<text x="{pad}" y="16" font-size="13" font-weight="bold">Carnivores (species &gt;50% meat, red) and plant-eaters (green) over 300k ticks</text>']
for k, (name, xs) in enumerate(rows):
    y0 = 30 + k*(H+12)
    pmax = max(max(x[1] for x in xs), 1)
    cmax = max(max(x[3] for x in xs), 1)
    svg.append(f'<rect x="{pad}" y="{y0}" width="{W-pad-10}" height="{H}" fill="#f7f7f7" stroke="#ddd"/>')
    def px(t): return pad + (W-pad-10) * t / T
    def py(v, vmax): return y0 + H - (H-8) * v / vmax
    pts = ' '.join(f'{px(t):.1f},{py(p, pmax):.1f}' for t, p, g, c in xs)
    svg.append(f'<polyline fill="none" stroke="#2e8b57" stroke-width="1.5" points="{pts}"/>')
    pts = ' '.join(f'{px(t):.1f},{py(c, cmax):.1f}' for t, p, g, c in xs)
    svg.append(f'<polyline fill="none" stroke="#c0392b" stroke-width="1.8" points="{pts}"/>')
    svg.append(f'<text x="{pad+4}" y="{y0+12}">{name}  (plant-eaters max {pmax}, carnivores max {cmax})</text>')
for t in range(0, int(T)+1, 50000):
    x = pad + (W-pad-10) * t / T
    svg.append(f'<text x="{x:.0f}" y="{30+len(rows)*(H+12)+4}" text-anchor="middle" fill="#666">{t//1000}k</text>')
svg.append('</svg>')
open(out, 'w').write('\n'.join(svg))
print(out)
