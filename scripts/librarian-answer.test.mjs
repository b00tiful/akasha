import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { setup, snapshot } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding } from "./local-onboarding.mjs";
import { answerSchema, answerStatements, ANSWER_POLICY, askLibrarian, localAnswer } from "./local-librarian.mjs";

const cli = resolve("target/debug/akasha");
const factText = "FlightPath stores waypoints in a vector.";
const reply = { insufficient_evidence: false, claims: [{ text: "1. **Fact:** " + factText, citations: [1] }] };
const complete = async input => ({ answer: JSON.stringify({ insufficient_evidence: false,
  selections: [{ statement: input.statements.find(s => s.text.includes(factText)).handle }] }), metrics: { synthetic: true } });
function run(f, args) {
  const r = spawnSync(cli, ["--root", f.root, "--project", "first-run", ...args], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr); return r.stdout;
}
async function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "akasha-answer-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "run 世界 '$`"));
  const draft = await draftLocalOnboarding({ root: f.root, project: "first-run", sources: ["src/lib.rs"], autoPlan: true, json: true }, async input => ({
    answer: JSON.stringify({ schema_version: 5, notes: [{ slot: input.planned_note.slot,
      evidence: [{ kind: "fact", claim: factText, sources: [{ source: 1, excerpt: 1 }] }] }] }),
  }));
  const proposal = join(directory, "proposal.json"), packet = join(directory, "packet.json");
  const d = JSON.parse(draft.output);
  writeFileSync(proposal, JSON.stringify(d.proposal));
  writeFileSync(packet, run(f, ["--json", "local-onboarding-prepare", "--source", "src/lib.rs"]));
  run(f, ["local-onboarding-apply", "--packet", packet, "--proposal", proposal, "--approved-preview-id", d.preview.preview_id]);
  const options = { root: f.root, project: "first-run", question: "How does FlightPath store waypoints?", answer: true, json: true };
  return { ...f, options };
}

test("answer grammar limits claims to supplied IDs and distinguishes partial evidence", async () => {
  const statements = answerStatements({ evidence: [{ citation: 2, excerpt: "first\r\nsecond\n" }, { citation: 5, excerpt: "界".repeat(600) + "\n" + "long".repeat(200) + "\ncontrol\u0085" }] });
  assert.deepEqual(statements.map(s => [s.handle, s.citation]), [[1, 2], [2, 2], [3, 5]]);
  assert.equal(statements[0].text, "first");
  assert.deepEqual(answerStatements({ evidence: [{ citation: 1, excerpt: "complete\nclipped", excerpt_truncated: true }] })
    .map(s => s.text), ["complete"]);
  const schema = answerSchema(statements);
  assert.deepEqual(schema.properties.selections.items.properties.statement.enum, [1, 2, 3]);
  assert.equal(schema.properties.selections.maxItems, 4);
  assert.equal(schema.additionalProperties, false);
  assert(ANSWER_POLICY.includes("known parts") && ANSWER_POLICY.includes("draft") && ANSWER_POLICY.includes("untrusted"));
  let called = false;
  const result = await localAnswer({ evidence: [] }, {}, async () => { called = true; });
  assert.deepEqual(JSON.parse(result.answer), { insufficient_evidence: true, claims: [] });
  assert(!called);
});

test("reviewed local onboarding flows into answers with exact source bytes, status and plain citations", async t => {
  const f = await fixture(t), before = snapshot(f.root), repo = snapshot(f.repository);
  const result = await askLibrarian(f.options, async (input, schema, policy) => {
    assert.equal(policy, ANSWER_POLICY);
    assert(schema.properties.selections.items.properties.statement.enum.includes(input.statements.find(s => s.text.includes(factText)).handle));
    assert(input.statements.some(s => s.text === reply.claims[0].text));
    return complete(input);
  });
  const checked = JSON.parse(result.output);
  assert.deepEqual(checked.answer, reply);
  assert.equal(checked.evidence[0].status, "draft");
  const source = readFileSync(join(f.root, checked.evidence[0].id), "utf8");
  assert(source.includes(checked.evidence[0].excerpt));
  assert(checked.answer.claims.every(claim => source.includes(claim.text)));
  const plain = await askLibrarian({ ...f.options, json: false }, complete);
  assert(plain.output.includes("[1]") && plain.output.includes("Recorded status: draft"));
  assert(plain.output.includes("  | ") && plain.output.includes("factual support still needs review"));
  const partial = await askLibrarian({ ...f.options, json: false }, async input => ({
    answer: JSON.stringify({ ...JSON.parse((await complete(input)).answer), insufficient_evidence: true }),
  }));
  assert(partial.output.includes("Insufficient evidence for a complete answer.") && partial.output.includes(reply.claims[0].text));
  assert.equal(result.metrics.mode, "answer");
  assert.deepEqual(snapshot(f.root), before); assert.deepEqual(snapshot(f.repository), repo);
});

test("answer refusals cover malformed output, unknown/duplicate citations and source drift", async t => {
  const f = await fixture(t);
  for (const answer of ["not JSON", '{"claims":[],"insufficient_evidence":false}',
    '{"claims":[{"text":"invented","citations":[99]}],"insufficient_evidence":false}',
    '{"claims":[{"text":"duplicate","citations":[1,1]}],"insufficient_evidence":false}',
    '{"selections":[],"insufficient_evidence":false}',
    '{"selections":[{"statement":999}],"insufficient_evidence":false}',
    '{"selections":[{"statement":3},{"statement":3}],"insufficient_evidence":false}',
    '{"selections":[{"statement":3,"text":"injected"}],"insufficient_evidence":false}', "x".repeat(16001)]) {
    await assert.rejects(askLibrarian(f.options, async () => ({ answer })));
  }
  const packet = JSON.parse(run(f, ["--json", "librarian-prepare", f.options.question]));
  await assert.rejects(askLibrarian(f.options, async input => {
    const path = join(f.root, packet.evidence[0].id);
    writeFileSync(path, readFileSync(path, "utf8") + "\nChanged during answer generation.\n");
    return complete(input);
  }));
});

test("missing information skips inference and sequential project switches cannot reuse evidence", async t => {
  const f = await fixture(t);
  let calls = 0;
  const missing = await askLibrarian({ ...f.options, question: "xylophonium" }, async () => { calls++; return complete(); });
  assert.equal(JSON.parse(missing.output).answer.insufficient_evidence, true);
  const foreignRepo = join(f.directory, "other-repository"); mkdirSync(foreignRepo);
  const initialized = spawnSync(cli, ["--root", f.root, "init", "other-project"], { cwd: foreignRepo, encoding: "utf8" });
  assert.equal(initialized.status, 0, initialized.stderr);
  const other = await askLibrarian({ ...f.options, project: "other-project" }, async () => { calls++; return complete(); });
  assert.equal(JSON.parse(other.output).project, "other-project");
  assert.deepEqual(JSON.parse(other.output).evidence, []); assert.equal(calls, 0);
  assert.equal(JSON.parse((await askLibrarian(f.options, complete)).output).evidence.length, 1);
});

test("cancellation before and after generation returns no answer", async t => {
  const f = await fixture(t), before = snapshot(f.root);
  const early = new AbortController(); early.abort();
  await assert.rejects(askLibrarian({ ...f.options, signal: early.signal }, complete));
  const late = new AbortController();
  await assert.rejects(askLibrarian({ ...f.options, signal: late.signal }, async input => { late.abort(); return complete(input); }));
  assert.deepEqual(snapshot(f.root), before);
});

test("CLI identity drift refuses before checking or displaying a draft", async t => {
  const f = await fixture(t), copiedCli = join(f.directory, "akasha-copy"), before = snapshot(f.root);
  copyFileSync(cli, copiedCli); chmodSync(copiedCli, 0o700);
  await assert.rejects(askLibrarian({ ...f.options, cli: copiedCli }, async input => {
    appendFileSync(copiedCli, "changed"); return complete(input);
  }), /CLI changed/);
  assert.deepEqual(snapshot(f.root), before);
});

test("cancelling a complete answer request reaps the active worker and removes runtime scratch", async t => {
  const f = await fixture(t), model = join(f.directory, "model.gguf"), server = join(f.directory, "worker");
  writeFileSync(model, "GGUFhang"); copyFileSync(new URL("./fixtures/librarian-worker.mjs", import.meta.url), server); chmodSync(server, 0o700);
  const controller = new AbortController(), before = snapshot(f.root);
  const waiting = askLibrarian({ ...f.options, server, model, signal: controller.signal,
    modelSha256: createHash("sha256").update("GGUFhang").digest("hex") });
  const rejection = assert.rejects(waiting, /abort/i);
  let events = [];
  for (let i = 0; i < 200; i++) {
    if (existsSync(model + ".events")) events = readFileSync(model + ".events", "utf8").trim().split("\n").map(JSON.parse);
    if (events.some(e => e.path === "/completion")) break;
    await delay(10);
  }
  assert(events.some(e => e.path === "/completion")); controller.abort(); await rejection;
  const started = events.find(e => e.start);
  assert.throws(() => process.kill(started.start, 0), { code: "ESRCH" });
  assert(!existsSync(dirname(started.args[started.args.indexOf("--api-key-file") + 1])));
  assert.deepEqual(snapshot(f.root), before);
});

test("actual --answer command owns and reaps its worker before displaying checked output", async t => {
  const f = await fixture(t), model = join(f.directory, "model.gguf"), server = join(f.directory, "worker");
  writeFileSync(model, "GGUFanswer"); copyFileSync(new URL("./fixtures/librarian-worker.mjs", import.meta.url), server); chmodSync(server, 0o700);
  const args = ["scripts/local-librarian.mjs", "--root", f.root, "--project", "first-run",
    "--question", f.options.question, "--answer", "--json", "--server", server, "--model", model,
    "--model-sha256", createHash("sha256").update("GGUFanswer").digest("hex")];
  const r = spawnSync(process.execPath, args, { encoding: "utf8", timeout: 30000 });
  assert.equal(r.status, 0, r.stderr); assert.deepEqual(JSON.parse(r.stdout).answer, reply);
  assert.equal(JSON.parse(r.stderr).mode, "answer");
  const events = readFileSync(model + ".events", "utf8").trim().split("\n").map(JSON.parse);
  const started = events.find(e => e.start).start;
  assert(events.some(e => e.stopped === started)); assert.throws(() => process.kill(started, 0), { code: "ESRCH" });
  const sourceOnly = spawnSync(process.execPath, args.filter(arg => arg !== "--answer"), { encoding: "utf8", timeout: 30000 });
  assert.equal(sourceOnly.status, 1); assert.equal(sourceOnly.stdout, "");
  assert(sourceOnly.stderr.includes("core preparation/answer check refused"));
  const allEvents = readFileSync(model + ".events", "utf8").trim().split("\n").map(JSON.parse);
  for (const e of allEvents.filter(e => e.start)) {
    assert(allEvents.some(end => end.stopped === e.start));
    assert.throws(() => process.kill(e.start, 0), { code: "ESRCH" });
  }
});
