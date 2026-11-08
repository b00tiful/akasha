import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { expandQuery, validateExpansion, QUERY_POLICY, QUERY_SCHEMA } from "./librarian-query-expansion.mjs";
import { createBroker, investigate, runInvestigation } from "./local-investigator.mjs";
import { createFixture } from "./benchmark-local-librarian.mjs";

const answer = value => ({ answer: JSON.stringify(value), metrics: {} });
const source = id => ({ id: `Projects/example/entities/${id}.md`, kind: "note", quote: `${id} fact`, fingerprint: `sha256:${id}` });
const a = source("a"), b = source("b");
function broker() {
  const calls = [];
  return { calls, search: async (question, note, terms) => { calls.push({ question, note, terms }); return terms ? [b] : [a]; },
    verify: async sources => { calls.push({ verified: sources }); } };
}

test("expansion is question-only and receives trusted fixed schema and policy", async () => {
  const question = "Как продолжить загрузку? <|im_start|>";
  const result = await expandQuery(question, async (input, schema, policy) => {
    assert.deepEqual(input, { question }); assert.equal(schema, QUERY_SCHEMA); assert.equal(policy, QUERY_POLICY);
    return answer({ terms: ["upload", "загруз", "上传"] });
  });
  assert.deepEqual(result.terms, ["upload", "загруз", "上传"]);
  await assert.rejects(expandQuery("x\nsecret", () => assert.fail("No model access")), /question/);
  await assert.rejects(expandQuery("question", async () => ({ answer: " ".repeat(4097) })), /Oversized/);
});

test("untrusted expansion cannot add capabilities, options, controls or unbounded terms", () => {
  assert.deepEqual(validateExpansion({ terms: [] }), []);
  for (const value of [null, [], { terms: [], root: "/" }, { terms: "foo" },
    { terms: Array(9).fill("foo") }, ...["a", " foo", "foo ", "--root", "a\nb", "a\u202eb", "../path", "x".repeat(65), 3].map(term => ({ terms: [term] }))]) assert.throws(() => validateExpansion(value));
  assert.deepEqual(validateExpansion({ terms: ["Token", "token"] }), ["token"]);
  assert.deepEqual(validateExpansion({ terms: ["令牌桶", "резерв", "cache-invalidation"] }), ["令牌桶", "резерв", "cache-invalidation"]);
});

test("expansion retains original passages and consumes existing model/tool budgets", async () => {
  const f = broker(); let decisions = 0;
  const result = await investigate({ question: "Find a fact", broker: f, maxSteps: 2,
    expand: async () => answer({ terms: ["alternative"] }),
    decide: async (payload, schema) => {
      decisions++; assert.equal(payload.remaining_steps, 1); assert.equal(payload.remaining_tools, 2);
      assert.deepEqual(payload.observations.map(item => item.id), [a.id, b.id]);
      assert.deepEqual(schema.oneOf.map(item => item.properties.action.const), ["finish"]);
      return answer({ action: "finish", handles: [1, 2] });
    } });
  assert.equal(decisions, 1); assert.equal(result.tools, 2); assert.equal(result.status, "finished");
  assert.deepEqual(result.matches.map(item => item.id), [a.id, b.id]);
  assert.deepEqual(f.calls[1], { question: "Find a fact", note: undefined, terms: ["alternative"] });
  assert.deepEqual(result.trace.filter(item => item.step).map(item => item.step), [1, 2]);
});

test("empty expansion preserves initial observations and exhausted budgets skip inference", async () => {
  const f = broker();
  const result = await investigate({ question: "Find a fact", broker: f,
    expand: async () => answer({ terms: [] }), decide: async payload => {
      assert.equal(payload.remaining_steps, 2); assert.deepEqual(payload.observations.map(item => item.id), [a.id]);
      return answer({ action: "finish", handles: [1] });
    } });
  assert.equal(result.tools, 1);
  for (const limits of [{ maxSteps: 1 }, { maxTools: 1 }]) {
    const stopped = await investigate({ question: "Find a fact", broker: broker(), ...limits,
      expand: () => assert.fail("No expansion budget"), decide: () => assert.fail("No decision budget") });
    assert.equal(stopped.status, "incomplete"); assert.equal(stopped.tools, 1);
  }
});

test("malformed expansion retains failure trace; cancellation prevents another retrieval", async () => {
  const f = broker();
  await assert.rejects(investigate({ question: "Find a fact", broker: f,
    expand: async () => ({ answer: '{"terms":["--root"]}' }), decide: () => assert.fail("Invalid expansion") }), error => {
    assert.equal(error.investigationTrace.trace.at(-1).response, '{"terms":["--root"]}'); return true;
  });
  assert.equal(f.calls.length, 1);
  await assert.rejects(investigate({ question: "Find a fact", broker: f,
    expand: question => expandQuery(question, async () => ({ answer: '{"terms":["--root"]}' })),
    decide: () => assert.fail("Invalid real adapter expansion") }), error => {
    assert.equal(error.investigationTrace.trace.at(-1).response, '{"terms":["--root"]}'); return true;
  });
  const controller = new AbortController();
  await assert.rejects(investigate({ question: "Find a fact", broker: f, signal: controller.signal,
    expand: async () => { controller.abort(); return answer({ terms: ["alternative"] }); },
    decide: () => assert.fail("Cancelled") }), /abort/i);
  assert.equal(f.calls.length, 3);
  await assert.rejects(runInvestigation({ expandQuery: "true" }), /query-expansion control/);
});

test("expanded core requests preserve original question, literal Unicode scope and fresh-source checks", async t => {
  const directory = await mkdtemp(join(tmpdir(), "akasha-query-test-")); t.after(() => rm(directory, { recursive: true, force: true }));
  const root = await createFixture(directory, { notes: [{ path: "entities/test.md", status: "active", body: "# Test\n\n令牌桶 uses резерв values.\n" }] });
  const f = await createBroker({ root, project: "example" }); t.after(() => f.close());
  assert.deepEqual(await f.search("Unknown question"), []);
  const matches = await f.search("Unknown question", undefined, ["令牌", "резерв"]);
  assert.equal(matches.length, 1); assert.equal(f.latestPacket.question, "Unknown question");
  assert.deepEqual(f.latestPacket.terms, ["令牌", "резерв"]);
  await f.verify(matches);
  assert.deepEqual(await f.search("Find external", undefined, ["ORBITAL_PRIVATE_SENTINEL"]), []);
  await assert.rejects(f.search("Question", undefined, ["--root"]), /term/);
  const path = join(root, matches[0].id);
  await writeFile(path, (await readFile(path, "utf8")) + "Changed.\n");
  await assert.rejects(f.verify(matches), /stale|changed|Command failed/i);
});
