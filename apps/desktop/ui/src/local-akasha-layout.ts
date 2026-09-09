export interface SkyPoint { x: number; y: number }
export const ORBIT_PAGE_SIZE = 16;
export const SKY_PAGE_SIZE = 12;

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

// Seeded, stratified placement reserves label space. Paging bounds density without hiding sections.
export function sectionLayout(key: string, ids: string[]): Map<string, SkyPoint> {
  const slots = [4, 2, 6, 0, 9, 7, 1, 11, 8, 5, 3, 10];
  const points = new Map<string, SkyPoint>();
  [...ids].sort().forEach((id, index) => {
    const slot = slots[index % SKY_PAGE_SIZE]!;
    const random = skyRandom(`${key}/${id}`);
    points.set(id, {
      x: 0.13 + (slot % 4) * 0.235 + (random() - 0.5) * 0.045,
      y: 0.21 + Math.floor(slot / 4) * 0.275 + (random() - 0.5) * 0.065,
    });
  });
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
