import assert from "node:assert/strict";
import test from "node:test";
import { extractSupportedAnswer, PLAN_POLICY, READ_POLICY } from "./librarian-support.mjs";

const packet = { question: "How is the report formatted and which encryption protects it?", evidence: [
  { citation: 1, status: "draft", excerpt: "# Report\nRows use tabs and end with a newline.\nJSON output is only a proposal.\n", excerpt_truncated: false },
  { citation: 2, status: "active", excerpt: "Settings default to 3.\n", excerpt_truncated: false },
] };
const parts = ["How is the report formatted?", "Which encryption protects the report?"];
const quote = "Rows use tabs and end with a newline.";
const response = { uncovered_question: false, parts: [
  { part: 1, sufficient: true, quotes: [quote] }, { part: 2, sufficient: false, quotes: [] },
] };
const reply = value => ({ answer: JSON.stringify(value), metrics: { synthetic: true } });
const reader = (plan = { parts }, support = response) => async input => reply(input.sources ? support : plan);

test("question-only planning and exact extraction preserve known parts alongside explicit gaps", async () => {
  let calls = 0;
  const result = await extractSupportedAnswer(packet, {}, async (input, schema, policy) => {
    calls++;
    if (calls === 1) { assert.deepEqual(input, { question: packet.question }); assert.equal(policy, PLAN_POLICY); return reply({ parts }); }
    assert.equal(input.question, packet.question); assert.deepEqual(input.parts.map(p => p.question), parts);
    assert.equal(policy, READ_POLICY); assert.equal(input.sources[0].status, "draft");
    assert.equal(schema.properties.parts.items.properties.quotes.items.type, "string");
    assert(input.sources.every(s => !("citation" in s)));
    return reply(response);
  });
  assert.equal(calls, 2);
  assert.deepEqual(JSON.parse(result.answer), { insufficient_evidence: true, claims: [{ text: quote, citations: [1] }] });
  assert.deepEqual(result.metrics.parts, [{ part: 1, sufficient: true, quotes: 1 }, { part: 2, sufficient: false, quotes: 0 }]);
});

test("global coverage gap, unknowns and duplicate support across parts are handled conservatively", async () => {
  const supported = { part: 2, sufficient: true, quotes: [quote] };
  const known = await extractSupportedAnswer(packet, {}, reader(undefined, { ...response, parts: [response.parts[0], supported] }));
  assert.equal(JSON.parse(known.answer).insufficient_evidence, false); assert.equal(JSON.parse(known.answer).claims.length, 1);
  const missed = await extractSupportedAnswer(packet, {}, reader(undefined, { ...response, uncovered_question: true, parts: [response.parts[0], supported] }));
  assert.equal(JSON.parse(missed.answer).insufficient_evidence, true);
  const unknown = await extractSupportedAnswer(packet, {}, reader(undefined, { ...response, parts: response.parts.map(p => ({ ...p, sufficient: false, quotes: [] })) }));
  assert.deepEqual(JSON.parse(unknown.answer), { insufficient_evidence: true, claims: [] });
});

test("invalid or expanding plans refuse before evidence is given to the model", async () => {
  for (const plan of [null, [], { parts: [] }, { parts: [" "] }, { parts: ["a", " A "] },
    { parts: Array(5).fill("q") }, { parts: ["a".repeat(513)] }, { parts: ["a\nb"] }, { parts, extra: "invented" }]) {
    let calls = 0;
    await assert.rejects(extractSupportedAnswer(packet, {}, async input => {
      assert(!input.sources); calls++; return reply(plan);
    }), /question-part plan/);
    assert.equal(calls, 1);
  }
});

test("every question part must occur exactly once with explicit support state", async () => {
  for (const support of [null, {}, { ...response, parts: response.parts.slice(0, 1) },
    { ...response, parts: [response.parts[0], response.parts[0]] }, { ...response, parts: response.parts.toReversed() },
    { ...response, parts: [{ part: 1, sufficient: true, quotes: [] }, response.parts[1]] },
    { ...response, uncovered_question: "false" }, { ...response, invented_answer: "AES" },
    { ...response, parts: [{ ...response.parts[0], answer: "AES" }, response.parts[1]] }]) {
    await assert.rejects(extractSupportedAnswer(packet, {}, reader(undefined, support)), /coverage|part support/);
  }
});

test("invented, ambiguous, altered, clipped and heading-only quotations refuse", async () => {
  const quotes = [{ citation: 99, quote }, "Rows use spaces.", quote.toUpperCase(), "# Report", "\n",
    "\u001b[31m", "\u202e", "x".repeat(601), { text: "extra" }];
  for (const q of quotes) await assert.rejects(extractSupportedAnswer(packet, {}, reader(undefined,
    { ...response, parts: [{ ...response.parts[0], quotes: [q] }, response.parts[1]] })), /quote/);
  await assert.rejects(extractSupportedAnswer(packet, {}, reader(undefined,
    { ...response, parts: [{ ...response.parts[0], quotes: [quote, quote] }, response.parts[1]] })), /Duplicate/);
  const ambiguous = { ...packet, evidence: [{ ...packet.evidence[0], excerpt: quote + "\n" + quote }] };
  await assert.rejects(extractSupportedAnswer(ambiguous, {}, reader()), /exactly once/);
  const crossSource = { ...packet, evidence: [packet.evidence[0], { ...packet.evidence[1], excerpt: quote }] };
  await assert.rejects(extractSupportedAnswer(crossSource, {}, reader()), /exactly once/);
  const clipped = { ...packet, evidence: [{ ...packet.evidence[0], excerpt: "whole\n" + quote, excerpt_truncated: true }] };
  await assert.rejects(extractSupportedAnswer(clipped, {}, reader()), /clipped/);
});

test("Unicode quote bytes stay intact and cancellation between phases prevents evidence reading", async () => {
  const unicode = { ...packet, evidence: [{ ...packet.evidence[0], excerpt: "Срок хранения — 14 дней.\n" }] };
  const support = { ...response, parts: [{ ...response.parts[0], quotes: ["Срок хранения — 14 дней."] }, response.parts[1]] };
  const result = await extractSupportedAnswer(unicode, {}, reader(undefined, support));
  assert.equal(JSON.parse(result.answer).claims[0].text, "Срок хранения — 14 дней.");
  const controller = new AbortController(); let calls = 0;
  await assert.rejects(extractSupportedAnswer(packet, { signal: controller.signal }, async () => {
    calls++; controller.abort(); return reply({ parts });
  }), /abort/i);
  assert.equal(calls, 1);
});
