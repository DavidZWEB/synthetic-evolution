/** Exact UTF-8 JSONL framing, including duplicate-key rejection before wire validation. */
import { integerParam, requireThat } from './shape.js';

export const MAX_LINE_BYTES = 1024 * 1024;
const encoder = new TextEncoder();

export function byteLength(text) {
  return encoder.encode(text).byteLength;
}

export function encodeLine(row) {
  const line = `${JSON.stringify(row)}\n`;
  requireThat(byteLength(line) <= MAX_LINE_BYTES, 'record exceeds 1 MiB including newline');
  return line;
}

export function parseLine(line) {
  requireThat(line.endsWith('\n'), 'truncated history record (missing newline)');
  requireThat(byteLength(line) <= MAX_LINE_BYTES, 'record exceeds 1 MiB including newline');
  const value = JSON.parse(line);
  // JSON.parse discards duplicate keys, whereas the native serde wire rejects them.
  const stack = [];
  const tokens = line.match(/"(?:[^"\\]|\\.)*"|[{}\[\]:,]|[^\s{}\[\]:,]+/g) ?? [];
  for (const token of tokens) {
    const frame = stack.at(-1);
    const path = frame ? [...frame.path, frame.seen ? frame.field : frame.index] : [];
    if (token[0] === '"') {
      const text = JSON.parse(token);
      requireThat(text.isWellFormed(), 'invalid Unicode string');
      if (frame?.key) {
        requireThat(!frame.seen.has(text), `duplicate field ${text}`);
        frame.seen.add(text);
        frame.field = text;
        frame.key = false;
      }
    } else if (token === '{') stack.push({ key: true, seen: new Set(), path });
    else if (token === '[') stack.push({ key: false, index: 0, path });
    else if (token === '}' || token === ']') stack.pop();
    else if (token === ',') {
      if (frame?.seen) frame.key = true;
      else if (frame) frame.index++;
    } else if (/^-?[0-9]/.test(token)) {
      const integer = path[0] !== 'data' || path[1] !== 'params' || integerParam(path.slice(2));
      requireThat(!integer || /^(0|[1-9][0-9]*)$/.test(token), 'unsigned integer encoded as a non-integer JSON number');
    }
  }
  return value;
}
