<script lang="ts">
  import type { Inspection } from '../inspect/model';
  import { summarizeGenes } from '../inspect/model';

  let {
    selectedIndex,
    inspection,
    message,
    onclose,
  }: {
    selectedIndex: number;
    inspection: Inspection | null;
    message: string | null;
    onclose: () => void;
  } = $props();

  const activationLevel = (value: number) => `${Math.min(1, Math.abs(value)) * 100}%`;
  let genomeJson = $derived(inspection ? JSON.stringify(inspection.genome, null, 2) : '');
  let geneSummary = $derived(inspection ? summarizeGenes(inspection.genome) : []);
</script>

<aside aria-label={`Inspector for agent ${selectedIndex}`}>
  <header>
    <strong>agent #{selectedIndex}</strong>
    <button onclick={onclose} aria-label="Close inspector">×</button>
  </header>

  {#if message}
    <p class="message">{message}</p>
  {:else if !inspection}
    <p class="message">reading brain…</p>
  {:else}
    <dl>
      <div><dt>tick</dt><dd>{inspection.tick}</dd></div>
      <div><dt>incarnation</dt><dd>{inspection.incarnation}</dd></div>
      <div><dt>energy</dt><dd>{inspection.energy.toFixed(2)}</dd></div>
      <div><dt>age</dt><dd>{inspection.age}</dd></div>
      <div><dt>species</dt><dd>{inspection.species_id}</dd></div>
      <div><dt>size</dt><dd>{inspection.size.toFixed(2)}</dd></div>
      <div><dt>brain units</dt><dd>{inspection.brain_units}</dd></div>
      <div><dt>sensor load</dt><dd>{inspection.sensor_load.toFixed(1)}</dd></div>
      <div><dt>parent A</dt><dd>{inspection.parent_a}</dd></div>
      <div><dt>parent B</dt><dd>{inspection.parent_b}</dd></div>
    </dl>

    <section>
      <h2>live activations</h2>
      <div class="neurons">
        {#each inspection.activations as activation, index}
          <div
            class:negative={activation < 0}
            class="neuron"
            style={`--level: ${activationLevel(activation)}`}
            title={`neuron ${index}: ${activation.toFixed(5)}`}
          >
            <span>{index}</span><output>{activation.toFixed(3)}</output>
          </div>
        {/each}
      </div>
    </section>

    <details>
      <summary>
        genome ({inspection.genome.length} genes;
        {#each geneSummary as entry, index}{index ? ', ' : ''}{entry.kind} {entry.count}{/each})
      </summary>
      <pre>{genomeJson}</pre>
    </details>
  {/if}
</aside>

<style>
  aside {
    position: absolute;
    inset: 0 0 0 auto;
    width: min(28rem, 92%);
    padding: 0.8rem;
    box-sizing: border-box;
    overflow: auto;
    border-left: 1px solid #2b2f38;
    background: rgb(14 16 20 / 96%);
    color: #d8dee9;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    position: sticky;
    top: -0.8rem;
    margin: -0.8rem -0.8rem 0.75rem;
    padding: 0.65rem 0.8rem;
    background: #14161a;
    border-bottom: 1px solid #2b2f38;
  }

  button {
    border: 0;
    background: transparent;
    color: #d8dee9;
    font: inherit;
    font-size: 1.2rem;
    cursor: pointer;
  }

  .message { color: #d08770; }

  dl {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.35rem 1rem;
    margin: 0 0 1rem;
  }

  dl div {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
  }

  dt { color: #6b7280; }
  dd { margin: 0; font-variant-numeric: tabular-nums; }

  h2 {
    margin: 0 0 0.45rem;
    color: #6b7280;
    font-size: 0.75rem;
    font-weight: 500;
    text-transform: uppercase;
  }

  .neurons {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(6.5rem, 1fr));
    gap: 0.25rem;
    margin-bottom: 1rem;
  }

  .neuron {
    display: flex;
    justify-content: space-between;
    padding: 0.2rem 0.3rem;
    border: 1px solid #2b2f38;
    background:
      linear-gradient(90deg, #5e81ac var(--level), transparent var(--level)),
      #1c1f26;
    font-size: 0.68rem;
    font-variant-numeric: tabular-nums;
  }

  .neuron.negative {
    background:
      linear-gradient(90deg, #bf616a var(--level), transparent var(--level)),
      #1c1f26;
  }

  output { color: #eceff4; }

  details {
    border-top: 1px solid #2b2f38;
    padding-top: 0.65rem;
  }

  summary {
    color: #88c0d0;
    cursor: pointer;
  }

  pre {
    overflow: auto;
    font: 0.68rem/1.35 ui-monospace, SFMono-Regular, Menlo, monospace;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
