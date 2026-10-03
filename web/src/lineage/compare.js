/**
 * Aligned comparison of two archived representative genomes by kind and innovation
 * ID. Pure bookkeeping for display; the distance itself comes from the core through
 * WASM so it is exactly the classifier's (spec §3.4).
 */

const KINDS = [['Neuron', 'neurons'], ['Sensor', 'sensors'], ['Effector', 'effectors'], ['Connection', 'connections']];

function index(genes) {
  const byKind = new Map(KINDS.map(([kind]) => [kind, new Map()]));
  for (const gene of genes) {
    const [kind, value] = Object.entries(gene)[0];
    byKind.get(kind)?.set(value.id, value);
  }
  return byKind;
}

/**
 * `{ kinds, connections }`: per-kind shared/only counts, and every connection whose
 * presence, weight, or enabled state differs, ordered by innovation ID.
 */
export function align(genesA, genesB) {
  const a = index(genesA);
  const b = index(genesB);
  const kinds = KINDS.map(([kind, label]) => {
    const left = a.get(kind);
    const right = b.get(kind);
    const shared = [...left.keys()].filter((id) => right.has(id)).length;
    return { kind: label, shared, onlyA: left.size - shared, onlyB: right.size - shared };
  });
  const left = a.get('Connection');
  const right = b.get('Connection');
  const ids = [...new Set([...left.keys(), ...right.keys()])].sort((x, y) => x - y);
  const connections = ids.flatMap((id) => {
    const x = left.get(id) ?? null;
    const y = right.get(id) ?? null;
    if (x && y && x.weight === y.weight && x.enabled === y.enabled) return [];
    return [{
      id,
      a: x && { weight: x.weight, enabled: x.enabled },
      b: y && { weight: y.weight, enabled: y.enabled },
      delta: x && y ? Math.abs(x.weight - y.weight) : null,
    }];
  });
  return { kinds, connections };
}
