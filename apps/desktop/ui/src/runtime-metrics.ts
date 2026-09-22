export type RuntimeSurface = "local" | "global";

export interface RuntimeRenderInfo {
  width: number;
  height: number;
  renderScale: number;
  drawCalls: number;
  triangles: number;
  lines: number;
  points: number;
  geometries: number;
  textures: number;
  renderTargets: number;
  estimatedRenderTargetBytes: number;
}

export interface RuntimeSurfaceSnapshot {
  frames: number;
  intervalsMs: number[];
  cpuMs: number[];
  gpuMs: number[];
  mounts: number;
  destroys: number;
  active: number;
  render: RuntimeRenderInfo | null;
}

interface RuntimeSurfaceState extends RuntimeSurfaceSnapshot {
  lastFrameAt: number | null;
}

const SAMPLE_LIMIT = 4_096;

function emptySurface(): RuntimeSurfaceState {
  return {
    frames: 0,
    intervalsMs: [],
    cpuMs: [],
    gpuMs: [],
    mounts: 0,
    destroys: 0,
    active: 0,
    render: null,
    lastFrameAt: null,
  };
}

function appendSample(samples: number[], value: number): void {
  if (!Number.isFinite(value) || value < 0) return;
  if (samples.length === SAMPLE_LIMIT) samples.shift();
  samples.push(value);
}

export class RuntimeMetrics {
  readonly enabled: boolean;
  readonly #surfaces: Record<RuntimeSurface, RuntimeSurfaceState> = {
    local: emptySurface(),
    global: emptySurface(),
  };

  constructor(enabled: boolean) {
    this.enabled = enabled;
  }

  mount(surface: RuntimeSurface, render: RuntimeRenderInfo): void {
    if (!this.enabled) return;
    const state = this.#surfaces[surface];
    state.mounts++;
    state.active++;
    state.render = render;
  }

  destroy(surface: RuntimeSurface): void {
    if (!this.enabled) return;
    const state = this.#surfaces[surface];
    state.destroys++;
    state.active = Math.max(0, state.active - 1);
    state.lastFrameAt = null;
  }

  frame(
    surface: RuntimeSurface,
    timestamp: number,
    cpuMs: number,
    render?: Partial<RuntimeRenderInfo>,
  ): void {
    if (!this.enabled) return;
    const state = this.#surfaces[surface];
    if (state.lastFrameAt !== null) {
      appendSample(state.intervalsMs, timestamp - state.lastFrameAt);
    }
    state.lastFrameAt = timestamp;
    state.frames++;
    appendSample(state.cpuMs, cpuMs);
    if (render && state.render) Object.assign(state.render, render);
  }

  gpu(surface: RuntimeSurface, milliseconds: number): void {
    if (!this.enabled) return;
    appendSample(this.#surfaces[surface].gpuMs, milliseconds);
  }

  resetSamples(surface: RuntimeSurface): void {
    if (!this.enabled) return;
    const state = this.#surfaces[surface];
    state.frames = 0;
    state.intervalsMs = [];
    state.cpuMs = [];
    state.gpuMs = [];
    state.lastFrameAt = null;
  }

  snapshot(surface: RuntimeSurface): RuntimeSurfaceSnapshot {
    const state = this.#surfaces[surface];
    return {
      frames: state.frames,
      intervalsMs: [...state.intervalsMs],
      cpuMs: [...state.cpuMs],
      gpuMs: [...state.gpuMs],
      mounts: state.mounts,
      destroys: state.destroys,
      active: state.active,
      render: state.render ? { ...state.render } : null,
    };
  }
}

export interface SampleSummary {
  count: number;
  mean: number | null;
  median: number | null;
  p95: number | null;
  maximum: number | null;
}

export function summarizeSamples(samples: number[]): SampleSummary {
  if (samples.length === 0) {
    return { count: 0, mean: null, median: null, p95: null, maximum: null };
  }
  const sorted = [...samples].sort((a, b) => a - b);
  const percentile = (value: number): number =>
    sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * value) - 1)]!;
  return {
    count: sorted.length,
    mean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length,
    median: percentile(0.5),
    p95: percentile(0.95),
    maximum: sorted.at(-1)!,
  };
}

export const runtimeProbeEnabled = import.meta.env.VITE_AKASHA_RUNTIME_PROBE === "1";
export const runtimeMetrics = new RuntimeMetrics(runtimeProbeEnabled);
