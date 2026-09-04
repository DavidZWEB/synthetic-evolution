/**
 * Coalesces snapshot publication when the transferable transport is out of buffers.
 *
 * A dropped intermediate frame is fine while the simulation is running, but the newest
 * state must be retried when a buffer returns so pause and manual stepping cannot leave
 * the renderer permanently behind the metrics.
 */

export function createSnapshotPublisher({
  writer,
  source,
  tick,
  population,
  meanEnergy,
  send,
  now = () => performance.now(),
  metricsIntervalMs = 250,
}) {
  let pendingSnapshot = false;
  let pendingForcedMetrics = false;
  let lastMetricsAt = Number.NEGATIVE_INFINITY;

  function publish(forceMetrics = false) {
    const currentTick = tick();
    const currentPopulation = population();
    const publication = writer.publish(source(), currentTick, currentPopulation);
    if (publication === false) {
      pendingSnapshot = true;
      pendingForcedMetrics ||= forceMetrics;
      return false;
    }

    pendingSnapshot = false;
    if (publication !== true) send(publication, [publication.buffer]);

    const sampledAt = now();
    if (
      forceMetrics ||
      pendingForcedMetrics ||
      sampledAt - lastMetricsAt >= metricsIntervalMs
    ) {
      pendingForcedMetrics = false;
      lastMetricsAt = sampledAt;
      send({
        kind: 'metrics',
        tick: currentTick.toString(),
        population: currentPopulation,
        meanEnergy: meanEnergy(),
      });
    }
    return true;
  }

  return {
    publish,

    recycle(buffer) {
      writer.recycle?.(buffer);
      if (pendingSnapshot) publish(pendingForcedMetrics);
    },
  };
}
