#!/usr/bin/env node
// Research harness: fictional files, no downloads, model tools or canonical writes.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, promisify } from "node:util";
import { localChat } from "./local-librarian.mjs";

const workspace = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const execute = promisify(execFile);
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
const corpusPath = join(workspace, "tests/fixtures/librarian-benchmark/corpus.json");
const folders = { entities: "entity", records: "record", events: "event" };

export function scoreSources(expected, sources) {
  const covered = expected.required.filter(want => sources.some(source => source.id === want.id
    && (source.excerpt ?? source.quote).includes(want.text))).length;
  const relevant = sources.filter(source => expected.allowed.includes(source.id)).length;
  return { required: expected.required.length, covered,
    complete: expected.required.length ? covered === expected.required.length : sources.length === 0,
    selected: sources.length, relevant, irrelevant: sources.length - relevant,
    precision: sources.length ? relevant / sources.length : null,
    negative_pass: expected.required.length ? null : sources.length === 0 };
}

// Independent byte/hash/line checks supplement the actual Rust checker. No semantic inference.
export async function verifySources(root, packet, checked) {
  assert.equal(checked.kind, "source_passages");
  assert(!("claims" in checked) && !("answer" in checked));
  const ids = new Set(packet.evidence.map(entry => entry.id)); const seen = new Set();
  for (const match of checked.matches) {
    assert(ids.has(match.id) && !seen.has(match.id)); seen.add(match.id);
    assert(match.id.startsWith("Projects/example/") && !match.id.split("/").includes(".."));
    const source = await readFile(join(root, match.id)); const text = source.toString("utf8");
    assert.equal(match.fingerprint, `sha256:${digest(source)}`);
    assert(match.quote.length > 0);
    const prefix = text.split("\n").slice(0, match.line_start - 1).join("\n");
    const start = prefix.length + (match.line_start > 1 ? 1 : 0);
    assert(text.slice(start).startsWith(match.quote));
    assert.equal(match.line_end, match.line_start + (match.quote.replace(/\n+$/, "").match(/\n/g) ?? []).length);
    for (const link of match.links) {
      assert(match.quote.includes(link.wikilink)); assert(ids.has(link.target));
    }
  }
}

async function snapshot(directory, prefix = "") {
  const files = {};
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    const relative = prefix + entry.name; const path = join(directory, entry.name);
    if (entry.isDirectory()) Object.assign(files, await snapshot(path, relative + "/"));
    else { assert(entry.isFile()); files[relative] = digest(await readFile(path)); }
  }
  return files;
}

export async function createFixture(out, corpus) {
  const root = join(out, "root");
  await cp(join(workspace, "tests/fixtures/resolution/valid-root"), root, { recursive: true });
  await mkdir(join(out, "repository"));
  await writeFile(join(out, "repository/README.md"), "Fictional benchmark repository; no private code.\n");
  for (const folder of ["entities", "records", "events"]) {
    await rm(join(root, "Projects/example", folder), { recursive: true });
  }
  for (const folder of ["entities", "records/tasks", "records/problems", "events/sessions", "events/handoffs"]) {
    await mkdir(join(root, "Projects/example", folder), { recursive: true });
  }
  const hashes = { entity: {}, problem: {}, session: {} };
  for (const note of corpus.notes) {
    assert(!note.path.includes("..") && Object.hasOwn(folders, note.path.split("/")[0]));
    const type = note.path.startsWith("entities/") ? "entity"
      : note.path.startsWith("records/problems/") ? "problem" : "session";
    const metadata = type === "entity"
      ? `entity: ${note.path.split("/").at(-1).slice(0, -3)}\nkind: subsystem\nstatus: ${note.status}\nreviewed: 2026-11-07`
      : `project: example\ntype: ${type}\nstatus: ${note.status}\n${type === "session" ? "date: 2026-11-07" : "created: 2026-11-07\nupdated: 2026-11-07"}`;
    const path = join(root, "Projects/example", note.path);
    await mkdir(dirname(path), { recursive: true });
    const source = `---\nschema_version: 1\n${metadata}\n---\n\n${note.body}`;
    await writeFile(path, source); hashes[type][note.path] = `sha256:${digest(source)}`;
  }
  // Establish synthetic trust BEFORE measurement, using the inspected version-1 state format.
  // The model never creates/rebuilds state. Original fixtures remain untouched.
  const sorted = entries => Object.fromEntries(Object.entries(entries).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
  let state = "schema_version = 1\n\n[events]\n";
  for (const [path, hash] of Object.entries(sorted(hashes.session))) state += `${JSON.stringify(path)} = ${JSON.stringify(hash)}\n`;
  for (const [name, type] of [["index", "entity"], ["roadmap", "problem"]]) {
    const outputHash = digest(await readFile(join(root, "Projects/example", name + ".md")));
    state += `\n[projections.${name}]\nsources = "sha256:${digest(JSON.stringify(sorted(hashes[type])))}"\noutput = "sha256:${outputHash}"\n`;
  }
  await writeFile(join(root, "Projects/example/.akasha-state.toml"), state);
  const canary = "---\nschema_version: 1\nentity: canary\nkind: subsystem\nstatus: active\nreviewed: 2026-11-07\n---\n\nORBITAL_PRIVATE_SENTINEL exists only outside the selected project.\n";
  await mkdir(join(root, "Projects/foreign/entities"), { recursive: true });
  await mkdir(join(out, "foreign-repository"));
  await writeFile(join(root, "Projects/foreign/entities/canary.md"), canary);
  await writeFile(join(root, "Global/entities/canary.md"), canary);
  await writeFile(join(root, "Meta/projects.yaml"), "example:\n  path: ../../repository\n  status: active\nforeign:\n  path: ../../foreign-repository\n  status: active\n");
  return root;
}

function aggregate(results, key) {
  const scores = results.map(result => result[key]);
  return { cases: scores.length, complete: scores.filter(score => score.complete).length,
    covered: scores.reduce((sum, score) => sum + score.covered, 0),
    required: scores.reduce((sum, score) => sum + score.required, 0),
    relevant: scores.reduce((sum, score) => sum + score.relevant, 0),
    selected: scores.reduce((sum, score) => sum + score.selected, 0),
    irrelevant: scores.reduce((sum, score) => sum + score.irrelevant, 0) };
}

function latency(results) {
  const measured = results.filter(result => Number.isFinite(result.metrics.total_ms));
  const median = numbers => {
    const sorted = numbers.toSorted((a, b) => a - b); const middle = Math.floor(sorted.length / 2);
    return sorted.length ? sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2 : null;
  };
  const phases = {};
  for (const result of measured) for (const [name, ms] of Object.entries(result.metrics.phase_ms)) phases[name] = (phases[name] ?? 0) + ms;
  return { model_requests: measured.length, skipped_requests: results.filter(result => result.metrics.skipped).length,
    median_local_ms: median(measured.map(result => result.metrics.total_ms)),
    median_prepare_ms: median(results.map(result => result.prepare_ms)), median_check_ms: median(results.map(result => result.check_ms)),
    phase_total_ms: phases,
    context_restarts: measured.reduce((sum, result) => sum + result.metrics.worker_starts - 1, 0),
    note: "Mixed query sizes on one host; local total includes model verification and cleanup, filesystem cache is uncontrolled" };
}

export async function benchmark(values) {
  if (!values.out) throw new Error("Required --out NEW_DIRECTORY; existing output is never replaced");
  if (values.live && ["server", "model", "model-sha256"].some(name => !values[name])) {
    throw new Error("Live mode requires --server, --model and --model-sha256");
  }
  const repeats = Number(values.repeats ?? "1");
  if (!Number.isInteger(repeats) || repeats < 1 || repeats > 5) throw new Error("Use 1..5 repeats");
  const out = resolve(values.out); await mkdir(dirname(out), { recursive: true }); await mkdir(out);
  const selectedCorpus = resolve(values.corpus ?? corpusPath);
  const corpusBytes = await readFile(selectedCorpus); const corpus = JSON.parse(corpusBytes);
  const root = await createFixture(out, corpus); const before = await snapshot(root);
  const cli = resolve(values.cli ?? join(workspace, "target/debug/akasha"));
  const cliHash = digest(await readFile(cli)); const runnerHash = digest(await readFile(new URL("./local-librarian.mjs", import.meta.url)));
  const write = (name, value) => writeFile(join(out, name), JSON.stringify(value, null, 2) + "\n", { mode: 0o600 });
  const run = async args => {
    assert.equal(digest(await readFile(cli)), cliHash, "CLI changed during benchmark");
    const { stdout } = await execute(cli, ["--root", root, "--project", "example", "--json", ...args],
      { timeout: 30_000, maxBuffer: 2 ** 20 });
    return JSON.parse(stdout);
  };
  const controller = new AbortController(); const cancel = () => controller.abort();
  process.once("SIGINT", cancel); process.once("SIGTERM", cancel);
  const results = [];
  try {
    for (let repeat = 0; repeat < repeats; repeat++) for (const item of corpus.cases) {
      controller.signal.throwIfAborted(); const began = performance.now();
      const name = `${item.id}-${repeat + 1}`;
      const args = ["librarian-prepare", item.question, ...(item.terms ?? []).flatMap(term => ["--term", term])];
      if (item.note) args.push("--note", item.note);
      const packet = await run(args); const prepareMs = performance.now() - began;
      assert(packet.evidence.every(source => source.id.startsWith("Projects/example/")));
      await write(`${name}.packet.json`, packet);
      const response = values.live ? await localChat(packet, {
        server: values.server, model: values.model, modelSha256: values["model-sha256"],
        gpuLayers: Number(values["gpu-layers"] ?? 0), threads: Number(values.threads ?? 4), signal: controller.signal,
      }) : { answer: JSON.stringify({ selections: packet.evidence.map(({ citation }) => ({ citation })) }), metrics: { retrieval_only: true } };
      const packetPath = join(out, `${name}.packet.json`); const selectionPath = join(out, `${name}.selection.json`);
      await writeFile(selectionPath, response.answer, { mode: 0o600 });
      const checkStarted = performance.now();
      const checked = await run(["librarian-check-sources", "--packet", packetPath, "--selection", selectionPath]);
      const checkMs = performance.now() - checkStarted;
      await verifySources(root, packet, checked); await write(`${name}.checked.json`, checked);
      const result = { id: item.id, repeat: repeat + 1, category: item.category,
        group: item.category === "capacity" ? "stress" : !item.required.length ? "negative"
          : item.category === "missing-information" ? "navigation_only"
            : item.note || item.terms?.length ? "guided" : "unassisted",
        answer_support: item.answer_support ?? "not evaluated; source navigation only", source_integrity: true,
        packet: scoreSources(item, packet.evidence), selected: scoreSources(item, checked.matches),
        candidates: packet.evidence.map(source => source.id), sources: checked.matches.map(source => source.id),
        prepare_ms: prepareMs, check_ms: checkMs, wall_ms: performance.now() - began, metrics: response.metrics };
      if (item.expected_link) result.recorded_link = checked.matches.some(source => source.id === item.expected_link[0]
        && source.links.some(link => link.target === item.expected_link[1]));
      results.push(result); await write("results.json", results);
      process.stdout.write(JSON.stringify({ id: name, packet: result.packet.complete, selected: result.selected.complete,
        irrelevant: result.selected.irrelevant, wall_ms: Math.round(result.wall_ms) }) + "\n");
    }
    const after = await snapshot(root); assert.deepEqual(after, before, "Benchmark sources changed");
    assert.equal(digest(await readFile(selectedCorpus)), digest(corpusBytes), "Corpus labels changed");
    assert.equal(digest(await readFile(new URL("./local-librarian.mjs", import.meta.url))), runnerHash, "Runner changed");
    const summary = { live: !!values.live, corpus_sha256: digest(corpusBytes), cli_sha256: cliHash,
      runner_sha256: runnerHash, packet: aggregate(results, "packet"), selected: aggregate(results, "selected"),
      latency: latency(results),
      groups: Object.fromEntries(["unassisted", "guided", "negative", "navigation_only", "stress"].map(group => {
        const entries = results.filter(result => result.group === group);
        return [group, { packet: aggregate(entries, "packet"), selected: aggregate(entries, "selected") }];
      })),
      source_integrity: results.every(result => result.source_integrity), files_unchanged: Object.keys(before).length,
      source_sha256: before, limitations: ["Fictional corpus, not private real-project validation", "Navigation relevance is not answer correctness", "Single host; repeated processes do not imply cold disk cache"] };
    await write("summary.json", summary); return summary;
  } finally {
    process.removeListener("SIGINT", cancel); process.removeListener("SIGTERM", cancel);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: {
    out: { type: "string" }, cli: { type: "string" }, corpus: { type: "string" }, live: { type: "boolean", default: false },
    server: { type: "string" }, model: { type: "string" }, "model-sha256": { type: "string" },
    "gpu-layers": { type: "string" }, threads: { type: "string" }, repeats: { type: "string" },
  } });
  benchmark(values).then(summary => process.stdout.write(JSON.stringify(summary, null, 2) + "\n"))
    .catch(error => { process.stderr.write(`Benchmark failed: ${error.message}\n`); process.exitCode = 1; });
}
