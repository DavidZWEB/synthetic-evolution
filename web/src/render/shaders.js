/**
 * GLSL programs for instanced agents, and for food: plants and corpses share one
 * disc program with different uniforms.
 *
 * Rendering orchestration owns buffers and uniforms; this module owns only the visual
 * projection of one instance into a shaded disc.
 */

export const AGENT_VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec3 a_position;
in float a_size;
in vec3 a_signature;
in float a_alive;

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_min_radius;
uniform float u_world;
uniform int u_selected;

out vec2 v_corner;
out vec3 v_color;
flat out float v_selected;

vec2 toward(vec2 point, vec2 from, float extent) {
  // Explicit half-world ties keep CPU picking and GPU image placement identical.
  vec2 d = point - from;
  return d - extent * floor(d / extent + 0.5);
}

void main() {
  v_corner = a_corner;
  v_color = a_signature;
  v_selected = gl_InstanceID == u_selected ? 1.0 : 0.0;

  // Dead snapshot slots collapse without a branch or CPU compaction.
  float radius = max(a_size * u_ppu, u_min_radius) * a_alive;
  vec2 pixels = toward(a_position.xy, u_center, u_world) * u_ppu + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

export const AGENT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in vec3 v_color;
flat in float v_selected;
out vec4 fragment;

void main() {
  float r2 = dot(v_corner, v_corner);
  if (r2 > 1.0) discard;
  if (v_selected > 0.5 && r2 > 0.62) {
    fragment = vec4(1.0, 0.85, 0.35, 1.0);
    return;
  }
  float shade = 0.65 + 0.35 * (1.0 - r2);
  fragment = vec4(v_color * shade, 1.0);
}`;

export const PLANT_VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec3 a_position;
in float a_energy;
in float a_glow;

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_radius;
uniform float u_min_radius;
uniform float u_max_energy;
uniform float u_world;
// 1 for corpses, whose free slots hold nothing and must not draw; 0 for plants, whose
// empty sites persist and regrow (spec §5.1).
uniform float u_hide_empty;

out vec2 v_corner;
out float v_fullness;
out float v_glow;

void main() {
  v_corner = a_corner;
  v_fullness = clamp(a_energy / u_max_energy, 0.0, 1.0);
  v_glow = a_glow;

  vec2 d = a_position.xy - u_center;
  d -= u_world * floor(d / u_world + 0.5);

  // A just-reseeded plant swells, so it can be found at any zoom. A hidden empty slot
  // collapses to a degenerate quad, as a dead agent does.
  float radius = max(u_radius * u_ppu, u_min_radius) * (1.0 + 3.0 * a_glow)
    * (1.0 - u_hide_empty * step(a_energy, 0.0));
  vec2 pixels = d * u_ppu + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

export const PLANT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in float v_fullness;
in float v_glow;
uniform vec3 u_color;
out vec4 fragment;

void main() {
  if (dot(v_corner, v_corner) > 1.0) discard;
  // Empty plants stay visible: they regrow, or have just reseeded (spec §5.1).
  vec3 base = u_color * (0.18 + 0.82 * v_fullness);
  // A reseed flashes warm white and fades back to the plant's own colour.
  fragment = vec4(mix(base, vec3(1.0, 0.95, 0.7), 0.85 * v_glow), 1.0);
}`;
