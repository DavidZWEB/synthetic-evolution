/**
 * Bites, wounds, and kills as wall-clock animations, for the renderer to draw.
 *
 * Each frame carries, per slot, the ticks since the agent last swung and since it was
 * last hit, where its latest swing landed, and its health (spec §2.2b). A swing or hit
 * younger than the ticks since the previous frame is new, and starts an animation timed
 * in wall-clock milliseconds, so it stays visible at any sim speed, as a plant's reseed
 * glow does. A kill is seen where a corpse appears beside an agent that vanished.
 * Display only: nothing here reaches the simulation.
 */

/** How long each animation lasts, in milliseconds. */
export const SWING_MS = 260;
export const HIT_MS = 420;
export const KILL_MS = 650;

/** Floats per overlay instance: centre x, y, radius, angle, r, g, b, a, kind, p1, p2. */
export const OVERLAY_STRIDE = 11;

/** Overlay shapes, as the effects shader draws them. */
export const SHAPE = { DISC: 0, RING: 1, WEDGE: 2, SEGMENT: 3 };

const SPECKS = 5;
/**
 * New kill animations one frame may start. A mass death draws its first kills rather
 * than matching every corpse against every vanished body on the UI thread.
 */
const KILLS_PER_FRAME = 64;
/** Where a swing or hit age saturates: "at least this long ago". */
const LONG_AGO = 255;
const [RED_R, RED_G, RED_B] = [0.95, 0.18, 0.15];

/**
 * The shortest offset from `from` to `to` on a torus `world` across, so an effect that
 * spans the seam is drawn across it rather than around the world.
 */
function wrapped(delta, world) {
  return delta - world * Math.floor(delta / world + 0.5);
}

/** Heading of a yaw-only quaternion `(x, y, z, w)` stored four floats per slot. */
function heading(orientation, slot) {
  return 2 * Math.atan2(orientation[slot * 4 + 2], orientation[slot * 4 + 3]);
}

/** A small deterministic hash in [0, 1), so specks scatter the same way every frame. */
function scatter(seed) {
  const x = Math.sin(seed * 12.9898) * 43758.5453;
  return x - Math.floor(x);
}

/**
 * Tracks combat events across frames for `capacity` agent slots and `corpseCapacity`
 * corpse slots in a world `worldSize` across, with room for `maxInstances` overlay
 * shapes and `maxKills` kills animating at once.
 */
export function createCombatEffects({
  capacity, corpseCapacity, worldSize, maxInstances = 4096, maxKills = 256,
}) {
  const swungAt = new Float64Array(capacity).fill(-Infinity);
  const hurtAt = new Float64Array(capacity).fill(-Infinity);
  // Where each agent's latest swing aimed: the victim's centre on a hit, NaN on a miss.
  const aim = new Float32Array(capacity * 2).fill(NaN);
  const wasAlive = new Uint8Array(capacity);
  const incarnation = new Uint32Array(capacity);
  const lastPosition = new Float32Array(capacity * 2);
  const lastSize = new Float32Array(capacity);
  const lastColor = new Float32Array(capacity * 3);
  // Each corpse slot as the previous frame showed it.
  const corpseHeld = new Uint8Array(corpseCapacity);
  const corpseAt = new Float32Array(corpseCapacity * 2);
  const corpseEnergy = new Float32Array(corpseCapacity);
  // Kills animating: x, y, the vanished body's radius and colour, and when.
  const kills = new Float64Array(maxKills * 7);
  let killCursor = 0;
  let lastTick = null;

  const offsets = new Float32Array(capacity * 2);
  const flashes = new Float32Array(capacity);
  const wounds = new Float32Array(capacity);
  const overlay = new Float32Array(maxInstances * OVERLAY_STRIDE);
  // Returned every frame rather than rebuilt, so drawing allocates nothing.
  const animation = { offsets, flashes, wounds };
  const shapes = { data: overlay, count: 0 };

  /** Appends one overlay shape, field by field: an array per shape would be garbage. */
  function push(x, y, radius, angle, r, g, b, a, kind, p1 = 0, p2 = 0) {
    if (shapes.count >= maxInstances) return;
    const o = shapes.count * OVERLAY_STRIDE;
    overlay[o] = x;
    overlay[o + 1] = y;
    overlay[o + 2] = radius;
    overlay[o + 3] = angle;
    overlay[o + 4] = r;
    overlay[o + 5] = g;
    overlay[o + 6] = b;
    overlay[o + 7] = a;
    overlay[o + 8] = kind;
    overlay[o + 9] = p1;
    overlay[o + 10] = p2;
    shapes.count++;
  }

  function forget() {
    swungAt.fill(-Infinity);
    hurtAt.fill(-Infinity);
    aim.fill(NaN);
    kills.fill(-Infinity);
    killCursor = 0;
  }
  forget();

  // Slots whose agent vanished since the previous frame, gathered once per frame so
  // matching a corpse to its body scans only them.
  const vanished = new Int32Array(capacity);
  let vanishedCount = 0;

  /**
   * The vanished slot whose last position lies nearest `(x, y)`, or -1, taken from the
   * candidates so no body is matched to two corpses.
   */
  function takeVanishedNear(x, y, reach) {
    let best = -1;
    let bestDistance = Infinity;
    for (let k = 0; k < vanishedCount; k++) {
      const i = vanished[k];
      const dx = wrapped(lastPosition[i * 2] - x, worldSize);
      const dy = wrapped(lastPosition[i * 2 + 1] - y, worldSize);
      const distance = Math.hypot(dx, dy);
      if (distance <= lastSize[i] + reach && distance < bestDistance) {
        best = k;
        bestDistance = distance;
      }
    }
    if (best < 0) return -1;
    const slot = vanished[best];
    vanished[best] = vanished[--vanishedCount];
    return slot;
  }

  return {
    /** How many overlay shapes one frame can hold. */
    maxInstances,

    /**
     * Notes what happened since the previous frame, at wall-clock `now`. The first
     * frame, and any frame whose tick does not advance, only sets the baseline: a world
     * that was just created or replaced has no events to replay. `colors` are the
     * agents' displayed colours, three floats per slot, for a kill's vanishing body.
     */
    observe(views, tick, now, colors, corpseRadius) {
      const elapsed = lastTick !== null && tick > lastTick ? Number(tick - lastTick) : 0;
      if (lastTick !== null && tick < lastTick) forget();
      // A saturated age says only "at least that long ago", so it is never new, however
      // many ticks a slow frame skipped; otherwise everyone would seem to swing at once.
      const horizon = Math.min(elapsed, LONG_AGO);
      for (let i = 0; i < capacity; i++) {
        if (views.alive[i] !== 1) continue;
        // A newborn in a reused slot must not inherit its predecessor's animation; its
        // own ages then decide, by the same rule.
        if (wasAlive[i] !== 1 || views.incarnation[i] !== incarnation[i]) {
          swungAt[i] = -Infinity;
          hurtAt[i] = -Infinity;
          aim[i * 2] = NaN;
          aim[i * 2 + 1] = NaN;
        }
        if (views.swingAge[i] < horizon) {
          swungAt[i] = now;
          aim[i * 2] = views.biteAt[i * 2];
          aim[i * 2 + 1] = views.biteAt[i * 2 + 1];
        }
        if (views.hurtAge[i] < horizon) hurtAt[i] = now;
      }
      vanishedCount = 0;
      if (elapsed > 0) {
        for (let i = 0; i < capacity; i++) {
          if (wasAlive[i] === 1 && (views.alive[i] === 0 || views.incarnation[i] !== incarnation[i])) {
            vanished[vanishedCount++] = i;
          }
        }
      }
      let started = 0;
      for (let c = 0; c < corpseCapacity; c++) {
        const energy = views.corpseEnergy[c];
        const x = views.corpsePosition[c * 3];
        const y = views.corpsePosition[c * 3 + 1];
        const held = energy > 0 ? 1 : 0;
        // A corpse never moves and only loses energy, so one that moved or gained is a
        // new death in a slot freed and refilled between frames (spec §5.1).
        const replaced = corpseHeld[c] === 1 &&
          (x !== corpseAt[c * 2] || y !== corpseAt[c * 2 + 1] || energy > corpseEnergy[c]);
        if (elapsed > 0 && held && (!corpseHeld[c] || replaced) && started < KILLS_PER_FRAME) {
          started++;
          const body = takeVanishedNear(x, y, corpseRadius);
          const at = killCursor * 7;
          kills[at] = x;
          kills[at + 1] = y;
          kills[at + 2] = body >= 0 ? lastSize[body] : corpseRadius;
          kills[at + 3] = body >= 0 ? lastColor[body * 3] : RED_R;
          kills[at + 4] = body >= 0 ? lastColor[body * 3 + 1] : RED_G;
          kills[at + 5] = body >= 0 ? lastColor[body * 3 + 2] : RED_B;
          kills[at + 6] = now;
          killCursor = (killCursor + 1) % maxKills;
        }
        corpseHeld[c] = held;
        corpseAt[c * 2] = x;
        corpseAt[c * 2 + 1] = y;
        corpseEnergy[c] = energy;
      }
      for (let i = 0; i < capacity; i++) {
        wasAlive[i] = views.alive[i];
        incarnation[i] = views.incarnation[i];
        if (views.alive[i] !== 1) continue;
        lastPosition[i * 2] = views.position[i * 3];
        lastPosition[i * 2 + 1] = views.position[i * 3 + 1];
        lastSize[i] = views.size[i];
        lastColor[i * 3] = colors[i * 3];
        lastColor[i * 3 + 1] = colors[i * 3 + 1];
        lastColor[i * 3 + 2] = colors[i * 3 + 2];
      }
      lastTick = tick;
    },

    /**
     * Records the colours living agents are now drawn in, after a redraw that changed
     * colours without a new frame, so a kill's vanishing body keeps the colour it was
     * last drawn in.
     */
    recolor(views, colors) {
      for (let i = 0; i < capacity; i++) {
        if (views.alive[i] !== 1) continue;
        lastColor[i * 3] = colors[i * 3];
        lastColor[i * 3 + 1] = colors[i * 3 + 1];
        lastColor[i * 3 + 2] = colors[i * 3 + 2];
      }
    },

    /**
     * Per-slot animation state at `now`: a world-unit offset (a swing's lunge toward its
     * aim, a hit's shake), a hit's red flash, and the wound health has not yet healed.
     */
    agents(views, now) {
      for (let i = 0; i < capacity; i++) {
        offsets[i * 2] = 0;
        offsets[i * 2 + 1] = 0;
        flashes[i] = 0;
        wounds[i] = views.alive[i] === 1 ? 1 - views.health[i] / 255 : 0;
        if (views.alive[i] !== 1) continue;
        const size = views.size[i];
        const swing = (now - swungAt[i]) / SWING_MS;
        if (swing >= 0 && swing < 1) {
          const angle = aimAngle(views, i);
          const lunge = 0.45 * size * Math.sin(Math.PI * swing);
          offsets[i * 2] += lunge * Math.cos(angle);
          offsets[i * 2 + 1] += lunge * Math.sin(angle);
        }
        const hurt = (now - hurtAt[i]) / HIT_MS;
        if (hurt >= 0 && hurt < 1) {
          const fade = 1 - hurt;
          const shake = 0.25 * size * fade;
          offsets[i * 2] += shake * Math.sin(hurt * 41 + i);
          offsets[i * 2 + 1] += shake * Math.cos(hurt * 47 + i);
          flashes[i] = fade;
        }
      }
      return animation;
    },

    /**
     * Overlay shapes at `now`, `OVERLAY_STRIDE` floats each: a swing's arc, a hit's
     * line and specks, and a kill's shrinking body and ring. `arc` and `reach` are the
     * bite's, from the params.
     */
    overlay(views, now, arc, reach, corpseRadius) {
      shapes.count = 0;
      for (let i = 0; i < capacity; i++) {
        if (views.alive[i] !== 1) continue;
        // The arc fades with the swing; a landed bite's line and specks last a hit's
        // longer time.
        const swing = (now - swungAt[i]) / SWING_MS;
        const hit = (now - swungAt[i]) / HIT_MS;
        if (!(hit >= 0 && hit < 1)) continue;
        const x = views.position[i * 3];
        const y = views.position[i * 3 + 1];
        const size = views.size[i];
        const angle = aimAngle(views, i);
        if (swing < 1) {
          push(x, y, size + reach, angle, 1, 0.82, 0.55, 0.28 * (1 - swing), SHAPE.WEDGE, arc);
        }
        if (!Number.isFinite(aim[i * 2])) continue;
        const fade = 1 - hit;
        const dx = wrapped(aim[i * 2] - x, worldSize);
        const dy = wrapped(aim[i * 2 + 1] - y, worldSize);
        push(x, y, Math.hypot(dx, dy), Math.atan2(dy, dx), RED_R, RED_G, RED_B, 0.75 * fade,
          SHAPE.SEGMENT, 0.18 * size);
        // Specks fly from the biter's mouth, where it met its victim.
        const mouthX = x + size * Math.cos(angle);
        const mouthY = y + size * Math.sin(angle);
        for (let s = 0; s < SPECKS; s++) {
          const spread = angle + (scatter(i * 31 + s + swungAt[i]) - 0.5) * 2.4;
          const travel = size * (0.4 + 1.6 * hit) * (0.6 + 0.8 * scatter(i * 17 + s));
          push(mouthX + travel * Math.cos(spread), mouthY + travel * Math.sin(spread),
            0.16 * size, 0, RED_R, RED_G, RED_B, fade, SHAPE.DISC);
        }
      }
      for (let k = 0; k < kills.length / 7; k++) {
        const at = k * 7;
        const t = (now - kills[at + 6]) / KILL_MS;
        if (!(t >= 0 && t < 1)) continue;
        const x = kills[at];
        const y = kills[at + 1];
        const body = kills[at + 2];
        // The body shrinks into its corpse in the first half; the ring spreads throughout.
        const shrink = Math.min(1, t * 2);
        if (shrink < 1) {
          push(x, y, body + (corpseRadius - body) * shrink, 0,
            kills[at + 3], kills[at + 4], kills[at + 5], 1 - shrink, SHAPE.DISC);
        }
        push(x, y, corpseRadius * (1 + 3 * t), 0, RED_R, RED_G, RED_B, 0.8 * (1 - t), SHAPE.RING,
          0.18);
      }
      return shapes;
    },
  };

  /** Where slot `i` aimed its latest swing: at its victim on a hit, else straight ahead. */
  function aimAngle(views, i) {
    if (Number.isFinite(aim[i * 2])) {
      return Math.atan2(
        wrapped(aim[i * 2 + 1] - views.position[i * 3 + 1], worldSize),
        wrapped(aim[i * 2] - views.position[i * 3], worldSize),
      );
    }
    return heading(views.orientation, i);
  }
}
