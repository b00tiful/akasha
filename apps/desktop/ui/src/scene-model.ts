export const SCENE_WIDTH = 1280;
export const SCENE_HEIGHT = 720;

export const SHELF_MOTION_BOUNDS = {
  minimumX: -1.18,
  maximumX: 1.18,
  minimumY: -0.62,
  maximumY: 0.62,
  minimumZ: 0.34,
  maximumZ: 1.08,
} as const;

export type SpatialDirection = "left" | "right" | "up" | "down";

export interface ShelfMotion {
  x: number;
  y: number;
  z: number;
  velocityX: number;
  velocityY: number;
  velocityZ: number;
}

export interface ShelfRoamingCell {
  minimumX: number;
  maximumX: number;
  minimumY: number;
  maximumY: number;
  minimumZ: number;
  maximumZ: number;
  centerX: number;
  centerY: number;
}

export interface ProjectedShelf {
  id: string;
  x: number;
  y: number;
  scale: number;
  alpha: number;
}

export interface PixelSymbol {
  name: string;
  rows: readonly string[];
}

export interface SealFrame {
  circleAlpha: number;
  glowAlpha: number;
  crackAlpha: number;
  fragmentOffset: number;
  fragmentAlpha: number;
  particleAlpha: number;
  contentReveal: number;
  approach: number;
}

const SYMBOLS: ReadonlyArray<{
  name: string;
  matches: readonly string[];
  rows: readonly string[];
}> = [
  {
    name: "research",
    matches: ["research", "knowledge", "study", "memory", "akasha"],
    rows: [
      "....#....",
      "...###...",
      "..#.#.#..",
      ".#..#..#.",
      "#...#...#",
      ".#..#..#.",
      "..#####..",
      "...#.#...",
      "....#....",
    ],
  },
  {
    name: "sessions",
    matches: ["session", "sessions", "handoff", "history"],
    rows: [
      "...###...",
      "..#...#..",
      ".#..#..#.",
      ".#.....#.",
      ".#..##.#.",
      ".#....#..",
      "..#...#..",
      "...###...",
      "....#....",
    ],
  },
  {
    name: "logs",
    matches: ["log", "logs", "event", "events", "chronicle"],
    rows: [
      "....#....",
      "...#.#...",
      "..#...#..",
      ".#..#..#.",
      ".#.#.#.#.",
      ".#..#..#.",
      "..#...#..",
      "...###...",
      "....#....",
    ],
  },
  {
    name: "ideas",
    matches: ["idea", "ideas", "concept", "concepts"],
    rows: [
      "....#....",
      "...#.#...",
      "..#...#..",
      ".#.#.#.#.",
      ".#..#..#.",
      "..#...#..",
      "...###...",
      "..#####..",
      "...###...",
    ],
  },
  {
    name: "problems",
    matches: ["problem", "problems", "issue", "issues", "blocker"],
    rows: [
      "#...#...#",
      ".#..#..#.",
      "..#####..",
      ".#.....#.",
      "#..###..#",
      ".#.....#.",
      "..#####..",
      "...#.#...",
      "....#....",
    ],
  },
  {
    name: "tasks",
    matches: ["task", "tasks", "roadmap", "milestone", "milestones"],
    rows: [
      "....#....",
      "...###...",
      "..#.#.#..",
      ".#..#..#.",
      "#...#...#",
      "....#....",
      "...###...",
      "..#####..",
      "....#....",
    ],
  },
  {
    name: "entities",
    matches: ["entity", "entities", "people", "person"],
    rows: [
      "...###...",
      "..#...#..",
      "..#...#..",
      "...###...",
      "....#....",
      "..#####..",
      ".#..#..#.",
      "...#.#...",
      "..#...#..",
    ],
  },
  {
    name: "architecture",
    matches: ["architecture", "design", "system"],
    rows: [
      "....#....",
      "...###...",
      "..#.#.#..",
      ".#..#..#.",
      "#########",
      ".#..#..#.",
      ".#..#..#.",
      ".#..#..#.",
      ".##.#.##.",
    ],
  },
  {
    name: "flight",
    matches: ["aero", "flight", "drone", "navigation", "sky"],
    rows: [
      "....#....",
      "...###...",
      "#..###..#",
      ".#######.",
      "..#####..",
      "...###...",
      "..#.#.#..",
      ".#..#..#.",
      "....#....",
    ],
  },
  {
    name: "code",
    matches: ["code", "software", "algorithm", "data"],
    rows: [
      "#...#...#",
      ".#.....#.",
      "..#.#.#..",
      "...#.#...",
      "....#....",
      "...#.#...",
      "..#.#.#..",
      ".#.....#.",
      "#...#...#",
    ],
  },
];

const GLOBAL_SYMBOL: PixelSymbol = {
  name: "global archive",
  rows: [
    "....#....",
    "...###...",
    "....#....",
    ".##...##.",
    "#..#.#..#",
    "#..#.#..#",
    "#..#.#..#",
    ".###.###.",
    "...#.#...",
  ],
};

export function sealSymbol(label: string, global: boolean): PixelSymbol {
  if (global) {
    return GLOBAL_SYMBOL;
  }
  const normalized = label.toLowerCase();
  const predefined = SYMBOLS.find((symbol) =>
    symbol.matches.some((match) => normalized.includes(match)),
  );
  if (predefined) {
    return { name: predefined.name, rows: predefined.rows };
  }
  return { name: "deterministic rune", rows: fallbackRune(label) };
}

export function shelfRoamingCells(count: number): ShelfRoamingCell[] {
  const cabinetCount = Math.max(1, Math.floor(count));
  const columns = Math.min(
    cabinetCount,
    Math.max(1, Math.ceil(Math.sqrt(cabinetCount * 2))),
  );
  const rows = Math.ceil(cabinetCount / columns);
  const totalWidth = SHELF_MOTION_BOUNDS.maximumX - SHELF_MOTION_BOUNDS.minimumX;
  const totalHeight = SHELF_MOTION_BOUNDS.maximumY - SHELF_MOTION_BOUNDS.minimumY;
  const slotWidth = totalWidth / columns;
  const slotHeight = totalHeight / rows;
  const cells: ShelfRoamingCell[] = [];
  for (let row = 0; row < rows; row += 1) {
    const rowStart = row * columns;
    const rowCount = Math.min(columns, cabinetCount - rowStart);
    for (let column = 0; column < rowCount; column += 1) {
      const centerX = (column - (rowCount - 1) / 2) * slotWidth;
      const centerY = SHELF_MOTION_BOUNDS.maximumY - (row + 0.5) * slotHeight;
      const roamX = Math.min(0.07, slotWidth * 0.13);
      const roamY = Math.min(0.05, slotHeight * 0.13);
      const depthRow = rows === 1 ? 0.5 : row / (rows - 1);
      const depthCenter = 0.74 - depthRow * 0.16 + (column % 2) * 0.025;
      cells.push({
        minimumX: centerX - roamX,
        maximumX: centerX + roamX,
        minimumY: centerY - roamY,
        maximumY: centerY + roamY,
        minimumZ: depthCenter - 0.035,
        maximumZ: depthCenter + 0.035,
        centerX,
        centerY,
      });
    }
  }
  return cells;
}

export function cabinetScaleForCount(count: number): number {
  return clamp(Math.sqrt(2 / Math.max(2, count)), 0.48, 1);
}

export function initialShelfMotion(
  id: string,
  index: number,
  cell?: ShelfRoamingCell,
): ShelfMotion {
  let state = hashString(`${id}:${index}`);
  const next = (): number => {
    state = nextHash(state);
    return state / 0xffff_ffff;
  };
  const speed = cell ? 0.0045 + next() * 0.0045 : 0.014 + next() * 0.012;
  const angle = next() * Math.PI * 2;
  const bounds = cell ?? SHELF_MOTION_BOUNDS;
  return {
    x: bounds.minimumX + (bounds.maximumX - bounds.minimumX) * next(),
    y: bounds.minimumY + (bounds.maximumY - bounds.minimumY) * next(),
    z: bounds.minimumZ + (bounds.maximumZ - bounds.minimumZ) * next(),
    velocityX: Math.cos(angle) * speed,
    velocityY: Math.sin(angle) * speed * 0.72,
    velocityZ: (next() - 0.5) * (cell ? 0.006 : 0.014),
  };
}

export function advanceShelfMotion(
  motion: ShelfMotion,
  seconds: number,
  cell?: ShelfRoamingCell,
): ShelfMotion {
  const next = { ...motion };
  const bounds = cell ?? SHELF_MOTION_BOUNDS;
  next.x += next.velocityX * seconds;
  next.y += next.velocityY * seconds;
  next.z += next.velocityZ * seconds;
  bounce(
    next,
    "x",
    "velocityX",
    bounds.minimumX,
    bounds.maximumX,
  );
  bounce(
    next,
    "y",
    "velocityY",
    bounds.minimumY,
    bounds.maximumY,
  );
  bounce(
    next,
    "z",
    "velocityZ",
    bounds.minimumZ,
    bounds.maximumZ,
  );
  return next;
}

export function projectShelf(id: string, motion: ShelfMotion): ProjectedShelf {
  const depthScale = 1 - motion.z;
  return {
    id,
    x: SCENE_WIDTH / 2 + motion.x * (470 + depthScale * 72),
    y: 305 + motion.y * (235 + depthScale * 45) - motion.z * 46,
    scale: 0.38 + depthScale * 0.36,
    alpha: 0.5 + depthScale * 0.46,
  };
}

export function resolveShelfOverlaps(
  motions: ShelfMotion[],
  maximumCorrection = Number.POSITIVE_INFINITY,
): void {
  for (let pass = 0; pass < 5; pass += 1) {
    for (let leftIndex = 0; leftIndex < motions.length; leftIndex += 1) {
      const left = motions[leftIndex];
      if (!left) {
        continue;
      }
      for (let rightIndex = leftIndex + 1; rightIndex < motions.length; rightIndex += 1) {
        const right = motions[rightIndex];
        if (!right) {
          continue;
        }
        separateProjectedPair(
          left,
          right,
          leftIndex,
          rightIndex,
          maximumCorrection,
        );
      }
    }
  }
}

export function steerShelfMotions(
  motions: ShelfMotion[],
  seconds: number,
): void {
  const delta = clamp(seconds, 0, 0.05);
  if (delta === 0 || motions.length < 2) {
    return;
  }
  const steering = motions.map(() => ({ x: 0, y: 0 }));
  for (let leftIndex = 0; leftIndex < motions.length; leftIndex += 1) {
    const left = motions[leftIndex];
    if (!left) {
      continue;
    }
    const leftProjection = projectShelf(String(leftIndex), left);
    for (let rightIndex = leftIndex + 1; rightIndex < motions.length; rightIndex += 1) {
      const right = motions[rightIndex];
      if (!right) {
        continue;
      }
      const rightProjection = projectShelf(String(rightIndex), right);
      let deltaX = rightProjection.x - leftProjection.x;
      let deltaY = rightProjection.y - leftProjection.y;
      if (Math.abs(deltaX) + Math.abs(deltaY) < 0.001) {
        deltaX = (leftIndex + rightIndex) % 2 === 0 ? 1 : -1;
        deltaY = leftIndex % 2 === 0 ? 0.35 : -0.35;
      }
      const scaleSum = leftProjection.scale + rightProjection.scale;
      const radiusX = 190 * scaleSum;
      const radiusY = 226 * scaleSum;
      const distance = Math.hypot(deltaX / radiusX, deltaY / radiusY);
      const influence = 1.42;
      if (distance >= influence) {
        continue;
      }
      const urgency = Math.pow((influence - distance) / influence, 1.35);
      const directionLength = Math.max(Math.hypot(deltaX / radiusX, deltaY / radiusY), 0.001);
      const directionX = deltaX / radiusX / directionLength;
      const directionY = deltaY / radiusY / directionLength;
      const horizontal = directionX * urgency * 0.022;
      const vertical = directionY * urgency * 0.015;
      steering[leftIndex]!.x -= horizontal;
      steering[leftIndex]!.y -= vertical;
      steering[rightIndex]!.x += horizontal;
      steering[rightIndex]!.y += vertical;
    }
  }
  motions.forEach((motion, index) => {
    const adjustment = steering[index];
    if (!adjustment) {
      return;
    }
    motion.velocityX = clamp(motion.velocityX + adjustment.x * delta, -0.042, 0.042);
    motion.velocityY = clamp(motion.velocityY + adjustment.y * delta, -0.034, 0.034);
  });
}

export function edgeShelfOrbitTarget(x: number): number {
  const edge = smoothstep(0.7, SHELF_MOTION_BOUNDS.maximumX, Math.abs(x));
  if (edge === 0) {
    return 0;
  }
  return Math.sign(x) * edge * (30 * Math.PI / 180);
}

export function orbitAroundGroundAxis(
  x: number,
  z: number,
  angle: number,
  centerX = 0,
  centerZ = -1.6,
): { x: number; z: number } {
  const relativeX = x - centerX;
  const relativeZ = z - centerZ;
  const cosine = Math.cos(angle);
  const sine = Math.sin(angle);
  return {
    x: centerX + relativeX * cosine + relativeZ * sine,
    z: centerZ - relativeX * sine + relativeZ * cosine,
  };
}

export function topLeftShelf(shelves: readonly ProjectedShelf[]): string | null {
  const first = [...shelves].sort((left, right) => {
    const vertical = left.y - right.y;
    return Math.abs(vertical) > 48 ? vertical : left.x - right.x;
  })[0];
  return first?.id ?? null;
}

export function spatialNeighbor(
  currentId: string,
  shelves: readonly ProjectedShelf[],
  direction: SpatialDirection,
): string | null {
  const current = shelves.find((shelf) => shelf.id === currentId);
  if (!current || shelves.length < 2) {
    return null;
  }
  const axis = direction === "left" || direction === "right" ? "x" : "y";
  const sign = direction === "left" || direction === "up" ? -1 : 1;
  const candidates = shelves
    .filter((shelf) => shelf.id !== currentId)
    .map((shelf) => {
      const deltaX = shelf.x - current.x;
      const deltaY = shelf.y - current.y;
      const primary = (axis === "x" ? deltaX : deltaY) * sign;
      const perpendicular = Math.abs(axis === "x" ? deltaY : deltaX);
      return {
        shelf,
        primary,
        score: Math.hypot(deltaX, deltaY) + perpendicular * 1.35,
      };
    });
  const forward = candidates
    .filter((candidate) => candidate.primary > 8)
    .sort((left, right) => left.score - right.score)[0];
  if (forward) {
    return forward.shelf.id;
  }
  const wrapped = [...candidates].sort((left, right) => {
    const leftAxis = axis === "x" ? left.shelf.x : left.shelf.y;
    const rightAxis = axis === "x" ? right.shelf.x : right.shelf.y;
    return sign < 0 ? rightAxis - leftAxis : leftAxis - rightAxis;
  })[0];
  return wrapped?.shelf.id ?? null;
}

export function sealFrame(progress: number): SealFrame {
  const value = clamp(progress, 0, 1);
  const circlePosition = smoothstep(0.06, 0.36, value);
  const circleAlpha = circlePosition === 0 || circlePosition === 1
    ? 0
    : Math.sin(circlePosition * Math.PI);
  const glowAlpha =
    smoothstep(0.04, 0.3, value) * (1 - smoothstep(0.76, 0.98, value));
  const crackAlpha =
    smoothstep(0.18, 0.36, value) * (1 - smoothstep(0.5, 0.7, value));
  const particlePosition = smoothstep(0.36, 0.96, value);
  return {
    circleAlpha,
    glowAlpha,
    crackAlpha,
    fragmentOffset: smoothstep(0.28, 0.72, value) * 7,
    fragmentAlpha: 1 - smoothstep(0.42, 0.76, value),
    particleAlpha:
      particlePosition === 0 || particlePosition === 1
        ? 0
        : Math.sin(particlePosition * Math.PI),
    contentReveal: smoothstep(0.48, 0.82, value),
    approach: smoothstep(0.12, 1, value),
  };
}

export function stellarDialTarget(
  sealProgress: number,
  shelfActive: boolean,
): 0 | 1 {
  return shelfActive && sealProgress >= 0.985 ? 1 : 0;
}

export function minorEffectDelay(random: number): number {
  return 8 + clamp(random, 0, 1) * 7;
}

export function majorEffectDelay(random: number): number {
  return 24 + clamp(random, 0, 1) * 18;
}

function fallbackRune(label: string): readonly string[] {
  let state = hashString(label || "akasha");
  const grid = Array.from({ length: 9 }, () => Array.from({ length: 9 }, () => "."));
  for (let row = 0; row < 9; row += 1) {
    grid[row]![4] = row === 0 || row === 4 || row === 8 ? "#" : ".";
    for (let column = 1; column <= 3; column += 1) {
      state = nextHash(state);
      if ((state & 7) > 3 && (row + column) % 2 === 0) {
        grid[row]![column] = "#";
        grid[row]![8 - column] = "#";
      }
    }
  }
  grid[1]![3] = "#";
  grid[1]![5] = "#";
  grid[2]![2] = "#";
  grid[2]![6] = "#";
  grid[6]![2] = "#";
  grid[6]![6] = "#";
  grid[7]![3] = "#";
  grid[7]![5] = "#";
  grid[0]![4] = "#";
  grid[4]![4] = "#";
  grid[8]![4] = "#";
  return grid.map((row) => row.join(""));
}

function separateProjectedPair(
  left: ShelfMotion,
  right: ShelfMotion,
  leftIndex: number,
  rightIndex: number,
  maximumCorrection: number,
): void {
  const leftProjection = projectShelf(String(leftIndex), left);
  const rightProjection = projectShelf(String(rightIndex), right);
  const deltaX = rightProjection.x - leftProjection.x;
  const deltaY = rightProjection.y - leftProjection.y;
  const scaleSum = leftProjection.scale + rightProjection.scale;
  const requiredX = 166 * scaleSum;
  const requiredY = 202 * scaleSum;
  const overlapX = requiredX - Math.abs(deltaX);
  const overlapY = requiredY - Math.abs(deltaY);
  if (overlapX <= 0 || overlapY <= 0) {
    return;
  }

  if (overlapX / requiredX <= overlapY / requiredY) {
    const direction = deltaX === 0 ? (leftIndex + rightIndex) % 2 === 0 ? 1 : -1 : Math.sign(deltaX);
    const push = Math.min(overlapX / 1010 + 0.006, maximumCorrection);
    left.x = clamp(
      left.x - direction * push,
      SHELF_MOTION_BOUNDS.minimumX,
      SHELF_MOTION_BOUNDS.maximumX,
    );
    right.x = clamp(
      right.x + direction * push,
      SHELF_MOTION_BOUNDS.minimumX,
      SHELF_MOTION_BOUNDS.maximumX,
    );
    left.velocityX = -direction * Math.max(Math.abs(left.velocityX), 0.008);
    right.velocityX = direction * Math.max(Math.abs(right.velocityX), 0.008);
    return;
  }

  const direction = deltaY === 0 ? (leftIndex + rightIndex) % 2 === 0 ? 1 : -1 : Math.sign(deltaY);
  const push = Math.min(overlapY / 510 + 0.008, maximumCorrection);
  left.y = clamp(
    left.y - direction * push,
    SHELF_MOTION_BOUNDS.minimumY,
    SHELF_MOTION_BOUNDS.maximumY,
  );
  right.y = clamp(
    right.y + direction * push,
    SHELF_MOTION_BOUNDS.minimumY,
    SHELF_MOTION_BOUNDS.maximumY,
  );
  left.velocityY = -direction * Math.max(Math.abs(left.velocityY), 0.006);
  right.velocityY = direction * Math.max(Math.abs(right.velocityY), 0.006);
}

function hashString(value: string): number {
  let hash = 0x811c9dc5;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

function nextHash(value: number): number {
  return (Math.imul(value, 1_664_525) + 1_013_904_223) >>> 0;
}

function bounce(
  motion: ShelfMotion,
  position: "x" | "y" | "z",
  velocity: "velocityX" | "velocityY" | "velocityZ",
  minimum: number,
  maximum: number,
): void {
  if (motion[position] < minimum) {
    motion[position] = minimum + (minimum - motion[position]);
    motion[velocity] = Math.abs(motion[velocity]);
  } else if (motion[position] > maximum) {
    motion[position] = maximum - (motion[position] - maximum);
    motion[velocity] = -Math.abs(motion[velocity]);
  }
}

function smoothstep(start: number, end: number, value: number): number {
  const position = clamp((value - start) / (end - start), 0, 1);
  return position * position * (3 - 2 * position);
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
