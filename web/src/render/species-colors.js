/** Observation-only colors; neither signatures nor snapshot buffers are mutated. */
import { isSpeciesSelection, NULL_SPECIES } from '../species/model.ts';

export function validateSpeciesView({ colorMode, selectedSpecies }) {
  if (!['signature', 'species'].includes(colorMode) || !isSpeciesSelection(selectedSpecies)) {
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

export function speciesSwatch(id) {
  if (id === null || !isSpeciesSelection(id)) throw new TypeError('invalid species ID');
  const color = new Float32Array(3);
  writeColor(id, color, 0);
  return `rgb(${[...color].map((value) => Math.round(value * 255)).join(' ')})`;
}

export function displayColors(views, count, { colorMode, selectedSpecies }, out) {
  if (colorMode === 'signature' && selectedSpecies === null) return views.signature;
  for (let index = 0; index < count; index++) {
    const offset = index * 3;
    const id = views.species[index];
    if (colorMode === 'species') writeColor(id, out, offset);
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
