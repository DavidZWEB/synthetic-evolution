<script>
  let {
    captureNext, oncapture, captureRepresentatives, onrepresentatives,
    runs, activeId, captureStatus, message, busy,
    onclose, onrefresh, onsave, onload, onlineage, ondelete, onstop,
  } = $props();
</script>

<section aria-label="Species history" class="history-panel">
  <header>
    <strong>Species history</strong>
    <button onclick={onclose} aria-label="Close species history">close</button>
  </header>
  <label class="capture">
    <input type="checkbox" checked={captureNext}
      onchange={(event) => oncapture(event.currentTarget.checked)} />
    record the next new / reseeded run
  </label>
  <label class="capture">
    <input type="checkbox" checked={captureRepresentatives} disabled={!captureNext}
      onchange={(event) => onrepresentatives(event.currentTarget.checked)} />
    include representative genomes
  </label>
  <p class="help">
    Stores each new species' genome at its origin (about 25 KB each), so runs reach the
    10 MiB limit, and stop recording, much sooner. Staging holds 65,536 genes; any origin
    beyond that is saved as unavailable, never reconstructed later.
  </p>
  <p>Capture: <strong>{captureStatus}</strong>. Species origins and extinctions, plus genomes when included.</p>
  {#if captureStatus === 'recording'}
    <button onclick={onstop} disabled={busy}>stop recording</button>
  {/if}
  {#if message}<p class="notice" role="status">{message}</p>{/if}
  <div class="actions">
    <button onclick={onsave} disabled={busy}>save run</button>
    <label class="import">
      load run
      <input type="file" accept=".sevrun,application/octet-stream"
        disabled={busy}
        onchange={(event) => {
          const file = event.currentTarget.files?.[0];
          if (file) onload(file);
          event.currentTarget.value = '';
        }} />
    </label>
  </div>
  <p class="help">
    Save run downloads the world exactly as it is now, with whatever history it has. Load run
    restores one, paused, and keeps its history. History-only files are a command-line tool.
  </p>
  <p class="help">
    History is kept locally per run, not per seed. Reload restores these archives, not the
    simulation. Open archives may still be active in another tab.
  </p>
  <p class="help">
    Limits: 10 MiB / run, 50 MiB total, 20 runs (serialized data). No automatic deletion.
    Browser storage can be cleared or evicted: save important runs.
  </p>
  <div class="actions">
    <button onclick={onrefresh} disabled={busy}>refresh history</button>
  </div>
  {#if runs.length === 0}
    <p>No saved histories.</p>
  {/if}
  <ul>
    {#each runs as run (run.id)}
      <li>
        <strong>seed {run.seed}</strong>
        <span>{run.cohorts.join(' + ')}</span>
        <span class="identity" title={run.id}>archive {run.id}</span>
        <span>{run.captureEnd ?? (run.id === activeId && captureStatus === 'recording'
          ? 'recording in this tab' : 'unfinalized / possibly active')}</span>
        <span>{run.eventCount} events · {run.gapCount} gaps · {(run.bytes / 1024).toFixed(1)} KiB</span>
        <div class="actions">
          <button disabled={busy} onclick={() => onlineage(run.id)}
            aria-label={`lineage for archive ${run.id} (seed ${run.seed})`}>lineage</button>
          <button disabled={busy} onclick={() => ondelete(run.id)}
            aria-label={`delete archive ${run.id} (seed ${run.seed})`}>delete</button>
        </div>
      </li>
    {/each}
  </ul>
</section>

<style>
  .history-panel {
    position: absolute;
    z-index: 4;
    inset: 0.75rem 0.75rem 0.75rem auto;
    width: min(32rem, calc(100% - 1.5rem));
    box-sizing: border-box;
    overflow: auto;
    padding: 1rem;
    background: #14161af5;
    border: 1px solid #3b4252;
    border-radius: 4px;
    overflow-wrap: anywhere;
  }
  header { display: flex; justify-content: space-between; align-items: center; gap: 1rem; }
  .capture { display: flex; gap: 0.5rem; margin-top: 1rem; align-items: center; }
  .help { color: #a8b0bf; }
  .notice { color: #eacb8a; }
  ul { padding: 0; list-style: none; }
  li { display: grid; gap: 0.3rem; border-top: 1px solid #3b4252; padding: 0.8rem 0; }
  .identity { font-size: 0.8rem; color: #a8b0bf; }
  .actions { display: flex; flex-wrap: wrap; gap: 0.5rem; }
  button, .import {
    font: inherit;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #3b4252;
    border-radius: 3px;
    padding: 0.25rem 0.6rem;
    cursor: pointer;
  }
  button:disabled { opacity: 0.4; cursor: default; }
  .import { min-width: 0; max-width: 100%; }
  .import input { display: block; max-width: 100%; font: inherit; margin-top: 0.3rem; }
</style>
