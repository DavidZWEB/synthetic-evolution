/**
 * Pointer gesture state for the simulation viewport.
 *
 * Keeps mutable pointer bookkeeping out of Svelte while reporting the small reactive
 * surface the component needs: pointer count and deliberate click coordinates.
 */

export function createPointerGestures({ getRenderer, onPointerCount, onClick }) {
  const pointers = new Map();
  let pinchDistance = 0;
  let clickStart = null;

  const spread = () => {
    const [a, b] = [...pointers.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };
  const midpoint = () => {
    const [a, b] = [...pointers.values()];
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  };

  function release(event) {
    pointers.delete(event.pointerId);
    onPointerCount(pointers.size);
    if (pointers.size < 2) pinchDistance = 0;
  }

  return {
    down(event) {
      try {
        event.currentTarget.setPointerCapture(event.pointerId);
      } catch {
        // Capture is an enhancement; dragging can still continue inside the canvas.
      }
      clickStart =
        pointers.size === 0
          ? { id: event.pointerId, x: event.clientX, y: event.clientY }
          : null;
      pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
      onPointerCount(pointers.size);
      if (pointers.size === 2) pinchDistance = spread();
    },

    move(event) {
      const previous = pointers.get(event.pointerId);
      if (!previous) return;
      const next = { x: event.clientX, y: event.clientY };
      if (
        clickStart?.id === event.pointerId &&
        Math.hypot(next.x - clickStart.x, next.y - clickStart.y) > 4
      ) {
        clickStart = null;
      }

      if (pointers.size === 1 && clickStart === null) {
        getRenderer()?.panBy(next.x - previous.x, next.y - previous.y);
      }
      pointers.set(event.pointerId, next);

      if (pointers.size === 2 && pinchDistance > 0) {
        const distance = spread();
        const centre = midpoint();
        getRenderer()?.zoomAt(centre.x, centre.y, distance / pinchDistance);
        pinchDistance = distance;
      }
    },

    up: release,

    cancel(event) {
      clickStart = null;
      release(event);
    },

    click(event) {
      if (!clickStart) return;
      clickStart = null;
      onClick(event.clientX, event.clientY);
    },
  };
}
