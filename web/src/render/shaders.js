/**
 * GLSL programs for instanced agents, for food (plants and corpses share one disc
 * program with different uniforms), and for the combat overlay drawn over both.
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
// Combat animation: a lunge or shake in world units, a hit's flash, and the share of
// health a wound has not yet healed.
in vec2 a_offset;
in float a_flash;
in float a_wound;

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_min_radius;
uniform float u_world;
uniform int u_selected;

out vec2 v_corner;
out vec3 v_color;
out float v_flash;
out float v_wound;
flat out float v_selected;

vec2 toward(vec2 point, vec2 from, float extent) {
  // Explicit half-world ties keep CPU picking and GPU image placement identical.
  vec2 d = point - from;
  return d - extent * floor(d / extent + 0.5);
}

void main() {
  v_corner = a_corner;
  v_color = a_signature;
  v_flash = a_flash;
  v_wound = a_wound;
  v_selected = gl_InstanceID == u_selected ? 1.0 : 0.0;

  // Dead snapshot slots collapse without a branch or CPU compaction.
  float radius = max(a_size * u_ppu, u_min_radius) * a_alive;
  vec2 pixels = (toward(a_position.xy, u_center, u_world) + a_offset) * u_ppu
    + a_corner * radius;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

export const AGENT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_corner;
in vec3 v_color;
in float v_flash;
in float v_wound;
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
  vec3 color = v_color * shade;
  // A wound rims the body in red, wider the more health is missing, so it narrows and
  // goes as health regenerates.
  if (v_wound > 0.02 && r2 > 1.0 - 0.5 * v_wound) color = mix(color, vec3(0.9, 0.12, 0.1), 0.85);
  // A hit flashes the whole body red, fading.
  color = mix(color, vec3(1.0, 0.3, 0.25), 0.75 * v_flash);
  fragment = vec4(color, 1.0);
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

/**
 * Combat overlay: translucent shapes drawn over the agents, one instance each, from
 * `combat-effects.js`'s overlay records. A disc, a ring, a wedge (the bite's arc), or a
 * segment (biter to victim), all from the same quad.
 */
export const EFFECT_VERTEX_SHADER = `#version 300 es
precision highp float;

in vec2 a_corner;
in vec2 a_center;
in float a_radius;
in float a_angle;
in vec4 a_color;
in float a_kind;
in vec2 a_params;

uniform vec2 u_center;
uniform float u_ppu;
uniform vec2 u_viewport;
uniform float u_world;

out vec2 v_local;
out vec4 v_color;
flat out float v_kind;
flat out vec2 v_params;

void main() {
  v_color = a_color;
  v_kind = a_kind;
  v_params = a_params;
  vec2 d = a_center - u_center;
  d -= u_world * floor(d / u_world + 0.5);
  vec2 axis = vec2(cos(a_angle), sin(a_angle));
  vec2 across = vec2(-axis.y, axis.x);
  vec2 offset;
  if (a_kind > 2.5) {
    // A segment runs from the centre along its angle for its radius, p1 thick.
    v_local = a_corner;
    offset = axis * (a_corner.x + 1.0) * 0.5 * a_radius + across * a_corner.y * a_params.x;
  } else {
    // Everything else fills a square of its radius, turned to its angle.
    v_local = a_corner;
    offset = (axis * a_corner.x + across * a_corner.y) * a_radius;
  }
  vec2 pixels = (d + offset) * u_ppu;
  gl_Position = vec4(pixels / (u_viewport * 0.5), 0.0, 1.0);
}`;

export const EFFECT_FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_local;
in vec4 v_color;
flat in float v_kind;
flat in vec2 v_params;
out vec4 fragment;

void main() {
  float r = length(v_local);
  float alpha = v_color.a;
  if (v_kind < 0.5) {
    // Disc.
    if (r > 1.0) discard;
  } else if (v_kind < 1.5) {
    // Ring, p1 of the radius thick.
    if (r > 1.0 || r < 1.0 - v_params.x) discard;
  } else if (v_kind < 2.5) {
    // Wedge: within the radius and p1 radians of the angle, fading toward the rim.
    if (r > 1.0 || abs(atan(v_local.y, v_local.x)) > v_params.x) discard;
    alpha *= 1.0 - 0.6 * r;
  } else {
    // Segment: fades toward both ends.
    alpha *= 1.0 - abs(v_local.x);
  }
  fragment = vec4(v_color.rgb, alpha);
}`;
