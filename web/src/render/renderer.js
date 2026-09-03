/**
 * WebGL2 instanced renderer: one quad, drawn once per slot, on a torus.
 *
 * WebGL2 rather than canvas2d, deliberately. canvas2d issues a draw call per agent and
 * falls over well before this phase's 5000; instancing issues one for the whole
 * population, and the per-instance attributes are exactly the arrays the snapshot
 * already stores. Canvas2d would also be thrown away at Phase 5 when bodies become
 * multi-part, so it is not even a shortcut worth taking twice.
 *
 * **Dead slots collapse rather than branch.** The snapshot is slot-indexed, so a frame
 * carries gaps; the vertex shader multiplies the radius by the `alive` byte, and a dead
 * agent becomes a degenerate quad the rasteriser discards. That keeps the draw one call
 * over a fixed instance count instead of a per-frame compaction pass on the CPU.
 *
 * Agents are drawn as discs by discarding outside the unit circle in the fragment
 * shader. A texture would be one more thing to load before anything appears on screen.
 *
 * **The world wraps, so the picture has to — but each agent is drawn once.** Both vertex
 * shaders place every instance at its nearest image to the camera centre, the same
 * minimum-image rule spatial.rs measures distance with, so panning across the seam is
 * continuous rather than hitting a wall the picture invented.
 *
 * That places the whole population inside one world-sized band around the camera, so an
 * agent never appears twice. The cost is that a viewport wider than the world has empty
 * margins rather than the wrapped copies a torus strictly has — the alternative, tiling
 * the draw, fills them by showing the same agent two or three times over, which makes a
 * population look larger than it is and is worse for the one question this view exists to
 * answer. The zoom floor stops just where the whole world is on screen, so the margins
 * only ever appear on the long axis of a window the world does not match.
 */

const VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec3 a_position;
in float a_size;
in vec3 a_signature;
in float a_alive;

uniform vec2 u_center;      // world units the viewport is centred on
uniform float u_ppu;        // pixels per world unit
uniform vec2 u_viewport;    // device pixels
uniform float u_min_radius; // device pixels
uniform float u_world;      // world extent, for the wrap

out vec2 v_corner;
out vec3 v_color;

// Offset from the camera to the *nearest image* of a point on the torus.
//
// The world wraps in x and y, so an agent at 999 and one at 1 are a couple of units
// apart and must draw that way. This is the same minimum-image rule that spatial.rs
// measures every distance with; without it the seam becomes a wall the picture invents
// and the simulation does not have.
vec2 toward(vec2 point, vec2 from, float extent) {
  vec2 d = point - from;
  return d - extent * round(d / extent);
}

void main() {
  v_corner = a_corner;
  v_color = a_signature;

  // A dead slot has alive = 0, so every radius term below multiplies to zero and the
  // quad has no area. Nothing is drawn and no branch was taken.
  float radius = max(a_size * u_ppu, u_min_radius) * a_alive;
  vec2 pixels = toward(a_position.xy, u_center, u_world) * u_ppu + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

/**
 * Plants: the same quad, sized by a uniform radius and shaded by how much the site holds.
 *
 * A separate program rather than a branch in the agent shader. Plants differ in every
 * respect that matters to a draw — one radius for all of them, colour from a parameter
 * rather than a gene, and no orientation — and folding them together would mean uploading
 * per-plant copies of values that are the same for every one.
 */
const PLANT_VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec3 a_position;
in float a_energy;

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_radius;      // world units
uniform float u_min_radius;  // device pixels
uniform float u_max_energy;
uniform float u_world;

out vec2 v_corner;
out float v_fullness;

void main() {
  v_corner = a_corner;
  v_fullness = clamp(a_energy / u_max_energy, 0.0, 1.0);

  vec2 d = a_position.xy - u_center;
  d -= u_world * round(d / u_world);

  float radius = max(u_radius * u_ppu, u_min_radius);
  vec2 pixels = d * u_ppu + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

const PLANT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in float v_fullness;
uniform vec3 u_color;
out vec4 fragment;

void main() {
  if (dot(v_corner, v_corner) > 1.0) discard;
  // An emptied site stays visible rather than blinking out: it persists and regrows
  // (spec §5.1), and a larder that vanished as it was eaten would make a starving world
  // look like an empty one.
  fragment = vec4(u_color * (0.18 + 0.82 * v_fullness), 1.0);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in vec3 v_color;
out vec4 fragment;

void main() {
  float r2 = dot(v_corner, v_corner);
  // Outside the inscribed circle the quad is not part of the agent.
  if (r2 > 1.0) discard;
  // A little shading toward the rim, so overlapping agents stay countable.
  float shade = 0.65 + 0.35 * (1.0 - r2);
  fragment = vec4(v_color * shade, 1.0);
}`;

function compile(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`shader failed to compile: ${log}`);
  }
  return shader;
}

function link(gl, vertexSource, fragmentSource) {
  const program = gl.createProgram();
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, vertexSource));
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, fragmentSource));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(`program failed to link: ${gl.getProgramInfoLog(program)}`);
  }
  return program;
}

/**
 * Builds a renderer over `canvas` for a world `worldSize` across holding `capacity` slots.
 *
 * Throws when WebGL2 is unavailable rather than falling back to canvas2d: a silent
 * downgrade to something that cannot hold the frame rate reads as "the sim is slow".
 */
export function createRenderer(canvas, { worldSize, capacity, plantCapacity, plantRadius, plantColor, plantMaxEnergy }) {
  const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  if (!gl) throw new Error('WebGL2 is unavailable in this browser');

  const program = link(gl, VERTEX_SHADER, FRAGMENT_SHADER);
  gl.useProgram(program);

  const uniforms = {
    center: gl.getUniformLocation(program, 'u_center'),
    ppu: gl.getUniformLocation(program, 'u_ppu'),
    viewport: gl.getUniformLocation(program, 'u_viewport'),
    minRadius: gl.getUniformLocation(program, 'u_min_radius'),
    world: gl.getUniformLocation(program, 'u_world'),
  };

  const plantProgram = link(gl, PLANT_VERTEX_SHADER, PLANT_FRAGMENT_SHADER);
  const plantUniforms = {
    center: gl.getUniformLocation(plantProgram, 'u_center'),
    ppu: gl.getUniformLocation(plantProgram, 'u_ppu'),
    viewport: gl.getUniformLocation(plantProgram, 'u_viewport'),
    radius: gl.getUniformLocation(plantProgram, 'u_radius'),
    minRadius: gl.getUniformLocation(plantProgram, 'u_min_radius'),
    color: gl.getUniformLocation(plantProgram, 'u_color'),
    maxEnergy: gl.getUniformLocation(plantProgram, 'u_max_energy'),
    world: gl.getUniformLocation(plantProgram, 'u_world'),
  };

  // The quad every instance is drawn from: a triangle strip of four corners, shared by
  // both programs.
  const corners = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );

  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  const cornerAttr = gl.getAttribLocation(program, 'a_corner');
  gl.enableVertexAttribArray(cornerAttr);
  gl.vertexAttribPointer(cornerAttr, 2, gl.FLOAT, false, 0, 0);

  /** One per-instance attribute, sized once at capacity and refilled each frame. */
  function instancedFor(target, name, components, type, bytesPerComponent, slots, normalized = false) {
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, slots * components * bytesPerComponent, gl.DYNAMIC_DRAW);
    const location = gl.getAttribLocation(target, name);
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, components, type, normalized, 0, 0);
    gl.vertexAttribDivisor(location, 1);
    return buffer;
  }

  const instanced = (name, components, type, bytes, normalized = false) =>
    instancedFor(program, name, components, type, bytes, capacity, normalized);

  const attributes = {
    position: instanced('a_position', 3, gl.FLOAT, 4),
    size: instanced('a_size', 1, gl.FLOAT, 4),
    signature: instanced('a_signature', 3, gl.FLOAT, 4),
    // **Not** normalized. The flag is 1 for a live slot, not 255, so normalizing would
    // divide it to 1/255 and shrink every agent to four thousandths of its radius —
    // which renders as an empty world rather than as an error.
    alive: instanced('a_alive', 1, gl.UNSIGNED_BYTE, 1, false),
  };

  gl.bindVertexArray(null);

  // Plants get their own vertex array so neither program has to rebind the other's
  // attributes every frame.
  const plantVao = gl.createVertexArray();
  gl.bindVertexArray(plantVao);
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  const plantCornerAttr = gl.getAttribLocation(plantProgram, 'a_corner');
  gl.enableVertexAttribArray(plantCornerAttr);
  gl.vertexAttribPointer(plantCornerAttr, 2, gl.FLOAT, false, 0, 0);

  const plantAttributes = {
    position: instancedFor(plantProgram, 'a_position', 3, gl.FLOAT, 4, plantCapacity),
    energy: instancedFor(plantProgram, 'a_energy', 1, gl.FLOAT, 4, plantCapacity),
  };
  gl.bindVertexArray(null);

  gl.clearColor(0.055, 0.063, 0.078, 1);

  let width = 0;
  let height = 0;
  /** Device pixels per CSS pixel, for sizing the backing store. */
  let ratio = 1;

  /**
   * The camera: where the viewport is centred, in world units, and how magnified it is.
   *
   * Held here rather than in the component because it is the projection — the same thing
   * the shaders read — and a second copy anywhere is a chance for the picture and the
   * pointer to disagree about where a click landed.
   */
  const camera = { x: worldSize / 2, y: worldSize / 2, ppu: 1 };

  /** Zoom that exactly fits the world into the smaller viewport axis. */
  function fitPpu() {
    return Math.min(width, height) / worldSize;
  }

  /**
   * Below a whole pixel an agent stops being drawn at all rather than drawn faintly, so
   * zooming out far enough would empty the world for a reason that is nothing to do with
   * the simulation. A floor keeps the population legible as a density instead.
   */
  const MIN_RADIUS_PX = 1.1;

  function resize() {
    ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
    const next = {
      width: Math.max(1, Math.round(canvas.clientWidth * ratio)),
      height: Math.max(1, Math.round(canvas.clientHeight * ratio)),
    };
    if (next.width === width && next.height === height) return;

    // Hold the zoom *relative to a fitted world* across a resize, so dragging the window
    // reframes the view rather than magnifying it.
    const relative = width === 0 ? 1 : camera.ppu / fitPpu();
    width = next.width;
    height = next.height;
    canvas.width = width;
    canvas.height = height;
    gl.viewport(0, 0, width, height);
    camera.ppu = fitPpu() * relative;
  }

  /** Wraps a camera coordinate back onto the torus, so panning loops rather than ends. */
  const wrap = (value) => ((value % worldSize) + worldSize) % worldSize;

  /**
   * Holds zoom inside its range.
   *
   * The floor is a fitted world: past it the whole world is already on screen and zooming
   * out only adds copies of it. Shared by every path that sets zoom so a restored view
   * cannot land somewhere the wheel would refuse to reach.
   */
  const clampPpu = (ppu) => Math.min(Math.max(ppu, fitPpu()), fitPpu() * 400);

  /**
   * A point in CSS pixels, as an offset in device pixels from the viewport's centre.
   *
   * Scaled by the backing store against the rect the browser actually laid out, not by
   * `devicePixelRatio`. Layout sizes are fractional and `canvas.width` is a whole number
   * of pixels, so the two disagree by a fraction of a pixel — which is invisible until
   * you anchor a zoom to the cursor, where it shows up as the world creeping out from
   * under the pointer.
   */
  function toDevice(cssX, cssY) {
    const rect = canvas.getBoundingClientRect();
    return {
      x: (cssX - rect.left) * (width / rect.width) - width / 2,
      y: (cssY - rect.top) * (height / rect.height) - height / 2,
    };
  }

  /** CSS pixels from the canvas's top-left, to a point in the world. */
  function screenToWorld(cssX, cssY) {
    const { x, y } = toDevice(cssX, cssY);
    return { x: wrap(camera.x + x / camera.ppu), y: wrap(camera.y + y / camera.ppu) };
  }

  return {
    resize,
    camera,
    worldSize,

    /**
     * The camera as a portable value: where it looks, and how magnified it is *relative to
     * a fitted world*.
     *
     * Relative rather than absolute, because `ppu` is pixels per world unit and depends on
     * the canvas size. Handing an absolute zoom to a renderer built at a different size
     * would silently change the magnification.
     */
    view() {
      return { x: camera.x, y: camera.y, zoom: camera.ppu / fitPpu() };
    },

    /** Restores a view taken from [`view`]. */
    setView({ x, y, zoom }) {
      resize();
      camera.x = wrap(x);
      camera.y = wrap(y);
      camera.ppu = clampPpu(fitPpu() * zoom);
    },

    /** Frames the whole world. */
    fit() {
      resize();
      camera.x = worldSize / 2;
      camera.y = worldSize / 2;
      camera.ppu = fitPpu();
    },

    /** How magnified the view is, as a multiple of the whole world fitting the viewport. */
    zoom() {
      return camera.ppu / fitPpu();
    },

    /**
     * Zooms by `factor` about a point on screen, holding whatever is under it still.
     *
     * Anchoring to the cursor rather than the centre is what makes a wheel feel like a
     * magnifier rather than a slider: the thing you were looking at is still the thing
     * you are looking at.
     */
    zoomAt(cssX, cssY, factor) {
      const anchor = screenToWorld(cssX, cssY);
      const next = clampPpu(camera.ppu * factor);
      if (next === camera.ppu) return;

      const { x, y } = toDevice(cssX, cssY);
      camera.ppu = next;
      camera.x = wrap(anchor.x - x / next);
      camera.y = wrap(anchor.y - y / next);
    },

    /** Drags the view by a mouse delta in CSS pixels. */
    panBy(cssDx, cssDy) {
      const rect = canvas.getBoundingClientRect();
      camera.x = wrap(camera.x - (cssDx * (width / rect.width)) / camera.ppu);
      camera.y = wrap(camera.y - (cssDy * (height / rect.height)) / camera.ppu);
    },

    screenToWorld,

    /** Draws one frame's views. `count` is the slot count, not the population. */
    draw(views, count) {
      resize();
      gl.clear(gl.COLOR_BUFFER_BIT);
      if (!views) return;

      const upload = (buffer, data) => {
        gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, data);
      };

      // Plants first, so agents draw over the food rather than under it.
      if (plantCapacity > 0) {
        gl.useProgram(plantProgram);
        gl.bindVertexArray(plantVao);
        upload(plantAttributes.position, views.plantPosition);
        upload(plantAttributes.energy, views.plantEnergy);
        gl.uniform2f(plantUniforms.center, camera.x, camera.y);
        gl.uniform1f(plantUniforms.ppu, camera.ppu);
        gl.uniform2f(plantUniforms.viewport, width, height);
        gl.uniform1f(plantUniforms.world, worldSize);
        gl.uniform1f(plantUniforms.radius, plantRadius);
        gl.uniform1f(plantUniforms.minRadius, MIN_RADIUS_PX);
        gl.uniform1f(plantUniforms.maxEnergy, plantMaxEnergy);
        gl.uniform3f(plantUniforms.color, plantColor[0], plantColor[1], plantColor[2]);
        gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, plantCapacity);
      }

      gl.useProgram(program);
      gl.bindVertexArray(vao);
      upload(attributes.position, views.position);
      upload(attributes.size, views.size);
      upload(attributes.signature, views.signature);
      upload(attributes.alive, views.alive);
      gl.uniform2f(uniforms.center, camera.x, camera.y);
      gl.uniform1f(uniforms.ppu, camera.ppu);
      gl.uniform2f(uniforms.viewport, width, height);
      gl.uniform1f(uniforms.world, worldSize);
      gl.uniform1f(uniforms.minRadius, MIN_RADIUS_PX);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);

      gl.bindVertexArray(null);
    },

    destroy() {
      gl.deleteProgram(program);
      gl.deleteProgram(plantProgram);
      gl.deleteVertexArray(vao);
      gl.deleteVertexArray(plantVao);
      gl.deleteBuffer(corners);
      for (const buffer of Object.values(attributes)) gl.deleteBuffer(buffer);
      for (const buffer of Object.values(plantAttributes)) gl.deleteBuffer(buffer);
    },
  };
}
