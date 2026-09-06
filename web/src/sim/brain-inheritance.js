/**
 * Run-level neural heredity modes.
 *
 * This is construction-time experiment configuration rather than a SimParams field:
 * changing it replaces the world instead of retuning one that is already running.
 */

export const EVOLVING = 'evolving';
export const RANDOMIZED_AT_BIRTH = 'randomized_at_birth';

export function parseBrainInheritance(value = EVOLVING) {
  if (value !== EVOLVING && value !== RANDOMIZED_AT_BIRTH) {
    throw new TypeError('inheritance must be evolving or randomized_at_birth');
  }
  return value;
}
