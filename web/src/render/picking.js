/**
 * Agent hit-testing in world coordinates.
 *
 * This is deliberately a click-time linear scan rather than another spatial index: it
 * runs at human speed, preserves slot identity, and must match the renderer's toroidal
 * nearest-image placement.
 */

/** `point` is unwrapped; `center` chooses the one toroidal image the renderer draws. */
export function pickAgent(views, capacity, point, worldSize, center, minimumRadius = 0) {
  let picked = null;
  let nearest = Number.POSITIVE_INFINITY;

  for (let index = 0; index < capacity; index += 1) {
    if (views.alive[index] !== 1) continue;
    let dx = views.position[index * 3] - center.x;
    let dy = views.position[index * 3 + 1] - center.y;
    dx -= worldSize * Math.floor(dx / worldSize + 0.5);
    dy -= worldSize * Math.floor(dy / worldSize + 0.5);
    dx += center.x - point.x;
    dy += center.y - point.y;
    const distanceSquared = dx * dx + dy * dy;
    const radius = Math.max(views.size[index], minimumRadius);
    if (distanceSquared <= radius * radius && distanceSquared < nearest) {
      picked = index;
      nearest = distanceSquared;
    }
  }

  return picked === null
    ? null
    : { index: picked, incarnation: views.incarnation[picked] };
}
