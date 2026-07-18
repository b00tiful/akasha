import { describe, expect, it } from "vitest";

import {
  SCENE_HEIGHT,
  SCENE_WIDTH,
  SHELF_MOTION_BOUNDS,
  advanceShelfMotion,
  cabinetScaleForCount,
  edgeShelfOrbitTarget,
  initialShelfMotion,
  majorEffectDelay,
  minorEffectDelay,
  orbitAroundGroundAxis,
  projectShelf,
  resolveShelfOverlaps,
  sealFrame,
  sealSymbol,
  shelfRoamingCells,
  spatialNeighbor,
  steerShelfMotions,
  stellarDialTarget,
  topLeftShelf,
} from "./scene-model";

describe("global library scene model", () => {
  it("uses a fine integer-scaled 16:9 pixel grid", () => {
    expect([SCENE_WIDTH, SCENE_HEIGHT]).toEqual([1280, 720]);
    expect(SCENE_WIDTH / SCENE_HEIGHT).toBeCloseTo(16 / 9);
  });

  it("creates stable independent XYZ motion that remains inside the visible volume", () => {
    const first = initialShelfMotion("project:alpha", 0);
    expect(first).toEqual(initialShelfMotion("project:alpha", 0));
    const advanced = advanceShelfMotion(
      { ...first, x: 1.17, velocityX: 0.1 },
      1,
    );
    expect(advanced.x).toBeLessThanOrEqual(SHELF_MOTION_BOUNDS.maximumX);
    expect(advanced.velocityX).toBeLessThan(0);
    const projected = projectShelf("project:alpha", advanced);
    expect(projected.x).toBeGreaterThan(0);
    expect(projected.x).toBeLessThan(SCENE_WIDTH);
    expect(projected.y).toBeGreaterThan(0);
    expect(projected.y).toBeLessThan(SCENE_HEIGHT);
  });

  it("assigns each cabinet a narrow deterministic cell and scales density monotonically", () => {
    const cells = shelfRoamingCells(7);
    expect(cells).toHaveLength(7);
    cells.slice(0, 4).forEach((cell) => expect(cell.centerY).toBeCloseTo(0.31));
    cells.slice(4).forEach((cell) => expect(cell.centerY).toBeCloseTo(-0.31));
    expect(cells[0]?.centerX).toBeCloseTo(-0.885);
    expect(cells[3]?.centerX).toBeCloseTo(0.885);
    expect(cells[4]?.centerX).toBeCloseTo(-0.59);
    for (const cell of cells) {
      expect(cell.maximumX - cell.minimumX).toBeCloseTo(0.14);
      expect(cell.maximumY - cell.minimumY).toBeCloseTo(0.1);
    }
    expect(cabinetScaleForCount(2)).toBe(1);
    expect(cabinetScaleForCount(4)).toBeLessThan(cabinetScaleForCount(3));
    expect(cabinetScaleForCount(7)).toBeLessThan(cabinetScaleForCount(4));

    const motion = initialShelfMotion("mirror:global:3", 3, cells[3]);
    const advanced = advanceShelfMotion(
      { ...motion, x: cells[3]!.maximumX - 0.001, velocityX: 0.02 },
      1,
      cells[3],
    );
    expect(advanced.x).toBeLessThanOrEqual(cells[3]!.maximumX);
    expect(advanced.velocityX).toBeLessThan(0);
  });

  it("separates projected shelf footprints and starts orbit only near side edges", () => {
    const motions = [
      {
        x: 0,
        y: 0,
        z: 0.5,
        velocityX: 0.01,
        velocityY: 0,
        velocityZ: 0,
      },
      {
        x: 0.02,
        y: 0.01,
        z: 0.52,
        velocityX: -0.01,
        velocityY: 0,
        velocityZ: 0,
      },
    ];
    resolveShelfOverlaps(motions);
    const [left, right] = motions.map((motion, index) => projectShelf(String(index), motion));
    expect(
      Math.abs((right?.x ?? 0) - (left?.x ?? 0)) > 160 ||
        Math.abs((right?.y ?? 0) - (left?.y ?? 0)) > 190,
    ).toBe(true);
    expect(edgeShelfOrbitTarget(0.4)).toBe(0);
    expect(degrees(edgeShelfOrbitTarget(1.18))).toBeCloseTo(30);
    expect(degrees(edgeShelfOrbitTarget(-1.18))).toBeCloseTo(-30);
  });

  it("steers nearby shelves apart before applying bounded overlap correction", () => {
    const motions = [
      {
        x: -0.01,
        y: 0,
        z: 0.5,
        velocityX: 0.01,
        velocityY: 0.002,
        velocityZ: 0.001,
      },
      {
        x: 0.01,
        y: 0,
        z: 0.51,
        velocityX: -0.008,
        velocityY: -0.001,
        velocityZ: -0.001,
      },
    ];
    steerShelfMotions(motions, 0.05);
    expect(motions[0]?.velocityX).toBeLessThan(0.01);
    expect(motions[1]?.velocityX).toBeGreaterThan(-0.008);
    expect(motions[0]?.velocityZ).toBe(0.001);
    expect(motions[1]?.velocityZ).toBe(-0.001);
  });

  it("orbits shelves around the ground-circle axis instead of the seal axis", () => {
    const counterclockwise = orbitAroundGroundAxis(2, -1.6, Math.PI / 2);
    const clockwise = orbitAroundGroundAxis(2, -1.6, -Math.PI / 2);
    expect(counterclockwise.x).toBeCloseTo(0);
    expect(counterclockwise.z).toBeCloseTo(-3.6);
    expect(clockwise.x).toBeCloseTo(0);
    expect(clockwise.z).toBeCloseTo(0.4);
    expect(Math.hypot(counterclockwise.x, counterclockwise.z + 1.6)).toBeCloseTo(2);
  });

  it("starts from the upper-left shelf and navigates by screen direction", () => {
    const shelves = [
      { id: "upper-left", x: 120, y: 100, scale: 0.5, alpha: 1 },
      { id: "upper-right", x: 900, y: 110, scale: 0.5, alpha: 1 },
      { id: "lower-left", x: 150, y: 540, scale: 0.5, alpha: 1 },
    ];
    expect(topLeftShelf(shelves)).toBe("upper-left");
    expect(spatialNeighbor("upper-left", shelves, "right")).toBe("upper-right");
    expect(spatialNeighbor("upper-left", shelves, "down")).toBe("lower-left");
    expect(spatialNeighbor("upper-left", shelves, "left")).toBe("upper-right");
  });

  it("selects predefined semantic seals and a stable symmetric fallback", () => {
    expect(sealSymbol("Research archive", false).name).toBe("research");
    expect(sealSymbol("Daily sessions", false).name).toBe("sessions");
    expect(sealSymbol("Any project", true).name).toBe("global archive");

    const first = sealSymbol("AeroStrike", false);
    const second = sealSymbol("AeroStrike", false);
    expect(first).toEqual(second);
    expect(first.name).toBe("flight");

    const fallback = sealSymbol("Example", false);
    expect(fallback.name).toBe("deterministic rune");
    for (const row of fallback.rows) {
      expect(row).toHaveLength(9);
      expect(row).toBe([...row].reverse().join(""));
    }
  });

  it("models the seal as a reversible trace-collapse-energy sequence", () => {
    expect(sealFrame(0)).toMatchObject({
      circleAlpha: 0,
      fragmentOffset: 0,
      fragmentAlpha: 1,
      contentReveal: 0,
      approach: 0,
    });
    expect(sealFrame(0.2).circleAlpha).toBeGreaterThan(0.85);
    expect(sealFrame(0.34).crackAlpha).toBeGreaterThan(0.8);
    expect(sealFrame(0.68).particleAlpha).toBeGreaterThan(0.8);
    expect(sealFrame(0.46).contentReveal).toBe(0);
    expect(sealFrame(0.82).contentReveal).toBe(1);
    expect(sealFrame(1)).toMatchObject({
      circleAlpha: 0,
      fragmentAlpha: 0,
      particleAlpha: 0,
      contentReveal: 1,
      approach: 1,
    });
    expect(stellarDialTarget(0.984, true)).toBe(0);
    expect(stellarDialTarget(0.985, true)).toBe(1);
    expect(stellarDialTarget(1, false)).toBe(0);
  });

  it("bounds minor and major random-event cadence", () => {
    expect([minorEffectDelay(0), minorEffectDelay(1)]).toEqual([8, 15]);
    expect([majorEffectDelay(0), majorEffectDelay(1)]).toEqual([24, 42]);
  });
});

function degrees(radians: number): number {
  return radians * 180 / Math.PI;
}
