#!/usr/bin/env node

import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktop = join(repository, "apps", "desktop");
const binary = join(repository, "target", "release", "akasha-desktop");
const skipBuild = process.argv.includes("--no-build");

if (!skipBuild) {
  const build = spawnSync(
    "npm",
    ["run", "tauri", "--", "build", "--features", "desktop,runtime-probe", "--no-bundle"],
    {
      cwd: desktop,
      env: { ...process.env, VITE_AKASHA_RUNTIME_PROBE: "1" },
      stdio: "inherit",
    },
  );
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const sleep = (milliseconds) => new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));

function clientProperties(pid) {
  let root;
  try {
    root = execFileSync("xprop", ["-root", "_NET_CLIENT_LIST"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
  } catch {
    return null;
  }
  for (const id of root.match(/0x[0-9a-f]+/gi) ?? []) {
    try {
      const properties = execFileSync("xprop", ["-id", id, "_NET_WM_PID", "_NET_WM_NAME"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      });
      const owner = Number(properties.match(/_NET_WM_PID\(CARDINAL\) = (\d+)/)?.[1]);
      if (owner !== pid) continue;
      const title = properties.match(/_NET_WM_NAME\(UTF8_STRING\) = "([^"]*)"/)?.[1] ?? "";
      return { id, title };
    } catch {
      // A window can disappear between root enumeration and property inspection.
    }
  }
  return null;
}

function processSample(rootPid) {
  const rows = execFileSync("ps", ["-eo", "pid=,ppid=,rss=,pcpu=,comm="], { encoding: "utf8" })
    .trim()
    .split("\n")
    .map((line) => {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+([\d.]+)\s+(.+)$/);
      return match
        ? { pid: Number(match[1]), ppid: Number(match[2]), rssKiB: Number(match[3]), cpu: Number(match[4]), command: match[5] }
        : null;
    })
    .filter(Boolean);
  const descendants = new Set([rootPid]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (descendants.has(row.ppid) && !descendants.has(row.pid)) {
        descendants.add(row.pid);
        changed = true;
      }
    }
  }
  const owned = rows.filter((row) => descendants.has(row.pid));
  return {
    rssMiB: owned.reduce((sum, row) => sum + row.rssKiB, 0) / 1024,
    cpuPercent: owned.reduce((sum, row) => sum + row.cpu, 0),
    processes: owned.length,
    commands: [...new Set(owned.map((row) => row.command))].sort(),
  };
}

function summarizeProcess(samples) {
  const first = samples.slice(0, Math.min(5, samples.length));
  const last = samples.slice(-Math.min(5, samples.length));
  const mean = (values) => values.reduce((sum, value) => sum + value, 0) / Math.max(values.length, 1);
  const rounded = (value) => Math.round(value * 100) / 100;
  const startRssMiB = mean(first.map((sample) => sample.rssMiB));
  const endRssMiB = mean(last.map((sample) => sample.rssMiB));
  const tail = samples.slice(-Math.min(20, samples.length));
  const tailStart = tail.slice(0, Math.min(5, tail.length));
  const tailEnd = tail.slice(-Math.min(5, tail.length));
  const steadyGrowthMiB = mean(tailEnd.map((sample) => sample.rssMiB)) -
    mean(tailStart.map((sample) => sample.rssMiB));
  return {
    samples: samples.length,
    startRssMiB: rounded(startRssMiB),
    endRssMiB: rounded(endRssMiB),
    growthMiB: rounded(endRssMiB - startRssMiB),
    steadyGrowthMiB: rounded(steadyGrowthMiB),
    peakRssMiB: rounded(Math.max(...samples.map((sample) => sample.rssMiB))),
    meanCpuPercent: rounded(mean(samples.map((sample) => sample.cpuPercent))),
    peakCpuPercent: rounded(Math.max(...samples.map((sample) => sample.cpuPercent))),
    peakProcesses: Math.max(...samples.map((sample) => sample.processes)),
    commands: [...new Set(samples.flatMap((sample) => sample.commands))].sort(),
  };
}

async function runScenario(name, fullscreen) {
  const isolated = mkdtempSync(join(tmpdir(), `akasha-runtime-${name}-`));
  const reportPath = join(isolated, "report.json");
  const output = [];
  const errors = [];
  const child = spawn(binary, [], {
    cwd: join(desktop, "src-tauri"),
    env: {
      ...process.env,
      AKASHA_RUNTIME_FULLSCREEN: fullscreen ? "1" : "0",
      AKASHA_RUNTIME_REPORT: reportPath,
      XDG_CACHE_HOME: join(isolated, "cache"),
      XDG_CONFIG_HOME: join(isolated, "config"),
      XDG_DATA_HOME: join(isolated, "data"),
      XDG_STATE_HOME: join(isolated, "state"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", (chunk) => output.push(chunk.toString()));
  child.stderr.on("data", (chunk) => errors.push(chunk.toString()));

  const samples = [];
  let report = null;
  let observedWindow = false;
  const deadline = Date.now() + 120_000;
  try {
    while (Date.now() < deadline) {
      if (child.exitCode !== null) throw new Error(`desktop exited with status ${child.exitCode}`);
      const window = clientProperties(child.pid);
      if (window) observedWindow = true;
      if (observedWindow) samples.push(processSample(child.pid));
      if (existsSync(reportPath)) {
        report = JSON.parse(readFileSync(reportPath, "utf8"));
        await sleep(500);
        samples.push(processSample(child.pid));
        break;
      }
      await sleep(250);
    }
    if (!report) throw new Error("timed out waiting for the native runtime report");
  } catch (error) {
    throw new Error(`${error.message}\nstdout:\n${output.join("").slice(-4_000)}\nstderr:\n${errors.join("").slice(-4_000)}`);
  } finally {
    child.kill("SIGTERM");
    await Promise.race([
      new Promise((resolveExit) => child.once("exit", resolveExit)),
      sleep(5_000).then(() => child.kill("SIGKILL")),
    ]);
    rmSync(isolated, { recursive: true, force: true });
  }

  const processMetrics = summarizeProcess(samples);
  const processChecks = [
    {
      name: "native RSS growth during final steady-state window",
      pass: processMetrics.steadyGrowthMiB <= 64,
      actual: processMetrics.steadyGrowthMiB,
      expected: "<= 64 MiB over the final five-second sample window",
    },
    {
      name: "native peak RSS",
      pass: processMetrics.peakRssMiB <= 1_536,
      actual: processMetrics.peakRssMiB,
      expected: "<= 1,536 MiB",
    },
  ];
  return {
    scenario: name,
    passed: report.passed && processChecks.every((check) => check.pass),
    report,
    process: processMetrics,
    processChecks,
    nativeStderr: errors.join("").trim().split("\n").filter(Boolean).slice(-12),
  };
}

const results = [];
try {
  results.push(await runScenario("windowed", false));
  results.push(await runScenario("fullscreen", true));
  const final = {
    version: 1,
    generatedAt: new Date().toISOString(),
    target: "Ubuntu 22.04 / native Tauri release / 2560x1600@165Hz",
    passed: results.every((result) => result.passed),
    results,
  };
  process.stdout.write(`${JSON.stringify(final, null, 2)}\n`);
  if (!final.passed) process.exitCode = 1;
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
}
