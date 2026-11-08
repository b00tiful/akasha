import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile, chmod, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createFixture } from "./benchmark-local-librarian.mjs";
import { actionSchema, createBroker, investigate, runInvestigation, selectReportPassages, validateAction } from "./local-investigator.mjs";

const note = { kind: "note", id: "Projects/example/entities/a.md", quote: "A recorded fact.", fingerprint: "sha256:a" };
const reply = action => ({ answer: JSON.stringify(action), metrics: {} });
function fake() {
  const calls = []; return { calls, codegraph: false,
    search: async (query, id) => { calls.push({ query, id }); return [note]; },
    verify: async sources => { calls.push({ verify: sources }); } };
}
test("focused reports retain late and distant requested context with exact bounded ranges", () => {
  const cases = [
    { query: "How does renewLease handle an expired epoch?", report: "unrelated preamble\n".repeat(360) + "function renewLease(epoch) {\n  if (epoch < current) return 'expired';\n}\n", spans: ["if (epoch < current) return 'expired'"] },
    { query: "scheduleDrain releasePermit", report: "header\n".repeat(800) + "function scheduleDrain() { waitForReaders(); }\n" + "middle\n".repeat(800) + "function releasePermit() { permits += 1; }\n", spans: ["waitForReaders()", "permits += 1"] },
    { query: "refreshSnapshot", report: "Overview: refreshSnapshot\n" + "other material\n".repeat(600) + "function refreshSnapshot() { discardObsoleteEpoch(); }\n", spans: ["discardObsoleteEpoch()"] },
    { query: "validateTicket handlers.ts", report: "Overview: validateTicket handlers.ts\n" + "context\n".repeat(180)
      + "See validateTicket in handlers.ts\n" + "context\n".repeat(450)
      + "function validateTicket() { rejectExpiredTicket(); }\n" + "tail\n".repeat(500), spans: ["rejectExpiredTicket()"] },
    { query: "reconcileGeneration", report: "header\n".repeat(950) + "function reconcileGeneration() {\n" + "// check inputs\n".repeat(25) + "rejectObsoleteGeneration();\n}\n" + "tail\n".repeat(500), spans: ["rejectObsoleteGeneration()"] },
    { query: "锁定快照", report: "😀旁白\n".repeat(1800) + "函数 锁定快照() { 返回 不可变状态; }\n", spans: ["返回 不可变状态"] },
  ];
  for (const { query, report, spans } of cases) {
    const passages = selectReportPassages(report, query); const chars = [...report];
    assert.deepEqual(selectReportPassages(report, query), passages);
    assert(passages.length >= 1 && passages.length <= 2);
    assert(passages.reduce((total, passage) => total + [...passage.quote].length, 0) <= 4000);
    for (const span of spans) assert(passages.some(passage => passage.quote.includes(span)), `Missing independent context: ${span}`);
    let previous = 0; let retained = 0;
    for (const passage of passages) {
      const { start, end, total, unit } = passage.report_range;
      assert(start >= previous && end > start && end <= total); previous = end;
      assert.equal(unit, "unicode_code_points"); assert.equal(total, chars.length);
      assert.equal(passage.quote, chars.slice(start, end).join(""));
      assert.equal(Buffer.from(passage.quote).toString("utf8"), passage.quote);
      assert.equal(passage.selection, "query_windows_v1"); assert(passage.partial_body);
      retained += end - start;
    }
    assert(passages.every(passage => passage.omitted_code_points === chars.length - retained));
  }
});
test("whole reports and no-match prefixes preserve text; identifier matching excludes substrings", () => {
  const short = "Exact 😀 report\r\nwith no requested symbol.";
  const [whole] = selectReportPassages(short, "absent");
  assert.equal(whole.quote, short); assert.equal(whole.partial_body, false);
  assert.equal(whole.omitted_code_points, 0); assert.equal(whole.selection, "whole");
  for (const query of ["unrecorded", "how does the", "Lease"]) {
    const long = "x".repeat(3999) + "😀" + " refreshLease ".repeat(300);
    const [passage] = selectReportPassages(long, query);
    assert.equal(passage.quote, [...long].slice(0, 4000).join(""));
    assert.equal(passage.selection, "prefix_no_query_match");
    assert.equal(passage.omitted_code_points, [...long].length - 4000);
  }
  for (const report of ["", " \n", "x".repeat(65_537)]) assert.throws(() => selectReportPassages(report, "valid"), /report/);
  const fragments = "preamble\n".repeat(700) + "handlers x ts\n".repeat(100);
  assert.equal(selectReportPassages(fragments, "handlers.ts")[0].selection, "prefix_no_query_match");
  assert.throws(() => selectReportPassages("source", "--path"), /query/);
});
test("repetitive reports and boundary-adjacent Unicode identifiers keep the same bounds", () => {
  for (const padding of [3999, 4000, 4001, 4498, 4499, 4500, 4999, 5000, 5001]) {
    const report = "😀".repeat(padding) + " İSTANBUL returnSafeState();\n" + "x".repeat(1000);
    const passages = selectReportPassages(report, "İSTANBUL");
    assert(passages.some(passage => passage.quote.includes("returnSafeState()")));
    assert(passages.reduce((size, passage) => size + [...passage.quote].length, 0) <= 4000);
  }
  const repetitive = selectReportPassages("id ".repeat(21_845), "id");
  assert(repetitive.reduce((size, passage) => size + [...passage.quote].length, 0) <= 4000);
});
test("bounded loop follows a known note and returns only verified original sources", async () => {
  const broker = fake(); let step = 0;
  const result = await investigate({ question: "Find A", broker, decide: async () => reply(step++ ? { action: "finish", handles: [1] } : { action: "read", id: note.id, query: "related" }) });
  assert.equal(result.status, "finished"); assert.equal(result.tools, 2);
  assert.equal(result.matches[0].quote, note.quote); assert.deepEqual(broker.calls[1], { query: "related", id: note.id });
  assert.equal(broker.calls.at(-1).verify.length, 1);
});
test("identical report quotes at distinct ranges keep distinct handles", async () => {
  const broker = { ...fake(), codegraph: true, explore: async () => [1000, 6000].map(start => ({
    kind: "codegraph_report", id: "codegraph:repeated", quote: "same exact text", fingerprint: "sha256:one-report",
    report_range: { start, end: start + 15, total: 8000, unit: "unicode_code_points" },
  })) }; let step = 0;
  const result = await investigate({ question: "Find repeated", broker, decide: async payload => {
    if (step++ === 0) return reply({ action: "explore", query: "repeated" });
    assert.deepEqual(payload.observations.map(source => source.handle), [2, 3]);
    return reply({ action: "finish", handles: [2, 3] });
  } });
  assert.deepEqual(result.matches.map(source => source.report_range.start), [1000, 6000]);
});
test("validation rejects extra capabilities, unknown notes/handles, controls and options", () => {
  const known = [{ ...note, handle: 1 }];
  for (const action of [{ action: "shell", command: "touch x" }, { action: "search", query: "a", root: "/" },
    { action: "search", query: "--path=/" }, { action: "search", query: "a\nb" },
    { action: "read", id: "Projects/foreign/entities/x.md", query: "a" }, { action: "finish", handles: [2] },
    { action: "finish", handles: [1, 1] }, { action: "explore", query: "a" }]) assert.throws(() => validateAction(action, known, false));
  assert.equal(actionSchema(known, false).oneOf.length, 3);
});
test("repeated actions, tool and step limits remain explicitly incomplete", async () => {
  const decide = async () => reply({ action: "search", query: "repeat" });
  assert.equal((await investigate({ question: "Find A", broker: fake(), decide })).reason, "repeated_action");
  assert.equal((await investigate({ question: "Find A", broker: fake(), decide, maxTools: 1 })).reason, "tool_budget");
  assert.equal((await investigate({ question: "Find A", broker: fake(), decide, maxSteps: 1 })).reason, "step_budget");
});
test("decisions retain bounded action history and exhausted menus offer only finish", async () => {
  let step = 0;
  const result = await investigate({ question: "Find A", broker: fake(), maxSteps: 2, decide: async (payload, schema) => {
    if (step++ === 0) {
      assert.deepEqual(payload.previous_actions, []); assert(schema.oneOf.some(choice => choice.properties.action.const === "read"));
      return reply({ action: "read", id: note.id, query: "related" });
    }
    assert.deepEqual(payload.previous_actions, [{ action: "read", id: note.id, query: "related" }]);
    assert.deepEqual(schema.oneOf.map(choice => choice.properties.action.const), ["finish"]);
    return reply({ action: "finish", handles: [1] });
  } });
  assert.equal(result.status, "finished");
  assert.deepEqual(actionSchema([], false, false).oneOf.map(choice => choice.properties.action.const), ["finish"]);
});
test("opt-in catalog exactly advertises the decoder actions and current source bindings", async () => {
  for (const codegraph of [false, true]) {
    const broker = { ...fake(), codegraph }; let step = 0;
    const result = await investigate({ question: "Find A", broker, maxSteps: 2, toolCatalog: true,
      decide: async (payload, schema) => {
        assert.deepEqual(payload.available_actions.map(entry => entry.schema), schema.oneOf);
        const choices = payload.available_actions.map(entry => entry.schema.properties.action.const);
        assert.equal(choices.includes("explore"), codegraph && step === 0);
        assert.equal(choices.includes("symbol"), codegraph && step === 0);
        const read = payload.available_actions.find(entry => entry.schema.properties.action.const === "read");
        if (step++ === 0) {
          assert.deepEqual(read.schema.properties.id.enum, [note.id]);
          assert(payload.available_actions.every(entry => typeof entry.description === "string"));
          return reply({ action: "read", id: note.id, query: "related" });
        }
        assert.deepEqual(choices, ["finish"]);
        assert.deepEqual(schema.oneOf[0].properties.handles.items.enum, [1]);
        return reply({ action: "finish", handles: [1] });
      } });
    assert(result.trace.filter(entry => entry.action).every(entry => entry.available_actions));
  }
});
test("symbol actions accept one identifier, obey existing menus/budgets and retain whole sources", async () => {
  const action = { action: "symbol", name: "fetchTicket" };
  assert.deepEqual(validateAction(action, [], true), action);
  for (const name of ["--file", "src/file.rs", "a\\b", "a.b", "Type::method", "a b", "a\nb", "锁定", "1abc", "a".repeat(161)]) {
    assert.throws(() => validateAction({ ...action, name }, [], true), /symbol name/);
  }
  assert.throws(() => validateAction(action, [], false), /not enabled/);
  assert.throws(() => validateAction({ ...action, file: "secret" }, [], true), /shape/);
  const source = { kind: "codegraph_symbol", id: "codegraph-symbol:fetchTicket", quote: "Complete function", fingerprint: "sha256:x", partial_body: false };
  let step = 0; let calls = 0;
  const broker = { ...fake(), codegraph: true, symbol: async name => { calls++; assert.equal(name, action.name); return [source]; } };
  const result = await investigate({ question: "Find fetchTicket", broker, toolCatalog: true, decide: async () => reply(step++ ? { action: "finish", handles: [2] } : action) });
  assert.equal(result.status, "finished"); assert.equal(result.tools, 2); assert.equal(calls, 1); assert.equal(result.matches[0].quote, source.quote);
  const limited = await investigate({ question: "Find fetchTicket", broker, maxTools: 1, decide: async () => reply(action) });
  assert.equal(limited.reason, "tool_budget"); assert.equal(calls, 1);
});
test("invalid inference resource settings refuse before broker or model access", async () => {
  for (const options of [{ gpuLayers: -1 }, { gpuLayers: 1000 }, { gpuLayers: NaN }, { threads: 0 }, { threads: 257 }, { maxContext: 1234 }]) {
    await assert.rejects(runInvestigation(options), /resource settings/);
  }
});
test("explicit symbol context is loaded before inference, consumes budget and validates before reads", async () => {
  let reads = 0; let decisions = 0;
  const source = { kind: "codegraph_symbol", id: "codegraph-symbol:fetchTicket", quote: "Full function", fingerprint: "sha256:x", partial_body: false };
  const broker = { ...fake(), codegraph: true, symbol: async name => { reads++; assert.equal(name, "fetchTicket"); return [source]; } };
  const decide = async payload => { decisions++; assert.equal(payload.observations[0].quote, source.quote); assert.equal(payload.remaining_tools, 2); return reply({ action: "finish", handles: [2] }); };
  const result = await investigate({ question: "Inspect this function", symbol: "fetchTicket", broker, decide });
  assert.equal(result.status, "finished"); assert.equal(result.tools, 2); assert.equal(result.trace[1].initial_symbol, "fetchTicket");
  assert.equal(reads, 1); assert.equal(decisions, 1);
  const limited = await investigate({ question: "Inspect this function", symbol: "fetchTicket", broker, decide, maxTools: 1 });
  assert.equal(limited.reason, "tool_budget"); assert.equal(reads, 1); assert.equal(decisions, 1);
  await assert.rejects(investigate({ question: "Inspect", symbol: "../bad", broker, decide }), /symbol name/);
  await assert.rejects(investigate({ question: "Inspect", symbol: "fetchTicket", broker: fake(), decide }), /requires enabled/);
});
test("symbol broker pins exact unique repository sources and rejects incomplete or unsafe reports", async t => {
  const out = await mkdtemp(join(tmpdir(), "akasha-symbol-test-")); t.after(() => rm(out, { recursive: true, force: true }));
  const root = await createFixture(out, { notes: [], cases: [] }); const repository = join(out, "repository");
  await mkdir(join(repository, ".codegraph"));
  await writeFile(join(repository, "unit.mjs"), "function fetchTicket() {\n  return '完整 😀';\n}\n");
  const node = { name: "fetchTicket", filePath: "unit.mjs", startLine: 1, endLine: 3, kind: "function" };
  const report = "**fetchTicket** (function)\n\n**Location:** unit.mjs:1\n\n```javascript\n1\tfunction fetchTicket() {\n2\t  return '完整 😀';\n3\t}\n```\n";
  const tool = join(out, "graph"); const queryFile = join(out, "query.json"); const reportFile = join(out, "report.txt"); const callsFile = join(out, "calls");
  await writeFile(tool, `#!/usr/bin/env node\nimport {appendFileSync,readFileSync} from 'node:fs';appendFileSync(${JSON.stringify(callsFile)},JSON.stringify(process.argv.slice(2))+'\\n');process.stdout.write(readFileSync(process.argv[3]==='query'?${JSON.stringify(queryFile)}:${JSON.stringify(reportFile)}));\n`);
  await chmod(tool, 0o700);
  const set = async (entries = [{ node }], text = report) => { await writeFile(queryFile, JSON.stringify(entries)); await writeFile(reportFile, text); };
  await set();
  const broker = await createBroker({ root, project: "example", codegraph: tool }); t.after(broker.close);
  const sources = await broker.symbol("fetchTicket"); await broker.verify(sources);
  assert.equal(sources[0].quote, report); assert.equal(sources[0].partial_body, false); assert.equal(sources[0].omitted_code_points, 0);
  assert.deepEqual(sources[0].symbol, { name: "fetchTicket", file: "unit.mjs", line_start: 1, line_end: 3 });
  const calls = (await readFile(callsFile, "utf8")).trim().split("\n").map(line => JSON.parse(line));
  assert.equal(calls.length, 4);
  assert.deepEqual(calls[0], ["--no-color", "query", "fetchTicket", "--json", "--limit", "8", "--path", repository]);
  assert.deepEqual(calls[1], ["--no-color", "node", "fetchTicket", "--file", "unit.mjs", "--path", repository]);
  for (const entries of [[], [{ node }, { node }], Array(8).fill({ node }), {}, [{ node: { ...node, endLine: 0 } }]]) {
    await set(entries); await assert.rejects(broker.symbol("fetchTicket"), /missing|ambiguous|malformed|truncated|identity/);
  }
  for (const filePath of ["../outside", "/etc/passwd", "a/../unit.mjs", "a\\b", "--file", "unit.mjs\n"]) {
    await set([{ node: { ...node, filePath } }]); await assert.rejects(broker.symbol("fetchTicket"), /identity|escapes/);
  }
  await symlink(join(repository, "unit.mjs"), join(repository, "alias.mjs"));
  await set([{ node: { ...node, filePath: "alias.mjs" } }]); await assert.rejects(broker.symbol("fetchTicket"), /escapes/);
  for (const text of ["", "Symbol not found", report.replace("2\t", "4\t"), report + "... (output truncated)", report + "x".repeat(4000), report + "auto-sync is DISABLED"]) {
    await set([{ node }], text); await assert.rejects(broker.symbol("fetchTicket"), /empty|partial|incompatible|exceeds|stale/);
  }
  await set([{ node: { ...node, updatedAt: 1 } }]); await assert.rejects(broker.verify(sources), /identity changed/);
  await set([{ node }], report.replace("完整", "changed")); await assert.rejects(broker.verify(sources), /report changed/);
  await set(); const forged = structuredClone(sources); forged[0].symbol.file = "other.mjs";
  await assert.rejects(broker.verify(forged), /passage changed/);
  const controller = new AbortController(); const cancelled = await createBroker({ root, project: "example", codegraph: tool, signal: controller.signal }); t.after(cancelled.close);
  controller.abort(); await assert.rejects(cancelled.symbol("fetchTicket"));
});
test("catalog defaults off and cannot enable tools after the retrieval budget is exhausted", async () => {
  const original = await investigate({ question: "Find A", broker: fake(), decide: async (payload) => {
    assert(!Object.hasOwn(payload, "available_actions"));
    return reply({ action: "finish", handles: [1] });
  } });
  assert(!Object.hasOwn(original.trace[1], "available_actions"));
  await investigate({ question: "Find A", broker: fake(), maxTools: 1, toolCatalog: true, decide: async (payload, schema) => {
    assert.deepEqual(payload.available_actions.map(entry => entry.schema), schema.oneOf);
    assert.deepEqual(schema.oneOf.map(choice => choice.properties.action.const), ["finish"]);
    return reply({ action: "finish", handles: [1] });
  } });
  await assert.rejects(investigate({ question: "Find A", broker: fake(), toolCatalog: "true", decide: async () => {} }), /tool-catalog/);
});
test("catalog bytes count toward the existing decision input limit before model access", async () => {
  const broker = fake(); broker.search = async () => [{ ...note, quote: "x".repeat(15_300) }];
  let calls = 0;
  const decide = async () => { calls++; return reply({ action: "finish", handles: [1] }); };
  assert.equal((await investigate({ question: "Find A", broker, decide })).status, "finished");
  assert.equal((await investigate({ question: "Find A", broker, decide, toolCatalog: true })).reason, "input_budget");
  assert.equal(calls, 1);
});
test("invalid model output, cancellation, drift and verifier failure retain failed trace", async () => {
  for (const mode of ["json", "cancel", "drift", "verify"]) {
    const broker = fake(); const controller = new AbortController(); let count = 0;
    if (mode === "drift") broker.search = async () => [{ ...note, fingerprint: "sha256:" + count++ }];
    if (mode === "verify") broker.verify = async () => { throw new Error("stale"); };
    await assert.rejects(investigate({ question: "Find A", broker, signal: controller.signal, decide: async () => {
      if (mode === "cancel") controller.abort();
      return mode === "json" ? { answer: "invalid", metrics: {} } : reply(mode === "drift" ? { action: "search", query: "new" } : { action: "finish", handles: [1] });
    } }), error => !!error.investigationTrace && error.investigationTrace.status === "failed");
  }
});
test("actual Rust-backed broker reads/checks synthetic notes and refuses source drift", async t => {
  const out = await mkdtemp(join(tmpdir(), "akasha-investigator-test-")); t.after(() => rm(out, { recursive: true, force: true }));
  const corpus = JSON.parse(await readFile(new URL("../tests/fixtures/librarian-benchmark/corpus.json", import.meta.url)));
  const root = await createFixture(out, corpus); const broker = await createBroker({ root, project: "example" }); t.after(broker.close);
  const sources = await broker.search("durable lease ledger"); assert(sources.length && sources.every(source => source.kind === "note"));
  await broker.verify(sources);
  await writeFile(join(root, sources[0].id), "changed");
  await assert.rejects(broker.verify(sources));
});
test("CodeGraph is explicitly bound to resolved repository and reports are rechecked", async t => {
  const out = await mkdtemp(join(tmpdir(), "akasha-investigator-graph-")); t.after(() => rm(out, { recursive: true, force: true }));
  const root = await createFixture(out, { notes: [], cases: [] });
  const repository = join(out, "repository"); const tool = join(out, "graph"); const argvFile = join(out, "argv.json");
  await writeFile(tool, `#!/usr/bin/env node\nimport {writeFileSync} from 'node:fs';writeFileSync(${JSON.stringify(argvFile)},JSON.stringify(process.argv.slice(2)));console.log('Exact source observation');\n`);
  await chmod(tool, 0o700);
  await assert.rejects(createBroker({ root, project: "example", codegraph: tool }), /ENOENT|index/);
  await mkdir(join(repository, ".codegraph"));
  const broker = await createBroker({ root, project: "example", codegraph: tool }); t.after(broker.close);
  const sources = await broker.explore("SomeSymbol"); await broker.verify(sources);
  assert.deepEqual(JSON.parse(await readFile(argvFile)), ["--no-color", "explore", "SomeSymbol", "--path", repository, "--max-files", "2"]);
  assert.equal(sources[0].kind, "codegraph_report");
  await broker.explore("AnotherSymbol"); // Different queries may legitimately return identical reports.
  await broker.verify(sources);
  assert.equal(JSON.parse(await readFile(argvFile))[2], "SomeSymbol");
  await writeFile(tool, "#!/usr/bin/env node\nconsole.log('x'.repeat(3999) + '😀rest');\n");
  const unicode = await broker.explore("Unicode");
  assert(unicode[0].quote.endsWith("😀")); assert(unicode[0].partial_body);
  assert.equal(Buffer.from(unicode[0].quote).toString("utf8"), unicode[0].quote);
  await broker.verify(unicode);
  await writeFile(tool, "#!/bin/sh\necho changed\n");
  await assert.rejects(broker.verify(sources), /changed/);
  await writeFile(tool, "#!/bin/sh\nexit 0\n");
  await assert.rejects(broker.explore("Empty"), /empty report/);
});
test("broker rechecks one full report for disjoint passages and refuses forged or stale observations", async t => {
  const out = await mkdtemp(join(tmpdir(), "akasha-investigator-passages-")); t.after(() => rm(out, { recursive: true, force: true }));
  const root = await createFixture(out, { notes: [], cases: [] });
  const repository = join(out, "repository"); await mkdir(join(repository, ".codegraph"));
  const tool = join(out, "graph"); const reportPath = join(out, "report.txt"); const calls = join(out, "calls");
  const report = "intro\n".repeat(1000) + "function acquireTicket() { rejectOldGeneration(); }\n"
    + "between\n".repeat(1000) + "function releaseTicket() { wakeNextWaiter(); }\n";
  await writeFile(reportPath, report);
  await writeFile(tool, `#!/usr/bin/env node\nimport {appendFileSync,readFileSync} from 'node:fs';appendFileSync(${JSON.stringify(calls)},'call\\n');process.stdout.write(readFileSync(${JSON.stringify(reportPath)}));\n`);
  await chmod(tool, 0o700);
  const broker = await createBroker({ root, project: "example", codegraph: tool }); t.after(broker.close);
  const sources = await broker.explore("acquireTicket releaseTicket");
  assert.equal(sources.length, 2); assert(sources.every(source => source.partial_body));
  assert(sources[0].quote.includes("rejectOldGeneration()")); assert(sources[1].quote.includes("wakeNextWaiter()"));
  await broker.verify(sources.map((source, index) => ({ ...source, handle: index + 1 })));
  assert.equal((await readFile(calls, "utf8")).trim().split("\n").length, 2);
  for (const change of [source => { source.quote += "invented"; }, source => { source.report_range.start++; },
    source => { source.omitted_code_points = 0; }, source => { source.fingerprint = "sha256:forged"; }]) {
    const forged = structuredClone(sources); change(forged[0]);
    await assert.rejects(broker.verify(forged), /passage changed/);
  }
  for (const banner of ["CodeGraph auto-sync is DISABLED", "changed on disk after the last index sync", "Some files referenced below were edited"]) {
    await writeFile(reportPath, report + banner);
    await assert.rejects(broker.explore("acquireTicket"), /stale/);
    await assert.rejects(broker.verify(sources), /stale/);
  }
  await writeFile(reportPath, report + "changed after selection");
  await assert.rejects(broker.verify(sources), /report changed/);
});
