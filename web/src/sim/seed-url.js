/**
 * Shareable run identity in the URL fragment.
 *
 * The fragment never reaches the static host, and seeds remain decimal strings so
 * values above JavaScript's safe-integer ceiling keep their exact u64 identity.
 */

import { parseSeed } from './inputs.js';

function positiveInteger(value, name) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new TypeError(`${name} must be a positive integer`);
  }
  return parsed;
}

function paramsJson(value) {
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string') throw new TypeError('params must be JSON text');
  const parsed = JSON.parse(value);
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new TypeError('params must encode an object');
  }
  return JSON.stringify(parsed);
}

export function readRunUrl(href, defaultFounders) {
  const url = new URL(href);
  const fragment = new URLSearchParams(url.hash.slice(1));
  if (!fragment.has('seed')) return null;

  const seed = fragment.get('seed');
  parseSeed(seed);
  const founders = fragment.has('founders')
    ? positiveInteger(fragment.get('founders'), 'founders')
    : defaultFounders;
  const params = fragment.has('params') ? paramsJson(fragment.get('params')) : null;
  return { seed, founders, params };
}

export function writeRunUrl(href, { seed, founders, params = null }) {
  parseSeed(seed);
  positiveInteger(founders, 'founders');
  const url = new URL(href);
  const fragment = new URLSearchParams({ seed, founders: String(founders) });
  const encodedParams = paramsJson(params);
  if (encodedParams) fragment.set('params', encodedParams);
  url.hash = fragment.toString();
  return url.toString();
}
