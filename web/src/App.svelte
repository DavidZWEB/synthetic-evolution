<script>
  import { onMount } from 'svelte';
  import ControlBar from './ui/ControlBar.svelte';
  import FailureBanner from './ui/FailureBanner.svelte';
  import InspectorPanel from './ui/InspectorPanel.svelte';
  import HistoryPanel from './ui/HistoryPanel.svelte';
  import { createHistorySession } from './history/controller.js';
  import { parseArchive } from './history/archive.js';
  import { DEFAULT_LIMITS, openHistoryStore } from './history/store.js';
  import StatusBar from './ui/StatusBar.svelte';
  import TimeSeries from './ui/TimeSeries.svelte';
  import { createInspectorController } from './inspect/controller.js';
  import { EVOLVING } from './sim/brain-inheritance.js';
  import { createSim } from './sim/client.js';
  import { createRunValidation } from './sim/run-validation.js';
  import { readRunUrl, writeRunUrl } from './sim/seed-url.js';
  import { appendMetric, metricFromMessage } from './telemetry/history.ts';
  import { createRenderer } from './render/renderer.js';
  import { createPointerGestures } from './ui/pointer-gestures.js';
  import { wheelZoomFactor } from './ui/wheel.js';

  const MAX_CHART_SAMPLES = 480;
  const DEFAULT_RUN = {
    seed: '42',
    founders: 2000,
    params: null,
    brainInheritance: EVOLVING,
  };

  let canvas;
  let sim = null;
  /** Built from the world's own hints, so nothing here has a second copy of `world.size`. */
  let renderer = null;
  let capacity = 0;

  let tick = $state(0n);
  let population = $state(0);
  let descendants = $state(0);
  let speciesCount = $state(0);
  let unclassifiedPopulation = $state(0);
  let meanEnergy = $state(0);
  let metricSamples = $state([]);
  let transport = $state(null);
  let fps = $state(0);
  let running = $state(false);
  let speed = $state(1);
  let founders = $state(2000);
  let brainInheritance = $state(EVOLVING);
  // Keep this as text until the worker parses it as u64. A JavaScript number silently
  // aliases distinct seeds above 2^53, which breaks seed-addressed reproducibility.
  let seed = $state('42');
  let failure = $state(null);
  let rendererFailure = $state(null);
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
  let activeRun = null;
  let runSource = 'create';
  let validating = $state(false);
  let transitioning = $state(false);
  let showHistory = $state(false);
  let captureNext = $state(false);
  let captureStatus = $state('off');
  let historyMessage = $state(null);
  let historyRuns = $state([]);
  let historyBusy = $state(false);
  let activeHistoryId = $state(null);
  let historySession = null;
  let historyStore = null;
  let disposed = false;

  function getHistoryStore() {
    historyStore ??= openHistoryStore().catch((error) => {
      historyStore = null;
      throw error;
    });
    return historyStore;
  }

  async function refreshHistory() {
    historyRuns = await (await getHistoryStore()).list();
  }

  async function historyAction(action) {
    if (historyBusy) return;
    historyBusy = true;
    historyMessage = null;
    try {
      await action();
      await refreshHistory();
    } catch (error) {
      historyMessage = String(error);
    } finally {
      historyBusy = false;
    }
  }

  function downloadHistory(text, id) {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/x-ndjson' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = `species-history-${id}.jsonl`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function exportHistory(id) {
    return historyAction(async () => {
      const text = historySession?.active && historySession.id === id
        ? await historySession.boundary('snapshot')
        : await (await getHistoryStore()).exportArchive(id);
      downloadHistory(text, id);
    });
  }

  function deleteHistory(id) {
    return historyAction(async () => {
      if (historySession?.active && historySession.id === id) {
        await historySession.boundary('stopped');
      }
      await (await getHistoryStore()).delete(id);
    });
  }

  function importHistory(file) {
    return historyAction(async () => {
      if (file.size > DEFAULT_LIMITS.perRunBytes) {
        throw new Error('History file exceeds the 10 MiB per-run limit');
      }
      const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })
        .decode(await file.arrayBuffer());
      const { default: init, validate_params } = await import('./wasm/wasm.js');
      await init();
      const archive = await parseArchive(text, (params) =>
        JSON.parse(validate_params(JSON.stringify(params))));
      await (await getHistoryStore()).importArchive(archive);
      historyMessage = 'History imported. No simulation was started or resumed.';
    });
  }

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
    renderer?.zoomAt(event.clientX, event.clientY, wheelZoomFactor(event));
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
    requestRun({ seed, founders, params: runParams, brainInheritance }, 'reseed');
  }

  async function activateRun(next) {
    if (transitioning) return;
    transitioning = true;
    const oldSession = historySession;
    try {
      if (oldSession?.active) {
        await oldSession.waitForBoundary();
        if (oldSession.active) await oldSession.boundary('reseeded');
      }
    } catch (error) {
      historyMessage = `Previous history remains incomplete: ${String(error)}`;
    }
    oldSession?.detach();
    historySession = null;
    if (disposed) return;
    sim?.destroy();
    sim = null;
    validating = false;
    runSource = next.source ?? 'create';
    seed = next.seed;
    founders = next.founders;
    brainInheritance = next.brainInheritance;
    runParams = next.params;
    activeRun = null;
    running = false;
    transport = null;
    capacity = 0;
    latestFrame = null;
    tick = 0n;
    population = 0;
    descendants = 0;
    speciesCount = 0;
    unclassifiedPopulation = 0;
    meanEnergy = 0;
    metricSamples = [];
    inspector.select(null);
    start();
    transitioning = false;
  }

  function requestRun(next, source) {
    if (transitioning) return;
    if (!sim) {
      activateRun({ ...next, source });
      return;
    }
    runValidation.request({ ...next, source });
  }

  const runValidation = createRunValidation({
    getSim: () => sim,
    onPendingChange: (pending) => (validating = pending),
    onAccepted: activateRun,
    onRejected: (error, request) => {
      failure = `${request.source}: ${error}`;
      if (request.source === 'url' && shareUrl) {
        globalThis.history.replaceState(null, '', shareUrl);
      }
    },
  });

  function updateRunUrl(run) {
    try {
      linkCopied = false;
      shareUrl = writeRunUrl(globalThis.location.href, run);
      globalThis.history.replaceState(null, '', shareUrl);
      return null;
    } catch (error) {
      return `share: ${String(error)}`;
    }
  }

  function start() {
    failure = null;
    transport = null;
    const startingSource = runSource;
    const previousShareUrl = shareUrl;
    const historyRunId = captureNext ? crypto.randomUUID() : null;
    const nextSim = createSim({ seed, founders, params: runParams, brainInheritance, historyRunId });
    sim = nextSim;
    activeHistoryId = null;
    captureStatus = historyRunId ? 'starting' : 'off';
    if (historyRunId) {
      const nextHistorySession = createHistorySession({
        sim: nextSim,
        getStore: getHistoryStore,
        onChange: ({ id, status, message }) => {
          // Queued aborts outlive the worker, but must not update a replacement session.
          if (disposed || historySession !== nextHistorySession) return;
          activeHistoryId = id;
          captureStatus = status;
          if (message) historyMessage = message;
        },
        onSaved: refreshHistory,
      });
      historySession = nextHistorySession;
    }

    nextSim.on('ready', ({ transport: kind, hints, run }) => {
      if (sim !== nextSim) return;
      activeRun = run;
      runParams = run.params;
      const runUrlError = updateRunUrl(run);
      // Rebuilt rather than reused: a reseed can carry different params, and a renderer
      // holding the previous world's extent would draw a correct picture of the wrong one.
      // The *view* survives that rebuild, so reseeding does not yank you back out to the
      // whole world — watching one patch across several seeds is the point of the button.
      const carried =
        renderer && renderer.worldSize === hints.world_size ? renderer.view() : null;
      let nextRenderer;
      rendererFailure = null;
      try {
        nextRenderer = createRenderer(canvas, {
          worldSize: hints.world_size,
          capacity: hints.agent_capacity,
          plantCapacity: hints.plant_capacity,
          plantRadius: hints.plant_radius,
          plantColor: hints.plant_signature,
          plantMaxEnergy: hints.plant_max_energy,
          onContextLost: () => {
            rendererFailure = 'renderer: WebGL context lost; restoring…';
          },
          onContextRestored: () => {
            rendererFailure = null;
          },
          onContextError: (error) => {
            rendererFailure = `renderer: ${String(error)}`;
          },
        });
      } catch (error) {
        failure = String(error);
        historySession?.abort(`renderer initialization failed: ${String(error)}`);
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
      if (runUrlError) failure = runUrlError;
      nextSim.setSpeed(speed);
    });
    nextSim.on('status', ({ running: nextRunning }) => {
      if (sim === nextSim) running = nextRunning;
    });
    nextSim.on('params', ({ hints, params }) => {
      if (sim !== nextSim) return;
      runParams = params;
      activeRun = { ...activeRun, params };
      const runUrlError = updateRunUrl(activeRun);
      if (runUrlError) failure = runUrlError;
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
        descendants = sample.descendants;
        speciesCount = sample.speciesCount;
        unclassifiedPopulation = sample.unclassifiedPopulation;
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
      const context = message.context === 'create' ? startingSource : message.context;
      failure = `${context}: ${message.message}`;
      if (message.fatal) {
        historySession?.abort(failure);
        running = false;
        transport = null;
        runValidation.cancel();
        nextSim.destroy();
        sim = null;
        activeRun = null;
        latestFrame = null;
        inspector.select(null);
        if (message.context === 'create' && startingSource === 'url' && previousShareUrl) {
          shareUrl = previousShareUrl;
          globalThis.history.replaceState(null, '', previousShareUrl);
        }
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
    void historyAction(refreshHistory);
    let validUrl = true;
    try {
      const shared = readRunUrl(globalThis.location.href, founders);
      const initial = shared ?? DEFAULT_RUN;
      seed = initial.seed;
      founders = initial.founders;
      runParams = initial.params;
      brainInheritance = initial.brainInheritance;
      runSource = shared ? 'url' : 'create';
    } catch (error) {
      runValidation.cancel();
      failure = `url: ${String(error)}`;
      validUrl = false;
    }
    if (validUrl) start();

    const onHashChange = () => {
      try {
        requestRun(
          readRunUrl(globalThis.location.href, DEFAULT_RUN.founders) ?? DEFAULT_RUN,
          'url',
        );
      } catch (error) {
        runValidation.cancel();
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
      disposed = true;
      cancelAnimationFrame(handle);
      globalThis.removeEventListener('hashchange', onHashChange);
      canvas.removeEventListener('wheel', onWheel);
      historySession?.detach();
      sim?.destroy();
      renderer?.destroy();
    };
  });
</script>

<main>
  <StatusBar {tick} {population} {descendants} {speciesCount} {unclassifiedPopulation} {meanEnergy} {fps} {zoom} {transport} />

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
    {#if failure || rendererFailure}
      <FailureBanner
        message={failure ?? rendererFailure}
        dismissible={Boolean(failure)}
        ondismiss={() => (failure = null)}
      />
    {/if}
    {#if showHistory}
      <HistoryPanel
        {captureNext}
        oncapture={(value) => (captureNext = value)}
        runs={historyRuns}
        activeId={activeHistoryId}
        {captureStatus}
        message={historyMessage}
        busy={historyBusy || transitioning}
        onclose={() => (showHistory = false)}
        onrefresh={() => historyAction(refreshHistory)}
        onimport={importHistory}
        onexport={exportHistory}
        ondelete={deleteHistory}
        onstop={() => historyAction(() => historySession?.boundary('stopped'))}
      />
    {/if}
  </div>

  <ControlBar
    ready={Boolean(transport) && !validating && !transitioning}
    {running}
    {speed}
    {seed}
    {founders}
    {brainInheritance}
    {shareUrl}
    {linkCopied}
    ontoggle={toggle}
    onstep={() => sim?.stepOnce(1)}
    onspeed={applySpeed}
    onseed={(value) => (seed = value)}
    onfounders={(value) => (founders = value)}
    onbraininheritance={(value) => (brainInheritance = value)}
    onreseed={reseed}
    oncopy={copyLink}
    onreset={resetView}
    onhistory={() => {
      showHistory = !showHistory;
      if (showHistory) void historyAction(refreshHistory);
    }}
    {captureStatus}
    {transitioning}
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
    height: 100dvh;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr) auto;
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

</style>
