/**
 * Which plants just reseeded, for the renderer to highlight.
 *
 * A plant slot moves only when its plant dies of starvation and reseeds elsewhere
 * (spec §5.1), so a slot whose position changed between two snapshots has just
 * reseeded. The glow fades over wall-clock time rather than ticks, so a reseed stays
 * visible at any sim speed. Display only: nothing here reaches the simulation.
 */

/** How long a reseeded plant stays highlighted, in milliseconds. */
export const GLOW_MS = 2500;

export function createReseedGlow(capacity, fadeMs = GLOW_MS) {
  const previous = new Float32Array(capacity * 3);
  const movedAt = new Float64Array(capacity).fill(-Infinity);
  const glow = new Float32Array(capacity);
  let primed = false;

  return {
    /**
     * Notes which slots moved since the last snapshot. The first snapshot only sets
     * the baseline: every plant "appearing" in a new world is not a reseed.
     */
    observe(positions, now) {
      if (primed) {
        for (let i = 0; i < capacity; i += 1) {
          const at = i * 3;
          if (positions[at] !== previous[at] || positions[at + 1] !== previous[at + 1]) {
            movedAt[i] = now;
          }
        }
      }
      previous.set(positions.subarray(0, capacity * 3));
      primed = true;
    },

    /** Per-slot glow at `now`: 1 the moment a plant reseeds, fading to 0. */
    values(now) {
      for (let i = 0; i < capacity; i += 1) {
        glow[i] = Math.max(0, 1 - (now - movedAt[i]) / fadeMs);
      }
      return glow;
    },
  };
}
