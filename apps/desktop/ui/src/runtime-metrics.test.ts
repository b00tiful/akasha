import { describe, expect, it } from "vitest";
import { RuntimeMetrics, summarizeSamples } from "./runtime-metrics";

describe("runtime metrics", () => {
  it("records bounded frame, GPU and lifecycle evidence only when enabled", () => {
    const render = {
      width: 2560,
      height: 1440,
      renderScale: 2,
      drawCalls: 0,
      triangles: 0,
      lines: 0,
      points: 0,
      geometries: 0,
      textures: 0,
      renderTargets: 1,
      estimatedRenderTargetBytes: 2560 * 1440 * 8,
    };
    const mountTiming = {
      fontWaitMs: 1,
      rendererMs: 2,
      ambientMs: 3,
      cabinetsMs: 4,
      wiringMs: 5,
      firstDrawMs: 6,
      totalMs: 21,
    };
    const metrics = new RuntimeMetrics(true);
    metrics.mount("global", render);
    metrics.mountTiming("global", mountTiming);
    metrics.frame("global", 10, 2.1, { drawCalls: 50 }, { updateMs: 0.7, renderMs: 1.4 });
    metrics.frame("global", 16, 2.4, { triangles: 12_000 }, { updateMs: 0.8, renderMs: 1.6 });
    metrics.gpu("global", 3.2);
    metrics.destroy("global");

    expect(metrics.snapshot("global")).toMatchObject({
      frames: 2,
      intervalsMs: [6],
      cpuMs: [2.1, 2.4],
      updateCpuMs: [0.7, 0.8],
      renderCpuMs: [1.4, 1.6],
      gpuMs: [3.2],
      mountTimings: [mountTiming],
      mounts: 1,
      destroys: 1,
      active: 0,
      render: { drawCalls: 50, triangles: 12_000 },
    });

    const disabled = new RuntimeMetrics(false);
    disabled.mount("global", render);
    disabled.mountTiming("global", mountTiming);
    disabled.frame("global", 10, 2.1, { drawCalls: 50 }, { updateMs: 0.7, renderMs: 1.4 });
    disabled.gpu("global", 3.2);
    disabled.destroy("global");
    expect(disabled.snapshot("global")).toMatchObject({
      frames: 0,
      intervalsMs: [],
      cpuMs: [],
      updateCpuMs: [],
      renderCpuMs: [],
      gpuMs: [],
      mountTimings: [],
      mounts: 0,
      destroys: 0,
      active: 0,
      render: null,
    });
  });

  it("resets samples without erasing lifecycle or render-target evidence", () => {
    const metrics = new RuntimeMetrics(true);
    metrics.mount("local", {
      width: 640,
      height: 360,
      renderScale: 1,
      drawCalls: 1,
      triangles: 0,
      lines: 0,
      points: 0,
      geometries: 0,
      textures: 3,
      renderTargets: 0,
      estimatedRenderTargetBytes: 0,
    });
    metrics.frame("local", 10, 1);
    metrics.frame("local", 44, 2);
    metrics.mountTiming("local", {
      fontWaitMs: 0,
      rendererMs: 1,
      ambientMs: 0,
      cabinetsMs: 0,
      wiringMs: 0,
      firstDrawMs: 0,
      totalMs: 1,
    });
    metrics.resetSamples("local");
    metrics.frame("local", 100, 3);

    expect(metrics.snapshot("local")).toMatchObject({
      frames: 1,
      intervalsMs: [],
      cpuMs: [3],
      updateCpuMs: [],
      renderCpuMs: [],
      mountTimings: [{ totalMs: 1 }],
      mounts: 1,
      active: 1,
      render: { width: 640, height: 360 },
    });
  });

  it("summarizes empty and ordered percentile evidence deterministically", () => {
    expect(summarizeSamples([])).toEqual({
      count: 0,
      mean: null,
      median: null,
      p95: null,
      maximum: null,
    });
    expect(summarizeSamples([10, 2, 8, 4, 6])).toEqual({
      count: 5,
      mean: 6,
      median: 6,
      p95: 10,
      maximum: 10,
    });
  });
});
