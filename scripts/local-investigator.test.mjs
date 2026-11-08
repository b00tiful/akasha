import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile, chmod } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createFixture } from "./benchmark-local-librarian.mjs";
import { actionSchema, createBroker, investigate, validateAction } from "./local-investigator.mjs";

const note = { kind: "note", id: "Projects/example/entities/a.md", quote: "A recorded fact.", fingerprint: "sha256:a" };
const reply = action => ({ answer: JSON.stringify(action), metrics: {} });
function fake() {
  const calls = []; return { calls, codegraph: false,
    search: async (query, id) => { calls.push({ query, id }); return [note]; },
    verify: async sources => { calls.push({ verify: sources }); } };
}
test("bounded loop follows a known note and returns only verified original sources", async () => {
  const broker = fake(); let step = 0;
  const result = await investigate({ question: "Find A", broker, decide: async () => reply(step++ ? { action: "finish", handles: [1] } : { action: "read", id: note.id, query: "related" }) });
  assert.equal(result.status, "finished"); assert.equal(result.tools, 2);
  assert.equal(result.matches[0].quote, note.quote); assert.deepEqual(broker.calls[1], { query: "related", id: note.id });
  assert.equal(broker.calls.at(-1).verify.length, 1);
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
