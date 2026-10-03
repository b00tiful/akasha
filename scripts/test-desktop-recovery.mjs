#!/usr/bin/env node

import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktop = join(repository, "apps/desktop");
const captureIndex = process.argv.indexOf("--capture-dir");
if (captureIndex >= 0 && !process.argv[captureIndex + 1]) {
  throw new Error("--capture-dir requires a path");
}
const captureDir = captureIndex < 0 ? null : resolve(process.argv[captureIndex + 1]);
const isolated = mkdtempSync(join(tmpdir(), "akasha-recovery-"));
const root = join(isolated, "valid-root");
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
let diagnosticBuildStarted = false;

function run(command, args, cwd = repository, env = process.env) {
  const result = spawnSync(command, args, { cwd, env, stdio: ["ignore", 2, 2] });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
}

function snapshot(directory, prefix = "") {
  return readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))
    .flatMap((entry) => {
      const path = join(directory, entry.name);
      const id = prefix + entry.name;
      if (entry.isDirectory()) return snapshot(path, `${id}/`);
      if (!entry.isFile()) throw new Error(`unexpected fixture entry: ${id}`);
      return [[id, createHash("sha256").update(readFileSync(path)).digest("hex")]];
    });
}

function windowFor(pid) {
  const clients = execFileSync("xprop", ["-root", "_NET_CLIENT_LIST"], { encoding: "utf8" });
  for (const id of clients.match(/0x[0-9a-f]+/gi) ?? []) {
    try {
      const properties = execFileSync("xprop", ["-id", id, "_NET_WM_PID", "_NET_WM_NAME"], {
        encoding: "utf8", stdio: ["ignore", "pipe", "ignore"],
      });
      if (Number(properties.match(/_NET_WM_PID\(CARDINAL\) = (\d+)/)?.[1]) === pid) {
        return { id, title: properties.match(/_NET_WM_NAME\(UTF8_STRING\) = "([^"]*)"/)?.[1] };
      }
    } catch { /* The window may close between enumeration and inspection. */ }
  }
  return null;
}

async function scenario(name, fullscreen, expected) {
  const reportPath = join(isolated, `${name}.json`);
  const output = [];
  const child = spawn(join(repository, "target/release/akasha-desktop"), [], {
    cwd: join(desktop, "src-tauri"),
    env: {
      ...process.env,
      AKASHA_RUNTIME_FULLSCREEN: fullscreen ? "1" : "0",
      AKASHA_RUNTIME_REPORT: reportPath,
      XDG_CONFIG_HOME: join(isolated, name, "config"),
      XDG_CACHE_HOME: join(isolated, name, "cache"),
      XDG_DATA_HOME: join(isolated, name, "data"),
      XDG_STATE_HOME: join(isolated, name, "state"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", (data) => output.push(data.toString()));
  child.stderr.on("data", (data) => output.push(data.toString()));
  let spawnError;
  child.on("error", (error) => { spawnError = error; });
  try {
    const deadline = Date.now() + 30_000;
    let window;
    while (Date.now() < deadline) {
      if (spawnError) throw spawnError;
      if (child.exitCode !== null) throw new Error(`desktop exited (${child.exitCode})`);
      window = windowFor(child.pid);
      if (existsSync(reportPath) && window) break;
      await sleep(100);
    }
    if (!existsSync(reportPath) || !window) throw new Error("native recovery report timed out");
    await sleep(200);
    const report = JSON.parse(readFileSync(reportPath, "utf8"));
    const preserved = JSON.stringify(snapshot(root)) === JSON.stringify(expected);
    if (captureDir) {
      const image = join(captureDir, `${name}.png`);
      run("import", ["-window", window.id, image]);
      chmodSync(image, 0o600);
    }
    return { name, passed: report.passed && preserved, preserved, report };
  } catch (error) {
    throw new Error(`${error.message}\n${output.join("").slice(-2_000)}`);
  } finally {
    const exited = new Promise((done) => child.once("exit", done));
    child.kill("SIGTERM");
    if (child.exitCode === null && !spawnError) {
      await Promise.race([exited, sleep(3_000)]);
      if (child.exitCode === null) { child.kill("SIGKILL"); await exited; }
    }
  }
}

try {
  if (captureDir) mkdirSync(captureDir, { recursive: true, mode: 0o700 });
  cpSync(join(repository, "tests/fixtures/resolution/valid-root"), root, { recursive: true });
  mkdirSync(join(isolated, "repository"));
  run("cargo", ["build", "-p", "akasha-cli"]);
  const id = "Projects/example/entities/core.md";
  const note = join(root, id);
  const state = join(root, "Projects/example/.akasha-state.toml");
  const noteBefore = readFileSync(note, "utf8");
  const stateBefore = readFileSync(state, "utf8");
  const noteAfter = `${noteBefore}\nSynthetic checked replacement.\n`;
  writeFileSync(join(isolated, "before.md"), noteBefore, { mode: 0o600 });
  writeFileSync(join(isolated, "after.md"), noteAfter, { mode: 0o600 });
  run(join(repository, "target/debug/akasha"), [
    "--root", root, "--project", "example", "update-entity", id,
    "--expected", join(isolated, "before.md"), "--replacement", join(isolated, "after.md"),
    "--index", join(root, "Projects/example/index.md"),
  ]);
  const stateAfter = readFileSync(state, "utf8");
  writeFileSync(note, "Unexpected external editor bytes.\n");
  writeFileSync(state, stateBefore);
  writeFileSync(join(root, "Projects/example/.akasha-edit-journal.json"), JSON.stringify({
    schema_version: 1, project: "example", id,
    note_before: noteBefore, note_after: noteAfter,
    state_before: stateBefore, state_after: stateAfter,
  }), { mode: 0o600 });
  const expected = snapshot(root);
  diagnosticBuildStarted = true;
  run("npm", ["run", "tauri", "--", "build", "--features", "desktop,runtime-probe", "--no-bundle"], desktop, {
    ...process.env, VITE_AKASHA_RUNTIME_PROBE: "0",
    VITE_AKASHA_RECOVERY_PROBE: "1", VITE_AKASHA_RECOVERY_PROBE_ROOT: root,
  });
  const results = [await scenario("windowed", false, expected), await scenario("fullscreen", true, expected)];
  const report = { version: 1, passed: results.every((result) => result.passed), results };
  if (captureDir) writeFileSync(join(captureDir, "report.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
  process.stdout.write(JSON.stringify(report, null, 2) + "\n");
  if (!report.passed) process.exitCode = 1;
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
} finally {
  if (diagnosticBuildStarted) {
    try {
      run("npm", ["run", "tauri", "--", "build", "--features", "desktop", "--no-bundle"], desktop, {
        ...process.env, VITE_AKASHA_RUNTIME_PROBE: "0",
        VITE_AKASHA_RECOVERY_PROBE: "0", VITE_AKASHA_RECOVERY_PROBE_ROOT: "",
      });
    } catch (error) {
      process.stderr.write(`Normal desktop build restoration failed: ${error.message}\n`);
      process.exitCode = 1;
    }
  }
  rmSync(isolated, { recursive: true, force: true });
}
