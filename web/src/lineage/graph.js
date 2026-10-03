/**
 * The species-origin graph of one archived cohort, as a layered tree (spec §3.4).
 * Built only from recorded origins and extinctions: a parent lost to a gap or a
 * resume is shown as unknown, never inferred. Owns no rendering or storage.
 */

function parentOf(parent) {
  if (parent.status === 'absent') return null;
  if (parent.status === 'observed' && parent.species_id !== null) return { species: parent.species_id };
  // Unavailable identity, or an observed but unclassified parent.
  return { species: null };
}

/**
 * `{ nodes, unknownBefore }` for `cohort`. Each node is
 * `{ id, originTick, extinctTick, parents, unknownParent, depth, representative }`;
 * `parents` lists in-graph parent species (two at most, so a later sexual DAG fits).
 */
export function lineage(archive, cohort) {
  const header = archive.header.data;
  const nodes = new Map();
  let unknownBefore = Object.hasOwn(header, 'resumed_from_tick');
  for (const row of archive.rows) {
    if (row.data.cohort !== cohort) continue;
    if (row.kind === 'gap') {
      unknownBefore = true;
      continue;
    }
    const event = row.data.event;
    if (event.kind === 'species_extinct') {
      const node = nodes.get(event.species_id);
      if (node) node.extinctTick = row.data.tick;
      continue;
    }
    const parents = [parentOf(event.parent_a), parentOf(event.parent_b)].filter(Boolean);
    const known = parents.filter((p) => p.species !== null && nodes.has(p.species)).map((p) => p.species);
    const node = {
      id: event.species_id,
      originTick: row.data.tick,
      extinctTick: null,
      parents: [...new Set(known)],
      // A named parent outside the graph was lost to a gap or predates a resume.
      unknownParent: parents.some((p) => p.species === null || !nodes.has(p.species)),
      depth: known.length === 0 ? 0 : 1 + Math.max(...known.map((id) => nodes.get(id).depth)),
      representative: row.data.representative ?? null,
    };
    nodes.set(node.id, node);
  }
  return { nodes: [...nodes.values()], unknownBefore };
}

/**
 * Hides extinct lineages without living descendants; every ancestor of a living
 * species stays, so pruning never breaks a surviving line.
 */
export function prune(nodes) {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const keep = new Set();
  const mark = (id) => {
    if (keep.has(id)) return;
    keep.add(id);
    for (const parent of byId.get(id)?.parents ?? []) mark(parent);
  };
  for (const node of nodes) if (node.extinctTick === null) mark(node.id);
  return nodes.filter((node) => keep.has(node.id));
}

/** Grid positions: one row per depth, species in origin order within a row. */
export function layout(nodes) {
  const columns = new Map();
  return nodes.map((node) => {
    const column = columns.get(node.depth) ?? 0;
    columns.set(node.depth, column + 1);
    return { node, row: node.depth, column };
  });
}
