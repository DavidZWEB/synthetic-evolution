/**
 * GLSL programs for instanced agents and plants.
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

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_radius;
uniform float u_min_radius;
uniform float u_max_energy;
uniform float u_world;

out vec2 v_corner;
out float v_fullness;

void main() {
  v_corner = a_corner;
  v_fullness = clamp(a_energy / u_max_energy, 0.0, 1.0);

  vec2 d = a_position.xy - u_center;
  d -= u_world * floor(d / u_world + 0.5);

  float radius = max(u_radius * u_ppu, u_min_radius);
  vec2 pixels = d * u_ppu + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

export const PLANT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in float v_fullness;
uniform vec3 u_color;
out vec4 fragment;

void main() {
  if (dot(v_corner, v_corner) > 1.0) discard;
  // Empty sites remain visible because they persist and regrow (spec §5.1).
  fragment = vec4(u_color * (0.18 + 0.82 * v_fullness), 1.0);
}`;
