/** On-demand live species sampling and world-local highlight selection. */
import { createRequestGate } from '../sim/request-gate.js';
import { decodeSpeciesSnapshot, isSpeciesSelection, NULL_SPECIES } from './model.ts';

export function createSpeciesController({
  getSim, onChange, now = () => performance.now(), intervalMs = 250,
}) {
  let open = false;
  let selected = null;
  let sample = null;
  let message = null;
  let wantedTick = null;
  const gate = createRequestGate({ intervalMs });
  const notify = () => onChange({ sample, selected, message });

  function request(timestamp = now()) {
    const sim = getSim();
    if (!sim || (!open && selected === null)) return;
    const requestId = gate.begin(timestamp);
    if (requestId !== null) sim.requestSpecies(requestId);
  }

  return {
    setOpen(value) {
      open = value;
      if (open) request();
    },
    select(id) {
      if (!isSpeciesSelection(id)) throw new TypeError('invalid species selection');
      selected = id === selected ? null : id;
      message = null;
      notify();
      if (!sample) request();
    },
    reset() {
      gate.invalidate({ resetCooldown: true });
      gate.demand(open);
      sample = null;
      selected = null;
      message = null;
      wantedTick = null;
      notify();
    },
    accept(response) {
      if (!gate.settle(response.requestId)) return false;
      try {
        if (response.diagnostics === null) throw new Error(response.message ?? 'species data unavailable');
        const next = decodeSpeciesSnapshot(response);
        if (sample && next.tick < sample.tick) throw new Error('species sample moved backwards');
        sample = next;
        gate.demand(wantedTick !== null && sample.tick < wantedTick);
        message = null;
        if (selected !== null && (selected === NULL_SPECIES
          ? sample.unclassifiedPopulation === 0
          : !sample.populations.some((row) => row.id === selected))) {
          message = selected === NULL_SPECIES ? 'No unclassified agents remain.'
            : `Species #${selected} is no longer active.`;
          selected = null;
        }
      } catch (error) {
        message = String(error);
      }
      notify();
      return true;
    },
    poll(frame, timestamp) {
      if (!open && selected === null) return;
      if (frame?.fresh) {
        wantedTick = frame.tick;
        if (!sample || frame.tick > sample.tick) gate.demand();
      }
      if (gate.due(timestamp)) request(timestamp);
    },
  };
}
