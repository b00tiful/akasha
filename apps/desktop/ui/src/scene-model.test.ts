import { describe, expect, it } from "vitest";

import {
  SCENE_HEIGHT,
  SCENE_WIDTH,
  SHELF_MOTION_BOUNDS,
  advanceShelfMotion,
  edgeShelfOrbitTarget,
  initialShelfMotion,
  majorEffectDelay,
  minorEffectDelay,
  orbitAroundGroundAxis,
  projectShelf,
  resolveShelfOverlaps,
  sealFrame,
  sealSymbol,
  spatialNeighbor,
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
    expect(degrees(edgeShelfOrbitTarget(1.18))).toBeCloseTo(52);
    expect(degrees(edgeShelfOrbitTarget(-1.18))).toBeCloseTo(-52);
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

  it("models the seal as a reversible circle-glow-crack-particle sequence", () => {
    expect(sealFrame(0)).toMatchObject({
      circleAlpha: 0,
      fragmentOffset: 0,
      fragmentAlpha: 1,
      contentReveal: 0,
      approach: 0,
    });
    expect(sealFrame(0.18).circleAlpha).toBeGreaterThan(0.95);
    expect(sealFrame(0.5).crackAlpha).toBeGreaterThan(0.8);
    expect(sealFrame(0.78).particleAlpha).toBeGreaterThan(0.5);
    expect(sealFrame(0.8).contentReveal).toBe(0);
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
    expect([minorEffectDelay(0), minorEffectDelay(1)]).toEqual([1.8, 3.8]);
    expect([majorEffectDelay(0), majorEffectDelay(1)]).toEqual([5, 9]);
  });
});

function degrees(radians: number): number {
  return radians * 180 / Math.PI;
}
