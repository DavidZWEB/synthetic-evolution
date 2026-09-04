/**
 * Validation for numeric values entering the worker from UI messages.
 *
 * Browser form constraints are presentation only; these guards preserve exact seeds
 * and keep JavaScript-to-WASM integer coercion from turning negatives into huge u32s.
 */

const MAX_U64 = (1n << 64n) - 1n;

export function parseSeed(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)$/.test(value)) {
    throw new TypeError('seed must be an unsigned decimal integer');
  }
  const parsed = BigInt(value);
  if (parsed > MAX_U64) throw new RangeError('seed exceeds the u64 range');
  return parsed;
}

export function founderCount(value, capacity) {
  if (!Number.isSafeInteger(value) || value < 1 || value > capacity) {
    throw new RangeError(`founders must be an integer between 1 and ${capacity}`);
  }
  return value;
}
