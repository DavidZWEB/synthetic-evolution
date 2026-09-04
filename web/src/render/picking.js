/**
 * Agent hit-testing in world coordinates.
 *
 * This is deliberately a click-time linear scan rather than another spatial index: it
 * runs at human speed, preserves slot identity, and must match the renderer's toroidal
 * nearest-image placement.
 */

export function pickAgent(views, capacity, point, worldSize, minimumRadius = 0) {
  let picked = null;
  let nearest = Number.POSITIVE_INFINITY;

  for (let index = 0; index < capacity; index += 1) {
    if (views.alive[index] !== 1) continue;
    let dx = views.position[index * 3] - point.x;
    let dy = views.position[index * 3 + 1] - point.y;
    dx -= worldSize * Math.round(dx / worldSize);
    dy -= worldSize * Math.round(dy / worldSize);
    const distanceSquared = dx * dx + dy * dy;
    const radius = Math.max(views.size[index], minimumRadius);
    if (distanceSquared <= radius * radius && distanceSquared < nearest) {
      picked = index;
      nearest = distanceSquared;
    }
  }

  return picked;
}
