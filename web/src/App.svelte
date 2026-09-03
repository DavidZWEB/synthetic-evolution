<script>
  import { onMount } from 'svelte';
  import { createSim } from './sim/client.js';
  import { createRenderer } from './render/renderer.js';

  /** Matches `WorldParams::size`. The renderer needs it to map world units to clip space. */
  const WORLD_SIZE = 1000;
  /** Matches `WorldParams::max_agents`, which is the slot count a frame carries. */
  const CAPACITY = 5000;

  const SPEEDS = [1, 2, 5, 10, 25, 50, 100];

  let canvas;
  let sim = null;
  let renderer = null;

  let tick = $state(0n);
  let population = $state(0);
  let transport = $state(null);
  let fps = $state(0);
  let running = $state(false);
  let speed = $state(1);
  let founders = $state(2000);
  let seed = $state(42);
  let failure = $state(null);

  function applySpeed(next) {
    speed = next;
    sim?.setSpeed(next);
  }

  function toggle() {
    running = !running;
    if (running) sim.play();
    else sim.pause();
  }

  function reseed() {
    sim?.destroy();
    running = false;
    start();
  }

  function start() {
    sim = createSim({ seed, founders, capacity: CAPACITY });
    sim.on('ready', (info) => {
      transport = info.transport;
      sim.setSpeed(speed);
    });
    sim.on('error', (message) => {
      failure = `${message.context}: ${message.message}`;
    });
  }

  onMount(() => {
    try {
      renderer = createRenderer(canvas, { worldSize: WORLD_SIZE, capacity: CAPACITY });
    } catch (error) {
      failure = String(error);
      return;
    }
    start();

    // The renderer runs on its own clock and draws whatever the latest frame is. It never
    // waits for the sim and the sim never waits for it — which is what the snapshot being
    // a buffer rather than a callback buys (spec §2.1).
    let frames = 0;
    let since = performance.now();
    let handle = 0;

    const loop = () => {
      handle = requestAnimationFrame(loop);
      const frame = sim?.latest();
      renderer.draw(frame?.views ?? null, CAPACITY);
      if (frame) {
        tick = frame.tick;
        population = frame.population;
      }

      frames += 1;
      const now = performance.now();
      if (now - since >= 500) {
        fps = Math.round((frames * 1000) / (now - since));
        frames = 0;
        since = now;
      }
    };
    handle = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(handle);
      sim?.destroy();
      renderer?.destroy();
    };
  });
</script>

<main>
  <header>
    <h1>Synthetic Evolution</h1>
    <dl>
      <div><dt>tick</dt><dd>{tick}</dd></div>
      <div><dt>agents</dt><dd>{population}</dd></div>
      <div><dt>fps</dt><dd class:slow={fps > 0 && fps < 55}>{fps}</dd></div>
      <div><dt>transport</dt><dd class:degraded={transport === 'transferable'}>{transport ?? '…'}</dd></div>
    </dl>
  </header>

  <div class="stage">
    <canvas bind:this={canvas}></canvas>
    {#if failure}
      <p class="failure">{failure}</p>
    {/if}
  </div>

  <footer>
    <button onclick={toggle} disabled={!transport}>{running ? 'pause' : 'play'}</button>
    <button onclick={() => sim?.stepOnce(1)} disabled={!transport || running}>step</button>

    <span class="speeds">
      {#each SPEEDS as option}
        <button class:active={speed === option} onclick={() => applySpeed(option)}>
          {option}×
        </button>
      {/each}
    </span>

    <label>seed <input type="number" bind:value={seed} min="0" /></label>
    <label>founders <input type="number" bind:value={founders} min="1" max={CAPACITY} /></label>
    <button onclick={reseed}>reseed</button>
  </footer>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #0e1014;
  }

  main {
    font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
    color: #d8dee9;
    height: 100vh;
    display: grid;
    grid-template-rows: auto 1fr auto;
  }

  header,
  footer {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.6rem 1rem;
    background: #14161a;
    border-color: #23262d;
    border-style: solid;
    border-width: 0;
  }
  header { border-bottom-width: 1px; }
  footer { border-top-width: 1px; flex-wrap: wrap; }

  h1 {
    font-size: 0.95rem;
    font-weight: 600;
    margin: 0;
    letter-spacing: 0.02em;
  }

  dl {
    display: flex;
    gap: 1.25rem;
    margin: 0 0 0 auto;
  }
  dl div { display: flex; gap: 0.4rem; }
  dt { color: #6b7280; }
  dd { margin: 0; color: #a3be8c; font-variant-numeric: tabular-nums; }
  dd.slow { color: #d08770; }
  dd.degraded { color: #d08770; }

  .stage { position: relative; min-height: 0; }
  canvas { display: block; width: 100%; height: 100%; }

  .failure {
    position: absolute;
    inset: 1rem;
    margin: 0;
    color: #bf616a;
  }

  button {
    font: inherit;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #2b2f38;
    border-radius: 3px;
    padding: 0.25rem 0.6rem;
    cursor: pointer;
  }
  button:hover:not(:disabled) { background: #242832; }
  button:disabled { opacity: 0.4; cursor: default; }
  button.active { background: #3b4252; border-color: #4c566a; }

  .speeds { display: flex; gap: 0.25rem; }

  label { color: #6b7280; display: flex; gap: 0.35rem; align-items: center; }
  input {
    font: inherit;
    width: 5rem;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #2b2f38;
    border-radius: 3px;
    padding: 0.2rem 0.4rem;
  }
</style>
