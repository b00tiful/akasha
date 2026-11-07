import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { createFixture, scoreSources } from "./benchmark-local-librarian.mjs";
import { createBroker, runInvestigation } from "./local-investigator.mjs";
import { localChat } from "./local-librarian.mjs";

const workspace = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const hash = value => createHash("sha256").update(value).digest("hex");
async function snapshot(path, prefix = "") {
  const files = {};
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const name = prefix + entry.name;
    if (entry.isDirectory()) Object.assign(files, await snapshot(join(path, entry.name), name + "/"));
    else { assert(entry.isFile()); files[name] = hash(await readFile(join(path, entry.name))); }
  }
  return files;
}
function score(task, sources) {
  if (task.required_code) return { complete: task.required_code.every(text => sources.some(source => source.kind === "codegraph_report" && source.quote.includes(text))),
    selected: sources.length, irrelevant: sources.filter(source => source.kind !== "codegraph_report").length };
  return scoreSources(task, sources);
}
export async function compare(values) {
  const out = resolve(values.out); await mkdir(dirname(out), { recursive: true }); await mkdir(out);
  const write = (name, value) => writeFile(join(out, name), JSON.stringify(value, null, 2) + "\n", { mode: 0o600, flag: "wx" });
  const corpusFile = join(workspace, "tests/fixtures/librarian-benchmark/corpus.json");
  const tasksFile = join(workspace, "tests/fixtures/librarian-benchmark/investigator.json");
  const corpus = JSON.parse(await readFile(corpusFile)); const tasks = JSON.parse(await readFile(tasksFile));
  const sourceFiles = [corpusFile, tasksFile, ...["local-librarian.mjs", "local-investigator.mjs", "benchmark-local-investigator.mjs"].map(name => join(workspace, "scripts", name)), join(workspace, "target/debug/akasha")];
  const pins = Object.fromEntries(await Promise.all(sourceFiles.map(async path => [path, hash(await readFile(path))])));
  await write("freeze.json", pins);
  const root = await createFixture(out, corpus);
  if (values.codegraph) await writeFile(join(root, "Meta/projects.yaml"), `example:\n  path: ${JSON.stringify(workspace)}\n  status: active\nforeign:\n  path: ../../foreign-repository\n  status: active\n`);
  const before = await snapshot(root); const results = [];
  const options = { root, project: "example", server: values.server, model: values.model, modelSha256: values["model-sha256"], threads: 4 };
  for (const entry of tasks.cases) {
    const task = entry.memory_case ? { ...corpus.cases.find(item => item.id === entry.memory_case), ...entry } : entry;
    if (task.required_code && !values.codegraph) { results.push({ id: task.id, skipped: "existing CodeGraph not supplied" }); continue; }
    const signal = AbortSignal.timeout(120_000);
    const baselineStarted = performance.now();
    const broker = await createBroker({ ...options, codegraph: task.required_code ? values.codegraph : undefined, signal });
    let deterministic, selected, deterministicMs, selectorMs;
    try {
      const initial = await broker.search(task.question);
      const initialPacket = broker.latestPacket;
      const initialMs = performance.now() - baselineStarted;
      deterministic = [...initial];
      for (const source of initial.slice(0, 2)) deterministic.push(...await broker.search(task.question, source.id));
      if (task.required_code) deterministic.push(...await broker.explore(task.question));
      deterministic = deterministic.filter((source, index, all) => all.findIndex(other => other.id === source.id && other.quote === source.quote) === index);
      await broker.verify(deterministic);
      deterministicMs = performance.now() - baselineStarted;
      // The historical selector has only one memory packet; code tasks are separate smoke checks.
      if (!task.required_code) {
        const selectorStarted = performance.now();
        const response = await localChat(initialPacket, options); await write(task.id + "-selector-raw.json", response);
        const ids = JSON.parse(response.answer).selections.map(item => item.citation);
        assert(ids.every(id => Number.isInteger(id) && id > 0 && id <= initial.length) && new Set(ids).size === ids.length);
        selected = ids.map(id => initial[id - 1]); await broker.verify(selected);
        selectorMs = initialMs + performance.now() - selectorStarted;
      }
    } finally { await broker.close(); }
    const started = performance.now(); let investigation;
    try { investigation = await runInvestigation({ ...options, question: task.question, codegraph: task.required_code ? values.codegraph : undefined }); }
    catch (error) { investigation = { ...error.investigationTrace, status: "failed", reason: error.message, matches: [] }; }
    await write(task.id + "-investigation.json", investigation);
    const row = { id: task.id, category: task.category ?? "code-smoke", deterministic: score(task, deterministic),
      selector: selected ? score(task, selected) : null, investigator: score(task, investigation.matches),
      status: investigation.status, reason: investigation.reason, tools: investigation.tools,
      deterministic_ms: deterministicMs, selector_ms: selectorMs ?? null,
      operational_complete: investigation.status === "finished" && score(task, investigation.matches).complete,
      model_calls: investigation.trace?.filter(item => item.response).length ?? 0, wall_ms: performance.now() - started };
    results.push(row); await write(task.id + "-scores.json", row); console.log(JSON.stringify(row));
    assert.deepEqual(await snapshot(root), before, "Canonical sources changed");
    for (const [path, digest] of Object.entries(pins)) assert.equal(hash(await readFile(path)), digest, "Frozen source changed");
  }
  await write("results.json", results); await write("sources.json", before); return results;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: Object.fromEntries(["out", "server", "model", "model-sha256", "codegraph"].map(name => [name, { type: "string" }])) });
  try {
    for (const key of ["out", "server", "model", "model-sha256"]) if (!values[key]) throw new Error(`Required --${key}`);
    await compare(values);
  } catch (error) { process.stderr.write(error.stack + "\n"); process.exitCode = 1; }
}
