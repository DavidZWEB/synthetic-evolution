<script>
  import { onMount } from 'svelte';
  import { createSim } from './sim/client.js';
  import { createRenderer } from './render/renderer.js';

  const SPEEDS = [1, 2, 5, 10, 25, 50, 100];

  let canvas;
  let sim = null;
  /** Built from the world's own hints, so nothing here has a second copy of `world.size`. */
  let renderer = null;
  let capacity = 0;

  let tick = $state(0n);
  let population = $state(0);
  let transport = $state(null);
  let fps = $state(0);
  let running = $state(false);
  let speed = $state(1);
  let founders = $state(2000);
  let seed = $state(42);
  let failure = $state(null);
  /** Mirrors the renderer's camera for display. Written every frame, never read by it. */
  let zoom = $state(1);

  /**
   * Pointers currently down, by id. One is a drag, two are a pinch.
   *
   * Pointer events rather than mouse events so a finger and a mouse take the same path —
   * spec §7.7 expects people to open the link on a phone, and a viewer that can only be
   * driven with a wheel is one they cannot use at all.
   */
  const pointers = new Map();
  let pinchDistance = 0;

  const spread = () => {
    const [a, b] = [...pointers.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };
  const midpoint = () => {
    const [a, b] = [...pointers.values()];
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  };

  function onPointerDown(event) {
    canvas.setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 2) pinchDistance = spread();
  }

  function onPointerMove(event) {
    const previous = pointers.get(event.pointerId);
    if (!previous) return;
    const next = { x: event.clientX, y: event.clientY };

    if (pointers.size === 1) {
      renderer?.panBy(next.x - previous.x, next.y - previous.y);
    }
    pointers.set(event.pointerId, next);

    if (pointers.size === 2 && pinchDistance > 0) {
      const distance = spread();
      const centre = midpoint();
      renderer?.zoomAt(centre.x, centre.y, distance / pinchDistance);
      pinchDistance = distance;
    }
  }

  function onPointerUp(event) {
    pointers.delete(event.pointerId);
    if (pointers.size < 2) pinchDistance = 0;
  }

  function onWheel(event) {
    // The page must not scroll under the canvas, so this is not passive — hence the
    // explicit listener in `onMount` rather than an `onwheel` attribute, which Svelte
    // registers passively and where preventDefault would be ignored.
    event.preventDefault();
    // Scroll down to zoom in, up to zoom out — pushing the world away and pulling it
    // closer, rather than driving a magnification slider.
    //
    // Exponential in the delta, so a trackpad's many small events and a mouse wheel's few
    // large ones cover the same ground at the same speed.
    renderer?.zoomAt(event.clientX, event.clientY, Math.exp(event.deltaY * 0.0015));
  }

  function resetView() {
    renderer?.fit();
  }

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
    sim = createSim({ seed, founders });
    sim.on('ready', ({ transport: kind, hints }) => {
      transport = kind;
      capacity = hints.agent_capacity;
      // Rebuilt rather than reused: a reseed can carry different params, and a renderer
      // holding the previous world's extent would draw a correct picture of the wrong one.
      // The *view* survives that rebuild, so reseeding does not yank you back out to the
      // whole world — watching one patch across several seeds is the point of the button.
      const carried =
        renderer && renderer.worldSize === hints.world_size ? renderer.view() : null;
      renderer?.destroy();
      try {
        renderer = createRenderer(canvas, {
          worldSize: hints.world_size,
          capacity: hints.agent_capacity,
          plantCapacity: hints.plant_capacity,
          plantRadius: hints.plant_radius,
          plantColor: hints.plant_signature,
          plantMaxEnergy: hints.plant_max_energy,
        });
      } catch (error) {
        failure = String(error);
        return;
      }
      // A world of a different size makes the old coordinates mean something else, so
      // that is the one case worth reframing for.
      if (carried) renderer.setView(carried);
      else renderer.fit();
      sim.setSpeed(speed);
    });
    sim.on('error', (message) => {
      failure = `${message.context}: ${message.message}`;
    });
  }

  onMount(() => {
    start();

    // Registered here, not as an attribute: Svelte adds `onwheel` passively and a passive
    // listener cannot preventDefault, so the page would scroll while you zoomed.
    canvas.addEventListener('wheel', onWheel, { passive: false });

    // The renderer runs on its own clock and draws whatever the latest frame is. It never
    // waits for the sim and the sim never waits for it — which is what the snapshot being
    // a buffer rather than a callback buys (spec §2.1).
    let frames = 0;
    let since = performance.now();
    let handle = 0;

    const loop = () => {
      handle = requestAnimationFrame(loop);
      const frame = sim?.latest();
      renderer?.draw(frame?.views ?? null, capacity);
      // Read back rather than tracked alongside: the renderer owns the camera, and a
      // second copy here would go stale the moment anything but an input moved it — a
      // resize, a reseed, the zoom floor refusing a scroll.
      zoom = renderer?.zoom() ?? 1;
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
      canvas.removeEventListener('wheel', onWheel);
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
      <div><dt>zoom</dt><dd>{zoom < 10 ? zoom.toFixed(1) : Math.round(zoom)}×</dd></div>
      <div><dt>transport</dt><dd class:degraded={transport === 'transferable'}>{transport ?? '…'}</dd></div>
    </dl>
  </header>

  <div class="stage">
    <canvas
      bind:this={canvas}
      class:dragging={pointers.size > 0}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      ondblclick={resetView}
    ></canvas>
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
    <label>founders <input type="number" bind:value={founders} min="1" /></label>
    <button onclick={reseed}>reseed</button>
    <button onclick={resetView}>reset view</button>
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
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: grab;
    /* The browser's own pan and pinch would fight the camera for the same gestures. */
    touch-action: none;
  }
  canvas.dragging { cursor: grabbing; }

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
