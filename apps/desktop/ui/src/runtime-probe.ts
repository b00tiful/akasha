import { invoke } from "@tauri-apps/api/core";

import {
  runtimeMetrics,
  runtimeProbeEnabled,
  summarizeSamples,
  type RuntimeRenderInfo,
  type RuntimeSurface,
  type SampleSummary,
} from "./runtime-metrics";

interface RuntimeCheck {
  name: string;
  pass: boolean;
  actual: number | string | boolean | null;
  expected: string;
}

interface SurfaceReport {
  frames: number;
  refreshHz: number | null;
  intervalMs: SampleSummary;
  cpuMs: SampleSummary;
  gpuMs: SampleSummary;
  render: RuntimeRenderInfo | null;
}

interface RuntimeReport {
  version: 1;
  passed: boolean;
  viewport: { width: number; height: number; screenWidth: number; screenHeight: number; dpr: number };
  local: SurfaceReport;
  globalIdle: SurfaceReport;
  globalTransition: SurfaceReport;
  interactions: {
    localTransitionMs: number;
    noteOpenMs: number;
    firstGlobalSwitchMs: number;
    soakSwitchMs: SampleSummary;
  };
  lifecycle: {
    local: { mounts: number; destroys: number; active: number };
    global: { mounts: number; destroys: number; active: number };
    canvases: number;
    webglCanvases: number;
  };
  eventLoopDelayMs: SampleSummary;
  longTasks: { supported: boolean; durationsMs: SampleSummary };
  checks: RuntimeCheck[];
  error?: string;
}

let started = false;

const sleep = (milliseconds: number): Promise<void> =>
  new Promise((resolve) => window.setTimeout(resolve, milliseconds));

async function waitFor(test: () => boolean, label: string, timeout = 8_000): Promise<void> {
  const startedAt = performance.now();
  while (!test()) {
    if (performance.now() - startedAt > timeout) throw new Error(`timed out waiting for ${label}`);
    await sleep(25);
  }
}

function rounded(value: number | null): number | null {
  return value === null ? null : Math.round(value * 100) / 100;
}

function roundedSummary(summary: SampleSummary): SampleSummary {
  return {
    count: summary.count,
    mean: rounded(summary.mean),
    median: rounded(summary.median),
    p95: rounded(summary.p95),
    maximum: rounded(summary.maximum),
  };
}

function surfaceReport(surface: RuntimeSurface): SurfaceReport {
  const snapshot = runtimeMetrics.snapshot(surface);
  const intervalMs = roundedSummary(summarizeSamples(snapshot.intervalsMs));
  return {
    frames: snapshot.frames,
    refreshHz: intervalMs.median === null ? null : rounded(1_000 / intervalMs.median),
    intervalMs,
    cpuMs: roundedSummary(summarizeSamples(snapshot.cpuMs)),
    gpuMs: roundedSummary(summarizeSamples(snapshot.gpuMs)),
    render: snapshot.render,
  };
}

async function publish(report: RuntimeReport): Promise<void> {
  const json = JSON.stringify(report);
  const marker = report.passed ? "PASS" : "FAIL";
  document.title = `AKASHA_RUNTIME_${marker}`;
  console.info("AKASHA_RUNTIME_REPORT", json);
  await invoke("write_runtime_probe_report", { report: json });
}

function click<T extends HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`runtime probe could not find ${selector}`);
  element.click();
  return element;
}

function check(
  checks: RuntimeCheck[],
  name: string,
  pass: boolean,
  actual: RuntimeCheck["actual"],
  expected: string,
): void {
  checks.push({ name, pass, actual: typeof actual === "number" ? rounded(actual) : actual, expected });
}

export function scheduleRuntimeProbe(): void {
  if (!runtimeProbeEnabled || started) return;
  started = true;
  document.title = "AKASHA_RUNTIME_RUNNING";
  window.setTimeout(() => void runRuntimeProbe(), 0);
}

async function runRuntimeProbe(): Promise<void> {
  const checks: RuntimeCheck[] = [];
  const eventLoopDelays: number[] = [];
  const longTasks: number[] = [];
  let local = surfaceReport("local");
  let globalIdle = surfaceReport("global");
  let globalTransition = surfaceReport("global");
  let localTransitionMs = 0;
  let noteOpenMs = 0;
  let firstGlobalSwitchMs = 0;
  const soakSwitches: number[] = [];
  let lastInterval = performance.now();
  const interval = window.setInterval(() => {
    const now = performance.now();
    eventLoopDelays.push(Math.max(0, now - lastInterval - 50));
    lastInterval = now;
  }, 50);
  let longTaskSupported = false;
  let observer: PerformanceObserver | null = null;
  try {
    if (PerformanceObserver.supportedEntryTypes.includes("longtask")) {
      longTaskSupported = true;
      observer = new PerformanceObserver((entries) => {
        for (const entry of entries.getEntries()) longTasks.push(entry.duration);
      });
      observer.observe({ entryTypes: ["longtask"] });
    }
  } catch {
    longTaskSupported = false;
  }

  try {
    await waitFor(() => document.querySelector(".local-akasha") !== null, "Local Akasha startup");
    const reduced = document.querySelector<HTMLInputElement>("#reduced-motion")!;
    reduced.checked = false;
    reduced.dispatchEvent(new Event("change"));

    await sleep(1_500);
    runtimeMetrics.resetSamples("local");
    await sleep(5_000);
    local = surfaceReport("local");
    check(checks, "local frame count", local.frames >= 100, local.frames, ">= 100 over five seconds");
    check(checks, "local cadence p95", (local.intervalMs.p95 ?? Infinity) <= 50, local.intervalMs.p95, "<= 50 ms");
    check(checks, "local draw CPU p95", (local.cpuMs.p95 ?? Infinity) <= 8, local.cpuMs.p95, "<= 8 ms");

    const beforeReduced = runtimeMetrics.snapshot("local").frames;
    reduced.checked = true;
    reduced.dispatchEvent(new Event("change"));
    await sleep(450);
    const afterReduced = runtimeMetrics.snapshot("local").frames;
    check(checks, "local reduced-motion pause", afterReduced === beforeReduced, afterReduced - beforeReduced, "0 frames");
    reduced.checked = false;
    reduced.dispatchEvent(new Event("change"));
    await sleep(250);
    check(
      checks,
      "local reduced-motion resume",
      runtimeMetrics.snapshot("local").frames > afterReduced,
      runtimeMetrics.snapshot("local").frames - afterReduced,
      "> 0 frames",
    );

    const sections = [...document.querySelectorAll<HTMLButtonElement>(".local-section")];
    const section = sections.find((candidate) => !candidate.getAttribute("aria-label")?.includes(", 0 notes")) ?? sections[0];
    if (!section) throw new Error("runtime probe found no configured Local Akasha section");
    if (document.querySelector(".local-akasha")?.getAttribute("data-phase") !== "directory") {
      throw new Error("runtime probe did not start from the isolated local directory");
    }
    const transitionStarted = performance.now();
    section.click();
    if (document.querySelector(".local-akasha")?.getAttribute("data-phase") !== "entering-section") {
      throw new Error("runtime probe did not begin the local section transition");
    }
    await waitFor(
      () => document.querySelector(".local-akasha")?.getAttribute("data-phase") === "section",
      "local section transition",
      3_000,
    );
    localTransitionMs = performance.now() - transitionStarted;
    check(checks, "local transition duration", localTransitionMs >= 1_200 && localTransitionMs <= 1_750,
      localTransitionMs, "1,200–1,750 ms around the authored 1,350 ms transition");

    const note = document.querySelector<HTMLButtonElement>(".local-note");
    if (!note) throw new Error("runtime probe selected a section without a note");
    const noteStarted = performance.now();
    note.click();
    await waitFor(
      () => !document.querySelector<HTMLElement>("#note-overlay")?.hidden &&
        document.querySelector("#note-viewer .cm-editor") !== null,
      "checked note reader",
      3_000,
    );
    noteOpenMs = performance.now() - noteStarted;
    check(checks, "note open latency", noteOpenMs <= 1_000, noteOpenMs, "<= 1,000 ms");
    const beforeReader = runtimeMetrics.snapshot("local").frames;
    await sleep(450);
    check(checks, "reader pauses local canvas", runtimeMetrics.snapshot("local").frames === beforeReader,
      runtimeMetrics.snapshot("local").frames - beforeReader, "0 frames");
    click("#note-close");
    await waitFor(() => document.querySelector<HTMLElement>("#note-overlay")?.hidden === true, "note close");

    const firstGlobalStarted = performance.now();
    click("#scope-toggle");
    await waitFor(
      () => document.querySelector(".world-canvas") !== null &&
        !document.querySelector(".pixel-stage")?.classList.contains("is-local"),
      "global scene",
      12_000,
    );
    firstGlobalSwitchMs = performance.now() - firstGlobalStarted;
    check(checks, "first global scene mount", firstGlobalSwitchMs <= 5_000, firstGlobalSwitchMs, "<= 5,000 ms");

    await sleep(3_000);
    runtimeMetrics.resetSamples("global");
    await sleep(8_000);
    globalIdle = surfaceReport("global");
    check(checks, "global native cadence", (globalIdle.refreshHz ?? 0) >= 150, globalIdle.refreshHz,
      ">= 150 Hz on the 165 Hz target display");
    check(checks, "global cadence p95", (globalIdle.intervalMs.p95 ?? Infinity) <= 12.5,
      globalIdle.intervalMs.p95, "<= 12.5 ms (at most two 165 Hz refresh periods)");
    check(checks, "global frame gap", (globalIdle.intervalMs.maximum ?? Infinity) < 50,
      globalIdle.intervalMs.maximum, "< 50 ms");
    check(checks, "global CPU p95", (globalIdle.cpuMs.p95 ?? Infinity) <= 5.5,
      globalIdle.cpuMs.p95, "<= 5.5 ms");
    check(checks, "global GPU timer available", globalIdle.gpuMs.count >= 30,
      globalIdle.gpuMs.count, ">= 30 native GPU samples");
    check(checks, "global GPU p95", (globalIdle.gpuMs.p95 ?? Infinity) <= 5.5,
      globalIdle.gpuMs.p95, "<= 5.5 ms");

    runtimeMetrics.resetSamples("global");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await sleep(4_000);
    globalTransition = surfaceReport("global");
    check(checks, "global activation cadence p95", (globalTransition.intervalMs.p95 ?? Infinity) <= 12.5,
      globalTransition.intervalMs.p95, "<= 12.5 ms");
    check(checks, "global activation frame gap", (globalTransition.intervalMs.maximum ?? Infinity) < 50,
      globalTransition.intervalMs.maximum, "< 50 ms");

    const beforeGlobalReduced = runtimeMetrics.snapshot("global").frames;
    reduced.checked = true;
    reduced.dispatchEvent(new Event("change"));
    await sleep(450);
    const afterGlobalReduced = runtimeMetrics.snapshot("global").frames;
    check(checks, "global reduced-motion pause", afterGlobalReduced === beforeGlobalReduced,
      afterGlobalReduced - beforeGlobalReduced, "0 frames");
    reduced.checked = false;
    reduced.dispatchEvent(new Event("change"));
    await sleep(250);
    check(checks, "global reduced-motion resume", runtimeMetrics.snapshot("global").frames > afterGlobalReduced,
      runtimeMetrics.snapshot("global").frames - afterGlobalReduced, "> 0 frames");

    for (let cycle = 0; cycle < 3; cycle++) {
      let switchedAt = performance.now();
      click("#scope-toggle");
      await waitFor(() => document.querySelector(".local-akasha") !== null, `soak local ${cycle}`);
      soakSwitches.push(performance.now() - switchedAt);
      switchedAt = performance.now();
      click("#scope-toggle");
      await waitFor(() => document.querySelector(".world-canvas") !== null, `soak global ${cycle}`, 12_000);
      soakSwitches.push(performance.now() - switchedAt);
    }
    const soakSwitchMs = roundedSummary(summarizeSamples(soakSwitches));
    check(checks, "scope-switch soak", (soakSwitchMs.maximum ?? Infinity) <= 5_000,
      soakSwitchMs.maximum, "every switch <= 5,000 ms");

    await sleep(5_000);

    const localLifecycle = runtimeMetrics.snapshot("local");
    const globalLifecycle = runtimeMetrics.snapshot("global");
    const canvases = document.querySelectorAll("canvas").length;
    const webglCanvases = document.querySelectorAll(".world-canvas").length;
    check(checks, "single active renderer", globalLifecycle.active === 1 && localLifecycle.active === 0,
      `local ${localLifecycle.active}, global ${globalLifecycle.active}`, "local 0, global 1");
    check(checks, "scene lifecycle balance",
      localLifecycle.mounts === localLifecycle.destroys && globalLifecycle.mounts === globalLifecycle.destroys + 1,
      `local ${localLifecycle.mounts}/${localLifecycle.destroys}, global ${globalLifecycle.mounts}/${globalLifecycle.destroys}`,
      "all replaced scenes destroyed; one global scene active");
    check(checks, "canvas ownership", webglCanvases === 1 && canvases === 1,
      `${webglCanvases} WebGL / ${canvases} total`, "one WebGL canvas and no orphan canvas");

    const eventLoopDelayMs = roundedSummary(summarizeSamples(eventLoopDelays));
    check(checks, "event-loop delay p95", (eventLoopDelayMs.p95 ?? Infinity) <= 20,
      eventLoopDelayMs.p95, "<= 20 ms");
    check(checks, "event-loop stall", (eventLoopDelayMs.maximum ?? Infinity) < 100,
      eventLoopDelayMs.maximum, "< 100 ms");

    await publish({
      version: 1,
      passed: checks.every((item) => item.pass),
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        screenWidth: window.screen.width,
        screenHeight: window.screen.height,
        dpr: window.devicePixelRatio,
      },
      local,
      globalIdle,
      globalTransition,
      interactions: {
        localTransitionMs: rounded(localTransitionMs)!,
        noteOpenMs: rounded(noteOpenMs)!,
        firstGlobalSwitchMs: rounded(firstGlobalSwitchMs)!,
        soakSwitchMs,
      },
      lifecycle: {
        local: { mounts: localLifecycle.mounts, destroys: localLifecycle.destroys, active: localLifecycle.active },
        global: { mounts: globalLifecycle.mounts, destroys: globalLifecycle.destroys, active: globalLifecycle.active },
        canvases,
        webglCanvases,
      },
      eventLoopDelayMs,
      longTasks: { supported: longTaskSupported, durationsMs: roundedSummary(summarizeSamples(longTasks)) },
      checks,
    });
  } catch (error) {
    const localLifecycle = runtimeMetrics.snapshot("local");
    const globalLifecycle = runtimeMetrics.snapshot("global");
    await publish({
      version: 1,
      passed: false,
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        screenWidth: window.screen.width,
        screenHeight: window.screen.height,
        dpr: window.devicePixelRatio,
      },
      local,
      globalIdle: globalIdle.frames ? globalIdle : surfaceReport("global"),
      globalTransition: globalTransition.frames ? globalTransition : surfaceReport("global"),
      interactions: {
        localTransitionMs: rounded(localTransitionMs)!,
        noteOpenMs: rounded(noteOpenMs)!,
        firstGlobalSwitchMs: rounded(firstGlobalSwitchMs)!,
        soakSwitchMs: roundedSummary(summarizeSamples(soakSwitches)),
      },
      lifecycle: {
        local: { mounts: localLifecycle.mounts, destroys: localLifecycle.destroys, active: localLifecycle.active },
        global: { mounts: globalLifecycle.mounts, destroys: globalLifecycle.destroys, active: globalLifecycle.active },
        canvases: document.querySelectorAll("canvas").length,
        webglCanvases: document.querySelectorAll(".world-canvas").length,
      },
      eventLoopDelayMs: roundedSummary(summarizeSamples(eventLoopDelays)),
      longTasks: { supported: longTaskSupported, durationsMs: roundedSummary(summarizeSamples(longTasks)) },
      checks,
      error: error instanceof Error ? error.message : String(error),
    });
  } finally {
    window.clearInterval(interval);
    observer?.disconnect();
  }
}
