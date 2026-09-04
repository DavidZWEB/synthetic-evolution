<script>
  import { onMount } from 'svelte';
  import ControlBar from './ui/ControlBar.svelte';
  import InspectorPanel from './ui/InspectorPanel.svelte';
  import StatusBar from './ui/StatusBar.svelte';
  import TimeSeries from './ui/TimeSeries.svelte';
  import { createInspectorController } from './inspect/controller.js';
  import { createSim } from './sim/client.js';
  import { createRunValidation } from './sim/run-validation.js';
  import { readRunUrl, writeRunUrl } from './sim/seed-url.js';
  import { appendMetric, metricFromMessage } from './telemetry/history.ts';
  import { createRenderer } from './render/renderer.js';
  import { createPointerGestures } from './ui/pointer-gestures.js';

  const MAX_CHART_SAMPLES = 480;
  const DEFAULT_RUN = { seed: '42', founders: 2000, params: null };

  let canvas;
  let sim = null;
  /** Built from the world's own hints, so nothing here has a second copy of `world.size`. */
  let renderer = null;
  let capacity = 0;

  let tick = $state(0n);
  let population = $state(0);
  let meanEnergy = $state(0);
  let metricSamples = $state([]);
  let transport = $state(null);
  let fps = $state(0);
  let running = $state(false);
  let speed = $state(1);
  let founders = $state(2000);
  // Keep this as text until the worker parses it as u64. A JavaScript number silently
  // aliases distinct seeds above 2^53, which breaks seed-addressed reproducibility.
  let seed = $state('42');
  let failure = $state(null);
  let shareUrl = $state('');
  let linkCopied = $state(false);
  let selectedIndex = $state(null);
  let inspection = $state(null);
  let inspectionMessage = $state(null);
  let pointerCount = $state(0);
  /** Mirrors the renderer's camera for display. Written every frame, never read by it. */
  let zoom = $state(1);
  let latestFrame = null;
  let runParams = null;
  let validating = $state(false);

  const inspector = createInspectorController({
    getSim: () => sim,
    getRenderer: () => renderer,
    getFrame: () => latestFrame,
    onChange: (next) => {
      selectedIndex = next.selectedIndex;
      inspection = next.inspection;
      inspectionMessage = next.message;
    },
  });

  const gestures = createPointerGestures({
    getRenderer: () => renderer,
    onPointerCount: (count) => (pointerCount = count),
    onClick: inspector.pickAt,
  });

  function onWheel(event) {
    // The page must not scroll under the canvas, so this is not passive — hence the
    // explicit listener in `onMount` rather than an `onwheel` attribute, which Svelte
    // registers passively and where preventDefault would be ignored.
    event.preventDefault();
    // Exponential in the wheel delta, so a trackpad's many small events and a mouse
    // wheel's few large ones cover the same ground at the same speed.
    renderer?.zoomAt(event.clientX, event.clientY, Math.exp(-event.deltaY * 0.0015));
  }

  function resetView() {
    renderer?.fit();
  }

  function applySpeed(next) {
    speed = next;
    sim?.setSpeed(next);
  }

  function toggle() {
    if (!sim) return;
    if (running) sim.pause();
    else sim.play();
  }

  function reseed() {
    requestRun({ seed, founders, params: runParams });
  }

  function activateRun(next) {
    sim?.destroy();
    sim = null;
    validating = false;
    seed = next.seed;
    founders = next.founders;
    runParams = next.params;
    running = false;
    transport = null;
    capacity = 0;
    latestFrame = null;
    tick = 0n;
    population = 0;
    meanEnergy = 0;
    metricSamples = [];
    inspector.select(null);
    start();
  }

  function requestRun(next) {
    if (!sim || !transport) {
      activateRun(next);
      return;
    }
    runValidation.request(next);
  }

  const runValidation = createRunValidation({
    getSim: () => sim,
    onPendingChange: (pending) => (validating = pending),
    onAccepted: activateRun,
    onRejected: (error) => {
      failure = `url: ${error}`;
      if (shareUrl) globalThis.history.replaceState(null, '', shareUrl);
    },
  });

  function updateRunUrl() {
    linkCopied = false;
    shareUrl = writeRunUrl(globalThis.location.href, {
      seed,
      founders,
      params: runParams,
    });
    globalThis.history.replaceState(null, '', shareUrl);
  }

  function start() {
    failure = null;
    transport = null;
    const nextSim = createSim({ seed, founders, params: runParams });
    sim = nextSim;

    nextSim.on('ready', ({ transport: kind, hints, params }) => {
      if (sim !== nextSim) return;
      runParams = params;
      updateRunUrl();
      // Rebuilt rather than reused: a reseed can carry different params, and a renderer
      // holding the previous world's extent would draw a correct picture of the wrong one.
      // The *view* survives that rebuild, so reseeding does not yank you back out to the
      // whole world — watching one patch across several seeds is the point of the button.
      const carried =
        renderer && renderer.worldSize === hints.world_size ? renderer.view() : null;
      let nextRenderer;
      try {
        nextRenderer = createRenderer(canvas, {
          worldSize: hints.world_size,
          capacity: hints.agent_capacity,
          plantCapacity: hints.plant_capacity,
          plantRadius: hints.plant_radius,
          plantColor: hints.plant_signature,
          plantMaxEnergy: hints.plant_max_energy,
        });
      } catch (error) {
        failure = String(error);
        nextSim.destroy();
        sim = null;
        return;
      }
      renderer?.destroy();
      renderer = nextRenderer;
      // A world of a different size makes the old coordinates mean something else, so
      // that is the one case worth reframing for.
      if (carried) renderer.setView(carried);
      else renderer.fit();
      capacity = hints.agent_capacity;
      transport = kind;
      failure = null;
      nextSim.setSpeed(speed);
    });
    nextSim.on('status', ({ running: nextRunning }) => {
      if (sim === nextSim) running = nextRunning;
    });
    nextSim.on('params', ({ hints, params }) => {
      if (sim !== nextSim) return;
      runParams = params;
      updateRunUrl();
      renderer?.setRenderHints({
        plantRadius: hints.plant_radius,
        plantColor: hints.plant_signature,
        plantMaxEnergy: hints.plant_max_energy,
      });
    });
    nextSim.on('metrics', (message) => {
      if (sim !== nextSim) return;
      try {
        const sample = metricFromMessage(message);
        meanEnergy = sample.meanEnergy;
        metricSamples = appendMetric(metricSamples, sample, MAX_CHART_SAMPLES);
      } catch (error) {
        failure = `metrics: ${String(error)}`;
      }
    });
    nextSim.on('inspection', (message) => {
      if (sim === nextSim) inspector.accept(message);
    });
    nextSim.on('validatedRun', (message) => {
      if (sim === nextSim) runValidation.accept(message);
    });
    nextSim.on('error', (message) => {
      if (sim !== nextSim) return;
      failure = `${message.context}: ${message.message}`;
      if (message.fatal) {
        running = false;
        transport = null;
      }
    });
  }

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(shareUrl);
      linkCopied = true;
    } catch (error) {
      failure = `share: ${String(error)}`;
    }
  }

  onMount(() => {
    let validUrl = true;
    try {
      const shared = readRunUrl(globalThis.location.href, founders);
      const initial = shared ?? DEFAULT_RUN;
      seed = initial.seed;
      founders = initial.founders;
      runParams = initial.params;
    } catch (error) {
      runValidation.cancel();
      failure = `url: ${String(error)}`;
      validUrl = false;
    }
    if (validUrl) start();

    const onHashChange = () => {
      try {
        requestRun(readRunUrl(globalThis.location.href, DEFAULT_RUN.founders) ?? DEFAULT_RUN);
      } catch (error) {
        failure = `url: ${String(error)}`;
        if (shareUrl) globalThis.history.replaceState(null, '', shareUrl);
      }
    };
    globalThis.addEventListener('hashchange', onHashChange);

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
      latestFrame = frame;
      renderer?.draw(frame?.views ?? null, capacity, frame?.fresh ?? false);
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
      inspector.poll(frame, now);
      if (now - since >= 500) {
        fps = Math.round((frames * 1000) / (now - since));
        frames = 0;
        since = now;
      }
    };
    handle = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(handle);
      globalThis.removeEventListener('hashchange', onHashChange);
      canvas.removeEventListener('wheel', onWheel);
      sim?.destroy();
      renderer?.destroy();
    };
  });
</script>

<main>
  <StatusBar {tick} {population} {meanEnergy} {fps} {zoom} {transport} />

  <div class="stage">
    <canvas
      bind:this={canvas}
      class:dragging={pointerCount > 0}
      onpointerdown={gestures.down}
      onpointermove={gestures.move}
      onpointerup={gestures.up}
      onpointercancel={gestures.cancel}
      onclick={gestures.click}
      ondblclick={resetView}
    ></canvas>
    <TimeSeries samples={metricSamples} />
    {#if selectedIndex !== null}
      <InspectorPanel
        {selectedIndex}
        {inspection}
        message={inspectionMessage}
        onclose={() => inspector.select(null)}
      />
    {/if}
    {#if failure}
      <p class="failure">{failure}</p>
    {/if}
  </div>

  <ControlBar
    ready={Boolean(transport) && !validating}
    {running}
    {speed}
    {seed}
    {founders}
    {shareUrl}
    {linkCopied}
    ontoggle={toggle}
    onstep={() => sim?.stepOnce(1)}
    onspeed={applySpeed}
    onseed={(value) => (seed = value)}
    onfounders={(value) => (founders = value)}
    onreseed={reseed}
    oncopy={copyLink}
    onreset={resetView}
  />
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

</style>
