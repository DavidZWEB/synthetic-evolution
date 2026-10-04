/**
 * Run-level neural heredity modes.
 *
 * This is construction-time experiment configuration rather than a SimParams field:
 * changing it replaces the world instead of retuning one that is already running.
 */

export const EVOLVING = 'evolving';
export const RANDOMIZED_AT_BIRTH = 'randomized_at_birth';
export const STRUCTURAL_NULL = 'structural_null';

const MODES = [EVOLVING, RANDOMIZED_AT_BIRTH, STRUCTURAL_NULL];

export function parseBrainInheritance(value = EVOLVING) {
  if (!MODES.includes(value)) {
    throw new TypeError('inheritance must be evolving, randomized_at_birth, or structural_null');
  }
  return value;
}

/**
 * Whether a run in this mode can record species history or be saved. History archives
 * and saved runs carry the evolving world or the scalar control, so the structural null
 * is watch-only here, as it is metrics-only in the native shell (spec §7.8).
 */
export function keepsHistory(mode) {
  return parseBrainInheritance(mode) !== STRUCTURAL_NULL;
}
