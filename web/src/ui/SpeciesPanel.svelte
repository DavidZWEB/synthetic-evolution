<script>
  import { speciesLabel } from '../inspect/model.ts';
  import { NULL_SPECIES } from '../species/model.ts';
  import { speciesSwatch } from '../render/species-colors.js';

  let { sample, selected, message, colorMode, oncolor, onselect, onclose } = $props();

  const complexityRows = (complexity) => [
    ['all genes', complexity.genomeGenes],
    ['neurons', complexity.neurons],
    ['connections', complexity.connections],
    ['enabled connections', complexity.enabledConnections],
    ...(complexity.wiring ? [
      ['wired hidden neurons', complexity.wiring.wiredHiddenNeurons],
      ['wired sensors', complexity.wiring.wiredSensors],
      ['driven effectors', complexity.wiring.drivenEffectors],
    ] : []),
  ];
</script>

<section id="species-browser" aria-label="Live species" class="species-panel">
  <header>
    <strong>Live species</strong>
    <button onclick={onclose} aria-label="Close species browser">close</button>
  </header>
  <label>
    color by
    <select value={colorMode} onchange={(event) => oncolor(event.currentTarget.value)}>
      <option value="signature">genetic signature</option>
      <option value="species">species</option>
    </select>
  </label>
  <p class="help">Display colors do not change genetic signatures. IDs identify species only within this world.</p>
  {#if message}<p role="status">{message}</p>{/if}
  {#if sample}
    <p class="sample-time">as of tick {sample.tick.toString()} · {sample.population} agents</p>
    <p>{sample.populations.length} active species · {sample.unclassifiedPopulation} unclassified</p>
    <button disabled={selected === null} onclick={() => onselect(null)}>clear species highlight</button>
    <p class="help">Select a row to highlight its agents; other agents remain visible and selectable.</p>
    <ul aria-label="Active species populations">
      {#each sample.populations as row (row.id)}
        <li>
          <button class:selected={selected === row.id} aria-pressed={selected === row.id}
            aria-label={`Highlight species ${row.id}, ${row.population} agents`}
            onclick={() => onselect(row.id)}>
            <span><i aria-hidden="true" style:background={speciesSwatch(row.id)}></i>{speciesLabel(row.id)}</span>
            <span>{row.population}</span>
          </button>
        </li>
      {/each}
      <li>
        <button class:selected={selected === NULL_SPECIES} aria-pressed={selected === NULL_SPECIES}
          disabled={sample.unclassifiedPopulation === 0}
          aria-label={`Highlight unclassified agents, ${sample.unclassifiedPopulation} agents`}
          onclick={() => onselect(NULL_SPECIES)}>
          <span><i aria-hidden="true" style:background={speciesSwatch(NULL_SPECIES)}></i>unclassified</span>
          <span>{sample.unclassifiedPopulation}</span>
        </button>
      </li>
    </ul>
    {#if sample.populations.length === 0}<p>No classified species are active.</p>{/if}
    <h3 id="genome-complexity-heading">Genome complexity</h3>
    <p class="help">Sizes across all living agents at the same tick. Disabled connections still cost energy. "Wired" counts structure on a path from a sensor or clock to an effector; the rest cannot affect behavior. Observation only, not fitness.</p>
    {#if sample.population === 0}
      <p>No living agents.</p>
    {:else}
      <div class="table-scroll">
        <table aria-labelledby="genome-complexity-heading">
          <thead>
            <tr><th scope="col">measure</th><th scope="col">median</th><th scope="col">middle half</th><th scope="col">range</th><th scope="col">mean</th></tr>
          </thead>
          <tbody>
            {#each complexityRows(sample.complexity) as [label, d] (label)}
              <tr>
                <th scope="row">{label}</th>
                <td>{d.median}</td>
                <td>{d.p25}–{d.p75}</td>
                <td>{d.min}–{d.max}</td>
                <td>{d.mean.toFixed(1)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {:else}
    <p>Waiting for a species sample.</p>
  {/if}
</section>

<style>
  .species-panel {
    position: absolute;
    z-index: 4;
    inset: 0.75rem 0.75rem 0.75rem auto;
    width: min(27rem, calc(100% - 1.5rem));
    box-sizing: border-box;
    overflow: auto;
    overflow-wrap: anywhere;
    padding: 1rem;
    background: #14161af5;
    border: 1px solid #3b4252;
    border-radius: 4px;
  }
  header { display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
  label { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; margin-top: 1rem; }
  .help, .sample-time { color: #a8b0bf; }
  [role=status] { color: #eacb8a; }
  button, select {
    font: inherit;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #3b4252;
    border-radius: 3px;
    padding: 0.25rem 0.6rem;
  }
  button { cursor: pointer; }
  button:disabled { opacity: 0.4; cursor: default; }
  ul { list-style: none; padding: 0; }
  li { margin: 0.35rem 0; }
  li button { width: 100%; display: flex; justify-content: space-between; gap: 0.5rem; text-align: left; }
  li button.selected { background: #3b4252; border-color: #a8b0bf; }
  li button span:first-child { display: flex; align-items: center; gap: 0.5rem; }
  h3 { font-size: 1em; margin: 1.25rem 0 0.25rem; }
  .table-scroll { overflow-x: auto; }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85em;
    font-variant-numeric: tabular-nums;
    overflow-wrap: normal;
  }
  th, td { padding: 0.2rem 0.35rem; text-align: right; border-bottom: 1px solid #2a2f3a; }
  td { white-space: nowrap; }
  th[scope=row], thead th:first-child { text-align: left; font-weight: normal; }
  thead th { color: #a8b0bf; font-weight: normal; }
  i { display: inline-block; flex-shrink: 0; width: 0.7rem; height: 0.7rem; border-radius: 50%; }
</style>
