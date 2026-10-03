<script>
  import { layout, lineage, prune } from '../lineage/graph.js';
  import { align } from '../lineage/compare.js';

  let { archive, compare, onclose } = $props();

  const NODE_W = 64;
  const NODE_H = 28;
  const GAP_X = 16;
  const GAP_Y = 36;
  const SHOWN_DIFFERENCES = 40;

  let chosenCohort = $state(null);
  // A choice from another archive falls back to this archive's first cohort.
  const cohort = $derived(archive.header.data.cohorts.includes(chosenCohort)
    ? chosenCohort : archive.header.data.cohorts[0]);
  let pruned = $state(false);
  let selected = $state([]);
  let comparison = $state(null);
  let comparisonError = $state(null);

  const graph = $derived(lineage(archive, cohort));
  const visible = $derived(pruned ? prune(graph.nodes) : graph.nodes);
  const positions = $derived(layout(visible));
  const byId = $derived(new Map(positions.map((p) => [p.node.id, p])));
  const width = $derived(Math.max(1, ...positions.map((p) => p.column + 1)) * (NODE_W + GAP_X) + GAP_X);
  const height = $derived(Math.max(1, ...positions.map((p) => p.row + 1)) * (NODE_H + GAP_Y) + GAP_Y);
  const x = (p) => GAP_X + p.column * (NODE_W + GAP_X);
  const y = (p) => GAP_Y + p.row * (NODE_H + GAP_Y);

  function choose(id) {
    selected = selected.includes(id) ? selected.filter((s) => s !== id) : [...selected, id].slice(-2);
  }

  function chooseCohort(value) {
    chosenCohort = value;
    selected = [];
  }

  function why(node) {
    const representative = node.representative;
    if (representative === null) return 'this archive did not record genomes';
    return representative.status === 'unavailable' ? `unavailable (${representative.reason})` : null;
  }

  const pair = $derived(selected.map((id) => graph.nodes.find((node) => node.id === id)).filter(Boolean));

  $effect(() => {
    comparison = null;
    comparisonError = null;
    const [a, b] = pair;
    if (!a || !b || why(a) || why(b)) return;
    let current = true;
    compare(a.representative.genes, b.representative.genes, archive.header.data.params)
      .then((distance) => {
        if (current) comparison = { distance, aligned: align(a.representative.genes, b.representative.genes) };
      })
      .catch((error) => { if (current) comparisonError = String(error); });
    return () => { current = false; };
  });

  const format = (value) => (value === null ? '—' : Number(value).toFixed(3));
</script>

<section aria-label="Species lineage" class="lineage-panel">
  <header>
    <strong>Species lineage</strong>
    <button onclick={onclose} aria-label="Close species lineage">close</button>
  </header>
  <p class="help">
    Seed {archive.header.data.provenance.seed}. Each species links to its founding parent's
    species; depth is speciation steps, not time. Not every member descends from one ancestor.
  </p>
  <div class="controls">
    {#if archive.header.data.cohorts.length > 1}
      <label>cohort
        <select value={cohort} onchange={(event) => chooseCohort(event.currentTarget.value)}>
          {#each archive.header.data.cohorts as option (option)}<option value={option}>{option}</option>{/each}
        </select>
      </label>
    {/if}
    <label><input type="checkbox" checked={pruned} onchange={(event) => (pruned = event.currentTarget.checked)} />
      hide extinct lineages without descendants</label>
  </div>
  {#if graph.unknownBefore}
    <p class="notice">Some lineage is unknown: capture gaps or a resumed segment hide earlier origins (marked ?).</p>
  {/if}
  {#if visible.length === 0}
    <p>No species origins recorded for this cohort.</p>
  {:else}
    <div class="graph" role="group" aria-label="Species origin tree">
      <svg {width} {height} viewBox={`0 0 ${width} ${height}`}>
        {#each positions as p (p.node.id)}
          {#each p.node.parents as parent (parent)}
            {@const from = byId.get(parent)}
            {#if from}
              <line x1={x(from) + NODE_W / 2} y1={y(from) + NODE_H} x2={x(p) + NODE_W / 2} y2={y(p)} />
            {/if}
          {/each}
          {#if p.node.unknownParent}
            <text class="unknown" x={x(p) + NODE_W / 2} y={y(p) - 6}>?</text>
          {/if}
          <g role="button" tabindex="0" class:extinct={p.node.extinctTick !== null}
            class:selected={selected.includes(p.node.id)}
            aria-pressed={selected.includes(p.node.id)}
            aria-label={`Species ${p.node.id}, origin tick ${p.node.originTick}${p.node.extinctTick === null ? '' : `, extinct at tick ${p.node.extinctTick}`}`}
            onclick={() => choose(p.node.id)}
            onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(p.node.id); } }}>
            <rect x={x(p)} y={y(p)} width={NODE_W} height={NODE_H} rx="4" />
            <text x={x(p) + NODE_W / 2} y={y(p) + NODE_H / 2 + 4}>#{p.node.id}</text>
          </g>
        {/each}
      </svg>
    </div>
  {/if}

  <h3>Compare representatives</h3>
  {#if pair.length < 2}
    <p class="help">Select two species to compare their genomes at origin.</p>
  {:else}
    {#each pair as node (node.id)}
      {#if why(node)}<p class="notice">Species #{node.id}: representative {why(node)}.</p>{/if}
    {/each}
    {#if comparisonError}<p class="notice" role="status">{comparisonError}</p>{/if}
    {#if comparison}
      {@const d = comparison.distance}
      <p>Species #{pair[0].id} vs #{pair[1].id}: distance <strong>{format(d.value)}</strong>
        (threshold {format(d.threshold)})</p>
      <p class="help">disjoint {format(d.disjoint_term)} · excess {format(d.excess_term)} ·
        weight {format(d.weight_term)} over {d.matching_connections} matching connections</p>
      <div class="table-scroll">
        <table aria-label="Gene alignment">
          <thead><tr><th scope="col">kind</th><th scope="col">shared</th>
            <th scope="col">only #{pair[0].id}</th><th scope="col">only #{pair[1].id}</th></tr></thead>
          <tbody>
            {#each comparison.aligned.kinds as row (row.kind)}
              <tr><th scope="row">{row.kind}</th><td>{row.shared}</td><td>{row.onlyA}</td><td>{row.onlyB}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if comparison.aligned.connections.length === 0}
        <p>Shared connections have identical weights and enabled states.</p>
      {:else}
        <div class="table-scroll">
          <table aria-label="Connection differences">
            <thead><tr><th scope="col">connection</th><th scope="col">#{pair[0].id}</th>
              <th scope="col">#{pair[1].id}</th><th scope="col">difference</th></tr></thead>
            <tbody>
              {#each comparison.aligned.connections.slice(0, SHOWN_DIFFERENCES) as row (row.id)}
                <tr>
                  <th scope="row">i{row.id}</th>
                  <td>{row.a ? `${format(row.a.weight)}${row.a.enabled ? '' : ' (off)'}` : '—'}</td>
                  <td>{row.b ? `${format(row.b.weight)}${row.b.enabled ? '' : ' (off)'}` : '—'}</td>
                  <td>{row.delta === null ? `only #${row.a ? pair[0].id : pair[1].id}` : row.delta === 0 ? 'enabled state' : format(row.delta)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if comparison.aligned.connections.length > SHOWN_DIFFERENCES}
          <p class="help">{comparison.aligned.connections.length - SHOWN_DIFFERENCES} more differing connections not shown.</p>
        {/if}
      {/if}
    {/if}
  {/if}
</section>

<style>
  .lineage-panel {
    position: absolute;
    z-index: 4;
    inset: 0.75rem 0.75rem 0.75rem auto;
    width: min(36rem, calc(100% - 1.5rem));
    box-sizing: border-box;
    overflow: auto;
    padding: 1rem;
    background: #14161af5;
    border: 1px solid #3b4252;
    border-radius: 4px;
    overflow-wrap: anywhere;
  }
  header { display: flex; justify-content: space-between; align-items: center; gap: 1rem; }
  .help { color: #a8b0bf; }
  .notice { color: #eacb8a; }
  .controls { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; }
  .graph, .table-scroll { overflow-x: auto; max-width: 100%; }
  svg { display: block; }
  line { stroke: #6b7385; stroke-width: 1.5; }
  rect { fill: #1c1f26; stroke: #8fa1b3; }
  g[role=button] { cursor: pointer; }
  g.extinct rect { stroke-dasharray: 4 3; fill: #14161a; }
  g.selected rect { stroke: #eacb8a; stroke-width: 2.5; }
  g:focus-visible rect { stroke: #ffffff; stroke-width: 2.5; }
  text { fill: #d8dee9; font-size: 12px; text-anchor: middle; }
  text.unknown { fill: #eacb8a; }
  h3 { font-size: 1em; margin: 1.25rem 0 0.25rem; }
  table { border-collapse: collapse; font-size: 0.85em; font-variant-numeric: tabular-nums; width: 100%; }
  th, td { padding: 0.2rem 0.4rem; text-align: right; border-bottom: 1px solid #2a2f3a; white-space: nowrap; }
  th[scope=row], thead th:first-child { text-align: left; font-weight: normal; }
  button, select {
    font: inherit; color: #d8dee9; background: #1c1f26; border: 1px solid #3b4252;
    border-radius: 3px; padding: 0.25rem 0.6rem;
  }
</style>
