/** Normalizes wheel units before converting a scroll into an exponential zoom factor. */

const PIXELS_PER_LINE = 16;
const MAX_DELTA_PIXELS = 1000;

export function wheelZoomFactor(event) {
  const pagePixels = Math.max(1, event.currentTarget?.clientHeight ?? 800);
  const unit = event.deltaMode === 1 ? PIXELS_PER_LINE : event.deltaMode === 2 ? pagePixels : 1;
  const pixels = Math.max(
    -MAX_DELTA_PIXELS,
    Math.min(MAX_DELTA_PIXELS, event.deltaY * unit),
  );
  return Math.exp(-pixels * 0.0015);
}
