export interface SkyPoint { x: number; y: number }
export const ORBIT_PAGE_SIZE = 16;
export const SKY_PAGE_SIZE = 12;
export const LOCAL_TRAVEL_MS = 1350;

export function hashSky(value: string): number {
  let hash = 2166136261;
  for (const char of value) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
  return hash >>> 0;
}

export function skyRandom(seed: string): () => number {
  let state = hashSky(seed);
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
}

// Best-candidate sampling creates a centered, irregular cloud, with a measured exclusion
// ellipse for each label. Re-centering each bounded page avoids a lopsided sparse directory.
export function sectionLayout(key: string, ids: string[]): Map<string, SkyPoint> {
  const points = new Map<string, SkyPoint>();
  const sorted = [...ids].sort();
  for (let page = 0; page * SKY_PAGE_SIZE < sorted.length; page++) {
    const names = sorted.slice(page * SKY_PAGE_SIZE, (page + 1) * SKY_PAGE_SIZE);
    const cloud: SkyPoint[] = [];
    const spread = names.length <= 6 ? 0.8 : 1;
    for (const id of names) {
      const random = skyRandom(`${key}/${id}`);
      let best = { x: .5, y: .48 };
      let bestScore = -Infinity;
      for (let attempt = 0; attempt < 160; attempt++) {
        const angle = random() * Math.PI * 2;
        const radius = Math.sqrt(random());
        const candidate = { x: .5 + Math.cos(angle) * radius * .38 * spread,
          y: .48 + Math.sin(angle) * radius * .4 * spread };
        const separation = cloud.length ? Math.min(...cloud.map((other) =>
          Math.hypot((candidate.x - other.x) / .19, (candidate.y - other.y) / .23))) : 1;
        // A mild center preference leaves negative space without producing concentric rings.
        const score = separation - radius * .22;
        if (score > bestScore) { best = candidate; bestScore = score; }
      }
      cloud.push(best);
    }
    const meanX = cloud.reduce((sum, p) => sum + p.x, 0) / cloud.length;
    const meanY = cloud.reduce((sum, p) => sum + p.y, 0) / cloud.length;
    const clearance = names.length > 8 ? 1.14 : 1;
    cloud.forEach((point, index) => points.set(names[index]!, {
      x: (point.x - meanX) * clearance + .5, y: (point.y - meanY) * clearance + .48,
    }));
  }
  return points;
}

export function orbitLayout(ids: string[]): Map<string, SkyPoint> {
  const points = new Map<string, SkyPoint>();
  // Up to eight per ring; offset rings avoid radial alignment and reserve the central label.
  ids.slice(0, ORBIT_PAGE_SIZE).forEach((id, index) => {
    const ring = Math.floor(index / 8);
    const count = Math.min(8, ids.length - ring * 8);
    const random = skyRandom(id);
    const angle = ((index % 8) / count) * Math.PI * 2 - Math.PI / 2 + ring * 0.24;
    const radius = 0.245 + ring * 0.175 + (random() - 0.5) * 0.013;
    points.set(id, { x: 0.5 + Math.cos(angle) * radius, y: 0.5 + Math.sin(angle) * radius });
  });
  return points;
}
