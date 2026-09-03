/**
 * WebGL2 instanced renderer: one quad, drawn once per slot.
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
 */

const VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec3 a_position;
in float a_size;
in vec3 a_signature;
in float a_alive;

uniform vec2 u_world;
uniform vec2 u_scale;
uniform vec2 u_offset;

out vec2 v_corner;
out vec3 v_color;

void main() {
  v_corner = a_corner;
  v_color = a_signature;

  // A dead slot has alive = 0, so its radius is 0 and its quad has no area. Nothing is
  // drawn and no branch was taken.
  float radius = a_size * a_alive;
  vec2 world = a_position.xy + a_corner * radius;

  vec2 unit = world / u_world;
  gl_Position = vec4(unit * 2.0 * u_scale - u_scale + u_offset, 0.0, 1.0);
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
export function createRenderer(canvas, { worldSize, capacity }) {
  const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  if (!gl) throw new Error('WebGL2 is unavailable in this browser');

  const program = link(gl, VERTEX_SHADER, FRAGMENT_SHADER);
  gl.useProgram(program);

  const uniforms = {
    world: gl.getUniformLocation(program, 'u_world'),
    scale: gl.getUniformLocation(program, 'u_scale'),
    offset: gl.getUniformLocation(program, 'u_offset'),
  };

  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);

  // The quad every instance is drawn from: a triangle strip of four corners.
  const corners = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );
  const cornerAttr = gl.getAttribLocation(program, 'a_corner');
  gl.enableVertexAttribArray(cornerAttr);
  gl.vertexAttribPointer(cornerAttr, 2, gl.FLOAT, false, 0, 0);

  /** One per-instance attribute, sized once at capacity and refilled each frame. */
  function instanced(name, components, type, bytesPerComponent, normalized = false) {
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      capacity * components * bytesPerComponent,
      gl.DYNAMIC_DRAW,
    );
    const location = gl.getAttribLocation(program, name);
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, components, type, normalized, 0, 0);
    gl.vertexAttribDivisor(location, 1);
    return buffer;
  }

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
  gl.clearColor(0.055, 0.063, 0.078, 1);

  let width = 0;
  let height = 0;

  function resize() {
    const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
    const next = {
      width: Math.max(1, Math.round(canvas.clientWidth * ratio)),
      height: Math.max(1, Math.round(canvas.clientHeight * ratio)),
    };
    if (next.width === width && next.height === height) return;
    width = next.width;
    height = next.height;
    canvas.width = width;
    canvas.height = height;
    gl.viewport(0, 0, width, height);
  }

  return {
    resize,

    /** Draws one frame's views. `count` is the slot count, not the population. */
    draw(views, count) {
      resize();
      gl.clear(gl.COLOR_BUFFER_BIT);
      if (!views) return;

      gl.useProgram(program);
      gl.bindVertexArray(vao);

      const upload = (buffer, data) => {
        gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, data);
      };
      upload(attributes.position, views.position);
      upload(attributes.size, views.size);
      upload(attributes.signature, views.signature);
      upload(attributes.alive, views.alive);

      // Letterbox rather than stretch: a world that changed shape with the window would
      // make distance mean something different along each axis, and every judgement
      // about clustering would be a judgement about the window.
      const fit = Math.min(width, height);
      gl.uniform2f(uniforms.world, worldSize, worldSize);
      gl.uniform2f(uniforms.scale, fit / width, fit / height);
      gl.uniform2f(uniforms.offset, 0, 0);

      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);
      gl.bindVertexArray(null);
    },

    destroy() {
      gl.deleteProgram(program);
      gl.deleteVertexArray(vao);
      gl.deleteBuffer(corners);
      for (const buffer of Object.values(attributes)) gl.deleteBuffer(buffer);
    },
  };
}
