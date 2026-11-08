#!/usr/bin/env node
// Frozen fictional retrieval comparison. No downloads or canonical writes during measurement.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, lstat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { createFixture, scoreSources, verifySources } from "./benchmark-local-librarian.mjs";
import { createBroker, investigate, POLICY } from "./local-investigator.mjs";
import { expandQuery } from "./librarian-query-expansion.mjs";
import { localCompletion, createInferenceSession } from "./local-librarian.mjs";

const workspace = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
async function snapshot(root, prefix = "") {
  const result = {};
  for (const entry of (await readdir(join(root, prefix))).sort()) {
    const id = prefix + entry, path = join(root, id), stat = await lstat(path);
    assert(stat.isDirectory() || stat.isFile()); assert(!stat.isSymbolicLink());
    result[id] = { mode: stat.mode & 0o777, ...(stat.isFile() ? { sha256: hash(await readFile(path)) } : {}) };
    if (stat.isDirectory()) Object.assign(result, await snapshot(root, id + "/"));
  }
  return result;
}
const aggregate = (results, key) => ({ cases: results.length, complete: results.filter(item => item[key].complete).length,
  covered: results.reduce((sum, item) => sum + item[key].covered, 0), required: results.reduce((sum, item) => sum + item[key].required, 0),
  irrelevant: results.reduce((sum, item) => sum + item[key].irrelevant, 0) });

export async function benchmark(values) {
  if (!values.out || Boolean(values.live) === Boolean(values.replay)) throw new Error("Use --out NEW_DIRECTORY and exactly one of --live or --replay PRIOR_RUN");
  if (values.live && ["server", "model", "model-sha256"].some(key => !values[key])) throw new Error("Explicit local runtime/model/hash required");
  if (values["worker-session"] !== undefined && typeof values["worker-session"] !== "boolean") throw new Error("Invalid worker-session control");
  const corpusFile = join(workspace, "tests/fixtures/librarian-benchmark/multilingual.json");
  const corpus = JSON.parse(await readFile(corpusFile));
  const selected = values.case ?? corpus.cases.map(item => item.id);
  if (!Array.isArray(selected) || !selected.length || new Set(selected).size !== selected.length
    || selected.some(id => !corpus.cases.some(item => item.id === id))) throw new Error("Use unique known --case IDs");
  process.umask(0o077);
  const out = resolve(values.out); await mkdir(dirname(out), { recursive: true }); await mkdir(out);
  const save = (name, value) => writeFile(join(out, name), JSON.stringify(value, null, 2) + "\n", { flag: "wx", mode: 0o600 });
  const root = await createFixture(out, corpus), before = await snapshot(root);
  const cli = resolve(values.cli ?? join(workspace, "target/debug/akasha"));
  const files = [corpusFile, cli, ...["benchmark-query-expansion.mjs", "local-investigator.mjs", "librarian-query-expansion.mjs", "local-librarian.mjs"].map(name => join(workspace, "scripts", name))];
  const pins = Object.fromEntries(await Promise.all(files.map(async path => [path, hash(await readFile(path))])));
  await save("frozen-inputs.json", { pins, before, options: { ...values, model: values.model ? resolve(values.model) : null } });
  const controller = new AbortController(), cancel = () => controller.abort();
  process.once("SIGINT", cancel); process.once("SIGTERM", cancel);
  const results = [];
  try {
    for (const item of corpus.cases.filter(item => selected.includes(item.id))) {
      const started = performance.now(), completions = [];
      const recorded = values.replay ? JSON.parse(await readFile(join(resolve(values.replay), `${item.id}.completions.json`))) : null;
      const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(120_000)]);
      const runtime = { server: values.server, model: values.model, modelSha256: values["model-sha256"],
        gpuLayers: Number(values["gpu-layers"] ?? 0), threads: 4, signal };
      const session = values["worker-session"] && !recorded ? createInferenceSession(runtime) : null;
      const complete = async (input, schema, policy) => {
        const request = { input, schema, policy };
        let output;
        if (recorded) {
          assert.deepEqual(request, recorded[completions.length].request, "Replay decision input changed");
          output = recorded[completions.length].output;
        } else output = session ? await session.complete(input, schema, policy) : await localCompletion(input, schema, policy, runtime);
        completions.push({ request, output }); return output;
      };
      let result, broker;
      try {
        broker = await createBroker({ root, project: "example", cli, signal });
        result = await investigate({ question: item.question, broker, signal,
          expand: question => expandQuery(question, complete), decide: (input, schema) => complete(input, schema, POLICY) });
        if (recorded) assert.equal(completions.length, recorded.length, "Replay action count changed");
      } catch (error) {
        await save(`${item.id}.failure.json`, { error: error.message, trace: error.investigationTrace ?? null }); throw error;
      } finally {
        try { if (session) await session.close(); } finally { if (broker) await broker.close(); }
        if (session) await save(`${item.id}.session.json`, session.metrics);
        await save(`${item.id}.completions.json`, completions);
      }
      if (session) result.inference_session = session.metrics;
      await save(`${item.id}.result.json`, result);
      const baseline = result.trace[0].observations;
      const expanded = result.trace.find(entry => entry.observations && !entry.initial_search)?.observations ?? baseline;
      const all = [...new Map([...baseline, ...expanded].map(source => [source.id + "\n" + source.quote, source])).values()];
      for (const passages of [all, result.matches]) for (const index of new Set(passages.map(source => source.packet_index))) {
        const group = passages.filter(source => source.packet_index === index);
        await verifySources(root, { evidence: group }, { kind: "source_passages", matches: group });
      }
      assert(result.inspected.every(source => source.kind === "note" && source.id.startsWith("Projects/example/")));
      const row = { worker_starts: session ? session.metrics.worker_starts : completions.reduce((sum, entry) => sum + (entry.output.metrics.worker_starts ?? 0), 0),
        id: item.id, category: item.category, status: result.status,
        baseline: scoreSources(item, baseline), expanded: scoreSources(item, all), selected: scoreSources(item, result.matches),
        decisions: completions.length, tools: result.tools, terms: result.trace.find(entry => entry.query_expansion)?.terms,
        wall_ms: performance.now() - started, inference_ms: recorded ? null : completions.reduce((sum, entry) => sum + entry.output.metrics.total_ms, 0) + (session?.metrics.cleanup_ms ?? 0) };
      results.push(row); await save(`${item.id}.score.json`, row);
      process.stdout.write(JSON.stringify(row) + "\n");
    }
    assert.deepEqual(await snapshot(root), before, "Source files, modes or inventory changed");
    for (const [path, digest] of Object.entries(pins)) assert.equal(hash(await readFile(path)), digest, `Frozen source changed: ${path}`);
    const summary = { replay: !!values.replay, baseline: aggregate(results, "baseline"), expanded: aggregate(results, "expanded"),
      selected: aggregate(results, "selected"), groups: Object.fromEntries([...new Set(results.map(item => item.category))].map(category => {
        const group = results.filter(item => item.category === category);
        return [category, Object.fromEntries(["baseline", "expanded", "selected"].map(key => [key, aggregate(group, key)]))];
      })), finished: results.filter(item => item.status === "finished").length,
      worker_starts: results.reduce((sum, row) => sum + row.worker_starts, 0),
      unchanged_files: Object.values(before).filter(entry => entry.sha256).length, exact_source_checks: true, results };
    await save("summary.json", summary); return summary;
  } finally { process.removeListener("SIGINT", cancel); process.removeListener("SIGTERM", cancel); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { ...Object.fromEntries(["out", "server", "model", "model-sha256", "gpu-layers", "cli", "replay"].map(key => [key, { type: "string" }])), live: { type: "boolean" }, "worker-session": { type: "boolean", default: false }, case: { type: "string", multiple: true } } });
  benchmark(values).catch(error => { process.stderr.write(`Retrieval benchmark failed: ${error.message}\n`); process.exitCode = 1; });
}
