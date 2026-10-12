#!/usr/bin/env node
// Development acceptance tooling: no model is started by setup, probe or verify.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { createInterface } from "node:readline";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const workspace = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cli = join(workspace, "target/debug/akasha");
const server = join(workspace, "target/debug/akasha-onboarding-mcp");
const project = "first-run";
const tools = ["akasha_onboarding_prepare", "akasha_onboarding_validate", "akasha_onboarding_preview", "akasha_onboarding_apply"];

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", timeout: 30_000, maxBuffer: 2 ** 20, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited ${result.status}: ${result.stderr ?? ""}`);
  return result.stdout;
}

export function snapshot(directory) {
  const entries = {};
  function visit(path, key) {
    const stat = lstatSync(path);
    assert(!stat.isSymbolicLink(), `Unexpected symlink: ${key}`);
    assert(stat.isFile() || stat.isDirectory(), `Unexpected entry: ${key}`);
    entries[key] = { type: stat.isDirectory() ? "directory" : "file", mode: stat.mode & 0o777 };
    if (stat.isFile()) entries[key].sha256 = createHash("sha256").update(readFileSync(path)).digest("hex");
    else for (const name of readdirSync(path).sort()) visit(join(path, name), key === "." ? name : `${key}/${name}`);
  }
  visit(directory, ".");
  return entries;
}

function paths(directory) {
  const base = resolve(directory);
  return { directory: base, root: join(base, "memory 世界"), repository: join(base, "repository with spaces"), clientHome: join(base, "client-home") };
}

function config(handoff) {
  return `approval_policy = "on-request"\napprovals_reviewer = "user"\nsandbox_mode = "read-only"\n\n[mcp_servers.akasha_onboarding]\ncommand = ${JSON.stringify(server)}\nargs = ${JSON.stringify(handoff.args)}\nrequired = true\ndefault_tools_approval_mode = "writes"\n\n[mcp_servers.akasha_onboarding.tools.akasha_onboarding_apply]\napproval_mode = "prompt"\n`;
}

function save(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx", mode: 0o600 });
}

function akasha(fixture, args) {
  return JSON.parse(run(cli, ["--root", fixture.root, "--project", project, "--json", ...args], { cwd: fixture.repository }));
}

export function setup(directory) {
  process.umask(0o077);
  assert(lstatSync(cli).isFile() && lstatSync(server).isFile(), "Build akasha-cli and akasha-mcp first");
  const fixture = paths(directory);
  assert.equal(realpathSync(dirname(fixture.directory)), dirname(fixture.directory), "Use a canonical existing parent");
  mkdirSync(fixture.directory, { mode: 0o700 }); // Create-only; never reset an earlier run.
  cpSync(join(workspace, "tests/fixtures/codex-onboarding/repository"), fixture.repository, { recursive: true });
  mkdirSync(fixture.clientHome, { mode: 0o700 });
  const before = snapshot(fixture.directory);
  const review = JSON.parse(run(cli, ["--json", "setup-root", fixture.root]));
  assert.deepEqual(snapshot(fixture.directory), before, "Setup review must not write");
  save(join(fixture.directory, "setup-review.json"), review);
  const applied = JSON.parse(run(cli, ["--json", "setup-root", fixture.root, "--plan-id", review.plan_id]));
  save(join(fixture.directory, "setup-result.json"), applied);
  const initialized = JSON.parse(run(cli, ["--root", fixture.root, "--json", "init", project], { cwd: fixture.repository }));
  save(join(fixture.directory, "init-result.json"), initialized);
  const memoryBefore = snapshot(fixture.root);
  const repositoryBefore = snapshot(fixture.repository);
  const handoff = akasha(fixture, ["onboard"]);
  assert.deepEqual(snapshot(fixture.root), memoryBefore, "Handoff must not write memory");
  assert.deepEqual(snapshot(fixture.repository), repositoryBefore, "Handoff must not write repository");
  assert.equal(handoff.command, "akasha-onboarding-mcp");
  writeFileSync(join(fixture.clientHome, "config.toml"), config(handoff), { flag: "wx", mode: 0o600 });
  const manifest = { schema_version: 1, ...fixture, handoff, memoryBefore, repositoryBefore };
  save(join(fixture.directory, "run.json"), manifest);
  return fixture;
}

function load(directory) {
  const expected = paths(directory);
  assert.equal(realpathSync(expected.directory), expected.directory, "Run directory must be canonical");
  const fixture = JSON.parse(readFileSync(join(expected.directory, "run.json"), "utf8"));
  assert.equal(fixture.schema_version, 1);
  for (const [key, value] of Object.entries(expected)) assert.equal(fixture[key], value, `Changed ${key}`);
  assert.deepEqual(snapshot(fixture.repository), fixture.repositoryBefore, "Source repository changed");
  assert.deepEqual(akasha(fixture, ["onboard"]), fixture.handoff, "Handoff changed");
  const clientConfig = readFileSync(join(fixture.clientHome, "config.toml"), "utf8");
  const expectedConfig = config(fixture.handoff);
  // Codex 0.162.1 persists this one-time TUI marker on first interactive launch.
  // Permit only that exact observed suffix; approval and server bytes stay bound.
  const detectionMarker = '\n[tui]\nscreen_reader_detection_done = true\n';
  assert(clientConfig === expectedConfig || clientConfig === expectedConfig + detectionMarker,
    "Client policy changed");
  return fixture;
}

function requireIsolatedRepository(repository) {
  for (let parent = repository; ; parent = dirname(parent)) {
    for (const name of [".git", "AGENTS.md", "AGENTS.override.md", ".codex/config.toml"]) {
      assert(!existsSync(join(parent, name)), `Inherited project context at ${join(parent, name)}; choose a run directory outside existing repositories and agent instructions`);
    }
    if (dirname(parent) === parent) break;
  }
}

// Actual local stdio transport, bounded requests and guaranteed child cleanup.
export async function withServer(fixture, action) {
  const child = spawn(server, fixture.handoff.args, { stdio: ["pipe", "pipe", "pipe"], cwd: fixture.repository });
  const lines = createInterface({ input: child.stdout });
  const pending = new Map();
  let nextId = 0;
  let stderr = "";
  const fail = (error) => { for (const request of pending.values()) request.reject(error); };
  child.stderr.on("data", (data) => { stderr = (stderr + data).slice(-4096); });
  child.on("error", fail);
  child.stdin.on("error", fail);
  child.on("exit", () => fail(new Error(`MCP server exited: ${stderr}`)));
  lines.on("line", (line) => {
    try {
      const message = JSON.parse(line);
      const request = pending.get(message.id);
      if (request) message.error ? request.reject(new Error(JSON.stringify(message.error))) : request.resolve(message.result);
    } catch (error) { fail(error); }
  });
  const send = (value) => child.stdin.write(`${JSON.stringify(value)}\n`);
  const call = async (method, params) => {
    const id = ++nextId;
    let timer;
    try {
      return await new Promise((resolveResponse, reject) => {
        timer = setTimeout(() => reject(new Error(`MCP ${method} timed out`)), 10_000);
        pending.set(id, { resolve: resolveResponse, reject });
        send({ jsonrpc: "2.0", id, method, params });
      });
    } finally { clearTimeout(timer); pending.delete(id); }
  };
  const tool = async (name, args = {}) => {
    const result = await call("tools/call", { name, arguments: args });
    assert.notEqual(result.isError, true, JSON.stringify(result.structuredContent));
    return result.structuredContent;
  };
  try {
    await call("initialize", { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "akasha-first-run-preflight", version: "1" } });
    send({ jsonrpc: "2.0", method: "notifications/initialized" });
    return await action({ call, tool });
  } finally {
    lines.close();
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill();
      await exited;
    }
  }
}

export async function probe(directory) {
  const fixture = load(directory);
  requireIsolatedRepository(fixture.repository);
  const memoryBefore = snapshot(fixture.root);
  const listed = JSON.parse(run("codex", ["mcp", "list", "--json"], {
    cwd: fixture.repository, env: { ...process.env, CODEX_HOME: fixture.clientHome },
  }));
  assert.equal(listed.length, 1, "Expected only the disposable onboarding server");
  assert.equal(listed[0].name, "akasha_onboarding");
  assert.equal(listed[0].transport.command, server);
  assert.deepEqual(listed[0].transport.args, fixture.handoff.args);
  const prepared = await withServer(fixture, async ({ call, tool }) => {
    const discovered = (await call("tools/list", {})).tools;
    assert.deepEqual(discovered.map((entry) => entry.name).sort(), [...tools].sort());
    const apply = discovered.find((entry) => entry.name === tools[3]);
    assert.equal(apply.annotations.destructiveHint, true);
    assert.equal(apply.annotations.idempotentHint, false);
    return (await tool(tools[0])).preparation;
  });
  assert.equal(prepared.project, project);
  assert.equal(prepared.templates.length, 7);
  assert.deepEqual(snapshot(fixture.root), memoryBefore, "Preflight must not write memory");
  assert.deepEqual(snapshot(fixture.repository), fixture.repositoryBefore, "Preflight must not write repository");
  return { preflight: "passed", host_approval: "not_observed", project, templates: prepared.templates.length };
}

export async function verify(directory, outcome) {
  assert(["cancel", "apply"].includes(outcome), "Outcome must be cancel or apply");
  const fixture = load(directory);
  const beforeVerification = snapshot(fixture.root);
  if (outcome === "cancel") {
    assert.deepEqual(snapshot(fixture.root), fixture.memoryBefore, "Cancellation changed memory bytes, entries or modes");
  } else {
    await withServer(fixture, async ({ tool }) => {
      const prepared = (await tool(tools[0])).preparation;
      const folder = prepared.note_types.entity.folder;
      const projectDir = prepared.project_dir;
      const prefix = relative(fixture.root, projectDir);
      const allowed = new Set([`${prefix}/index.md`, `${prefix}/roadmap.md`, `${prefix}/.akasha-state.toml`, `${prefix}/${folder}/core.md`]);
      for (const key of new Set([...Object.keys(fixture.memoryBefore), ...Object.keys(beforeVerification)])) {
        if (!allowed.has(key)) assert.deepEqual(beforeVerification[key], fixture.memoryBefore[key], `Unexpected memory change: ${key}`);
        else {
          assert.equal(beforeVerification[key]?.type, "file", `Expected regular file: ${key}`);
          assert.equal(beforeVerification[key].mode, fixture.memoryBefore[key]?.mode ?? 0o600, `Changed file mode: ${key}`);
        }
      }
      const source = readFileSync(join(projectDir, folder, "core.md"), "utf8");
      const frontmatter = source.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/)?.[1];
      assert(frontmatter, "Expected persisted frontmatter");
      for (const kind of ["fact", "inference", "unknown"]) {
        assert(new RegExp(`^\\s*-?\\s*kind: ${kind}\\s*$`, "m").test(frontmatter), `Missing ${kind} evidence`);
      }
      // Delegate parsing, evidence hashes/rationale and projection checks to the core.
      const validated = await tool(tools[1], { proposal: {
        notes: [{ note_type: "entity", path: "core.md", source }],
        index: readFileSync(join(projectDir, "index.md"), "utf8"),
        roadmap: readFileSync(join(projectDir, "roadmap.md"), "utf8"),
      } });
      const preview = await tool(tools[2], { proposal_id: validated.proposal_id });
      assert.equal(prepared.existing_notes.length, 1, "Expected only the proposed entity");
      assert.equal(preview.notes.length, 1);
      assert.equal(preview.notes[0].action, "unchanged");
      assert.equal(preview.index_changed, false);
      assert.equal(preview.roadmap_changed, false);
      assert.equal(preview.state_changed, false);
    });
    akasha(fixture, ["validate"]);
  }
  assert.deepEqual(snapshot(fixture.root), beforeVerification, "Verification must not write memory");
  assert.deepEqual(snapshot(fixture.repository), fixture.repositoryBefore, "Source repository changed");
  return { filesystem_verification: "passed", outcome, host_approval: "not_observed", human_claim_review: "required" };
}

export function launchArguments(fixture) {
  const prompt = `${fixture.handoff.agent_request}\nFor this synthetic acceptance run, propose exactly one entity at core.md using the supplied entity template. Include at least one fact, one explicitly labeled inference and one unknown in persisted evidence; use unquoted kind values fact, inference and unknown. Inspect README.md, Cargo.toml and src/lib.rs; use actual whole-file SHA-256 evidence and justified rationale. Link the entity to the project index and roadmap. Do not invent tasks or history and do not modify repository sources. Show the exact approval_summary, proposal_id, preview_id and actions, then request apply in this same turn. The human will deny the first run and approve a fresh second run. If denied, stop without retrying or using a file-write workaround. After approval, validate and report unknowns.`;
  return ["--strict-config", "--no-daemon", "--no-alt-screen", "--cd", fixture.repository,
    "--sandbox", "read-only", "--ask-for-approval", "on-request", prompt];
}

async function main() {
  const [command, directory, outcome, ...extra] = process.argv.slice(2);
  assert(directory && extra.length === 0 && ["setup", "probe", "run", "verify"].includes(command),
    "Usage: node scripts/first-run-onboarding.mjs {setup|probe|run} NEW_RUN_DIR | verify RUN_DIR {cancel|apply}");
  assert(command === "verify" || outcome === undefined, "Unexpected argument");
  if (command === "setup") console.log(JSON.stringify(setup(directory), null, 2));
  else if (command === "probe") console.log(JSON.stringify(await probe(directory), null, 2));
  else if (command === "verify") console.log(JSON.stringify(await verify(directory, outcome), null, 2));
  else {
    assert(process.stdin.isTTY && process.stdout.isTTY, "Run requires an interactive terminal for independent human approval");
    const fixture = load(directory);
    requireIsolatedRepository(fixture.repository);
    const result = spawnSync("codex", launchArguments(fixture), {
      cwd: fixture.repository, env: { ...process.env, CODEX_HOME: fixture.clientHome }, stdio: "inherit",
    });
    if (result.error) throw result.error;
    process.exitCode = result.status ?? 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => { console.error(`first-run-onboarding: ${error.message}`); process.exitCode = 1; });
}
