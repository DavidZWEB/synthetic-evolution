/** Observation-only colors; neither signatures nor snapshot buffers are mutated. */
import { UNFED } from '../diet/controller.js';
import { isSpeciesSelection, NULL_SPECIES } from '../species/model.ts';

export function validateSpeciesView({ colorMode, selectedSpecies }) {
  if (
    !['signature', 'species', 'diet'].includes(colorMode) || !isSpeciesSelection(selectedSpecies)
  ) {
    throw new TypeError('invalid species display settings');
  }
}

function writeColor(id, out, offset) {
  if (id === NULL_SPECIES) {
    out[offset] = out[offset + 1] = out[offset + 2] = 0.55;
    return;
  }
  const hue = ((Math.imul(id, 0x9e3779b1) >>> 0) / 0x1_0000_0000) * 6;
  const x = 0.65 * (1 - Math.abs(hue % 2 - 1));
  const sector = Math.floor(hue);
  out[offset] = 0.3 + (sector === 0 || sector === 5 ? 0.65 : sector === 1 || sector === 4 ? x : 0);
  out[offset + 1] = 0.3 + (sector === 1 || sector === 2 ? 0.65 : sector === 0 || sector === 3 ? x : 0);
  out[offset + 2] = 0.3 + (sector === 3 || sector === 4 ? 0.65 : sector === 2 || sector === 5 ? x : 0);
}

// A diet runs from green (all plants) to red (all meat); the unfed, and slots the
// latest sample does not cover, stay grey.
const PLANTS = [0.3, 0.85, 0.35];
const MEAT = [0.95, 0.2, 0.15];

function writeDiet(diets, views, index, out, offset) {
  const share = diets && diets.incarnation[index] === views.incarnation[index]
    ? diets.shares[index]
    : UNFED;
  if (share === UNFED) {
    out[offset] = out[offset + 1] = out[offset + 2] = 0.55;
    return;
  }
  const meat = share / 254;
  for (let channel = 0; channel < 3; channel++) {
    out[offset + channel] = PLANTS[channel] + (MEAT[channel] - PLANTS[channel]) * meat;
  }
}

export function speciesSwatch(id) {
  if (id === null || !isSpeciesSelection(id)) throw new TypeError('invalid species ID');
  const color = new Float32Array(3);
  writeColor(id, color, 0);
  return `rgb(${[...color].map((value) => Math.round(value * 255)).join(' ')})`;
}

/**
 * Each slot's displayed colour. `diets` is the latest diet sample, one byte per slot
 * with the incarnations it was taken at, for the diet mode.
 */
export function displayColors(views, count, { colorMode, selectedSpecies }, out, diets = null) {
  if (colorMode === 'signature' && selectedSpecies === null) return views.signature;
  for (let index = 0; index < count; index++) {
    const offset = index * 3;
    const id = views.species[index];
    if (colorMode === 'species') writeColor(id, out, offset);
    else if (colorMode === 'diet') writeDiet(diets, views, index, out, offset);
    else {
      out[offset] = views.signature[offset];
      out[offset + 1] = views.signature[offset + 1];
      out[offset + 2] = views.signature[offset + 2];
    }
    if (selectedSpecies !== null && id !== selectedSpecies) {
      out[offset] *= 0.35;
      out[offset + 1] *= 0.35;
      out[offset + 2] *= 0.35;
    }
  }
  return out;
}
