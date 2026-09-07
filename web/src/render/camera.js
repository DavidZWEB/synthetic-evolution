/**
 * Toroidal camera and screen/world coordinate conversion.
 *
 * Owns viewport sizing and camera math, but knows nothing about WebGL resources or
 * snapshot data. Keeping those concerns separate makes pointer behavior testable
 * without constructing a renderer.
 */

export function createCamera(canvas, worldSize, onResize) {
  let width = 0;
  let height = 0;
  const state = { x: worldSize / 2, y: worldSize / 2, ppu: 1 };

  /** Zoom that exactly fits the world into the smaller viewport axis. */
  const fitPpu = () => Math.min(width, height) / worldSize;
  const wrap = (value) => ((value % worldSize) + worldSize) % worldSize;
  const clampPpu = (ppu) => Math.min(Math.max(ppu, fitPpu()), fitPpu() * 400);

  function resize() {
    const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
    const next = {
      width: Math.max(1, Math.round(canvas.clientWidth * ratio)),
      height: Math.max(1, Math.round(canvas.clientHeight * ratio)),
    };
    if (next.width === width && next.height === height) return;

    // Hold zoom relative to a fitted world, so resizing reframes rather than zooms.
    const relative = width === 0 ? 1 : state.ppu / fitPpu();
    width = next.width;
    height = next.height;
    canvas.width = width;
    canvas.height = height;
    onResize(width, height);
    state.ppu = fitPpu() * relative;
  }

  /**
   * Converts a CSS point to device pixels from the viewport centre, in world
   * orientation. CSS y grows downward while world y grows upward.
   */
  function toDevice(cssX, cssY) {
    const rect = canvas.getBoundingClientRect();
    return {
      x: (cssX - rect.left) * (width / rect.width) - width / 2,
      y: -((cssY - rect.top) * (height / rect.height) - height / 2),
    };
  }

  function screenToWorldUnwrapped(cssX, cssY) {
    const { x, y } = toDevice(cssX, cssY);
    return { x: state.x + x / state.ppu, y: state.y + y / state.ppu };
  }

  function screenToWorld(cssX, cssY) {
    const point = screenToWorldUnwrapped(cssX, cssY);
    return { x: wrap(point.x), y: wrap(point.y) };
  }

  return {
    state,
    worldSize,
    resize,
    get width() {
      return width;
    },
    get height() {
      return height;
    },

    view() {
      return { x: state.x, y: state.y, zoom: state.ppu / fitPpu() };
    },

    setView({ x, y, zoom }) {
      resize();
      state.x = wrap(x);
      state.y = wrap(y);
      state.ppu = clampPpu(fitPpu() * zoom);
    },

    fit() {
      resize();
      state.x = worldSize / 2;
      state.y = worldSize / 2;
      state.ppu = fitPpu();
    },

    zoom() {
      return state.ppu / fitPpu();
    },

    zoomAt(cssX, cssY, factor) {
      const anchor = screenToWorld(cssX, cssY);
      const next = clampPpu(state.ppu * factor);
      if (next === state.ppu) return;

      const { x, y } = toDevice(cssX, cssY);
      state.ppu = next;
      state.x = wrap(anchor.x - x / next);
      state.y = wrap(anchor.y - y / next);
    },

    panBy(cssDx, cssDy) {
      const rect = canvas.getBoundingClientRect();
      state.x = wrap(state.x - (cssDx * (width / rect.width)) / state.ppu);
      state.y = wrap(state.y + (cssDy * (height / rect.height)) / state.ppu);
    },

    screenToWorld,
    screenToWorldUnwrapped,
  };
}
