/**
 * Time-sliced worker scheduling for a fixed-timestep simulation.
 *
 * One callback performs at most one bounded `step` call and always yields before doing
 * more work. Requested speed is therefore best-effort when the simulation is slower
 * than real time, but pause and command messages cannot sit behind an unbounded catch-up
 * batch.
 */

const TARGET_SLICE_MS = 8;
const MAX_BATCH_TICKS = 64;
const MAX_QUEUED_BATCHES = 2;
const MAX_ELAPSED_MS = 250;
const IDLE_POLL_MS = 1000 / 60;
const MIN_TICK_SAMPLE_MS = 0.01;

export function createTickScheduler({
  step,
  publish,
  onError,
  onRunningChange,
  now = () => performance.now(),
  schedule = (callback, delay) => setTimeout(callback, delay),
  cancel = (handle) => clearTimeout(handle),
}) {
  let running = false;
  let speed = 1;
  let secondsPerTick = 1 / 60;
  let timer = null;
  let lastSliceAt = 0;
  let debt = 0;
  // Start conservatively: the first slice measures one tick before batching.
  let estimatedTickMs = TARGET_SLICE_MS;

  const maxBatch = () =>
    Math.min(MAX_BATCH_TICKS, Math.max(1, Math.floor(TARGET_SLICE_MS / estimatedTickMs)));

  function capDebt() {
    debt = Math.min(debt, maxBatch() * MAX_QUEUED_BATCHES);
  }

  function accrue(timestamp) {
    const elapsed = Math.max(0, Math.min(timestamp - lastSliceAt, MAX_ELAPSED_MS));
    lastSliceAt = timestamp;
    debt += (elapsed / 1000) * (speed / secondsPerTick);
    capDebt();
  }

  function delayUntilWork() {
    if (speed === 0) return IDLE_POLL_MS;
    return Math.min(
      IDLE_POLL_MS,
      Math.max(0, ((1 - debt) * secondsPerTick * 1000) / speed),
    );
  }

  function queue(delay) {
    timer = schedule(runSlice, delay);
  }

  function fail(error) {
    running = false;
    timer = null;
    debt = 0;
    onRunningChange(false);
    onError(error);
  }

  function runSlice() {
    timer = null;
    if (!running) return;

    accrue(now());
    const ticks = Math.min(Math.floor(debt), maxBatch());
    if (ticks === 0) {
      queue(delayUntilWork());
      return;
    }

    const startedAt = now();
    try {
      step(ticks);
      publish();
    } catch (error) {
      fail(error);
      return;
    }

    const sample = Math.max(MIN_TICK_SAMPLE_MS, (now() - startedAt) / ticks);
    // React immediately when ticks get slower so the next batch cannot compound a
    // regression. Speed up cautiously because populations grow over time.
    estimatedTickMs =
      sample > estimatedTickMs ? sample : estimatedTickMs * 0.75 + sample * 0.25;
    debt = Math.max(0, debt - ticks);
    capDebt();

    // Yield even while behind. Worker messages queued during `step` run before this
    // timer, which bounds control latency to one measured batch.
    queue(debt >= 1 ? 0 : delayUntilWork());
  }

  return {
    start() {
      if (running) return;
      running = true;
      debt = 0;
      lastSliceAt = now();
      onRunningChange(true);
      queue(0);
    },

    stop() {
      if (!running) return;
      running = false;
      debt = 0;
      if (timer !== null) cancel(timer);
      timer = null;
      onRunningChange(false);
    },

    setSpeed(value) {
      if (!Number.isFinite(value) || value < 0) {
        throw new TypeError('speed must be a finite non-negative number');
      }
      speed = value;
      capDebt();
    },

    setSecondsPerTick(value) {
      if (!Number.isFinite(value) || value <= 0) {
        throw new TypeError('seconds per tick must be a finite positive number');
      }
      secondsPerTick = value;
      capDebt();
    },
  };
}
