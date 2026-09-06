/** Bounded live metrics retained by the browser chart. */

export interface MetricSample {
  tick: bigint;
  population: number;
  descendants: number;
  meanEnergy: number;
}

interface MetricMessage {
  tick: string;
  population: number;
  descendants: number;
  meanEnergy: number;
}

export function metricFromMessage(message: MetricMessage): MetricSample {
  const tick = BigInt(message.tick);
  if (
    tick < 0n ||
    !Number.isSafeInteger(message.population) ||
    message.population < 0 ||
    !Number.isSafeInteger(message.descendants) ||
    message.descendants < 0 ||
    message.descendants > message.population ||
    !Number.isFinite(message.meanEnergy) ||
    message.meanEnergy < 0
  ) {
    throw new TypeError('invalid metrics payload');
  }
  return {
    tick,
    population: message.population,
    descendants: message.descendants,
    meanEnergy: message.meanEnergy,
  };
}

export function appendMetric(
  samples: MetricSample[],
  sample: MetricSample,
  limit: number,
): MetricSample[] {
  if (!Number.isSafeInteger(limit) || limit < 1) {
    throw new RangeError('metric history limit must be a positive integer');
  }
  const next =
    samples.at(-1)?.tick === sample.tick ? [...samples.slice(0, -1), sample] : [...samples, sample];
  return next.length > limit ? next.slice(next.length - limit) : next;
}
