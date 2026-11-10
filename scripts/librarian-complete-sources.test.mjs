import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { setup, snapshot } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding } from "./local-onboarding.mjs";
import { askLibrarian } from "./local-librarian.mjs";

const cli = resolve("target/debug/akasha");
const fact = "FlightPath stores waypoints in a vector.";
async function fixture(t, scenario = "provenance") {
  const directory = mkdtempSync(join(tmpdir(), "akasha-complete-sources-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "onboarding 世界"));
  const run = (args, okay = true) => {
    const result = spawnSync(cli, ["--root", f.root, "--project", "first-run", ...args], { encoding: "utf8" });
    if (okay) assert.equal(result.status, 0, result.stderr);
    return result;
  };
  // Synthetic content is explicitly source-reviewed before the ordinary approved fixture write.
  const draft = JSON.parse((await draftLocalOnboarding({ root: f.root, project: "first-run",
    sources: ["src/lib.rs"], autoPlan: true, json: true }, async input => ({
    answer: JSON.stringify({ schema_version: 5, notes: [{ slot: input.planned_note.slot,
      evidence: [{ kind: "fact", claim: fact, sources: [{ source: 1, excerpt: 1 }] }] }] }),
  }))).output);
  const onboardingPacket = join(directory, "onboarding.json"), proposal = join(directory, "proposal.json");
  writeFileSync(onboardingPacket, run(["--json", "local-onboarding-prepare", "--source", "src/lib.rs"]).stdout);
  writeFileSync(proposal, JSON.stringify(draft.proposal));
  run(["local-onboarding-apply", "--packet", onboardingPacket, "--proposal", proposal,
    "--approved-preview-id", draft.preview.preview_id]);
  const model = join(directory, "model.gguf"), server = join(directory, "worker");
  writeFileSync(model, "GGUF" + scenario);
  copyFileSync(new URL("./fixtures/librarian-worker.mjs", import.meta.url), server); chmodSync(server, 0o700);
  const options = { root: f.root, project: "first-run", question: "FlightPath vector", allSources: true,
    json: true, server, model, modelSha256: createHash("sha256").update("GGUF" + scenario).digest("hex") };
  return { ...f, directory, run, options, model };
}

test("onboarded memory remains visible after a model selects nothing; highlights and default output remain separate", async t => {
  const f = await fixture(t, "empty"), memory = snapshot(f.root), repository = snapshot(f.repository);
  const result = await askLibrarian(f.options), review = JSON.parse(result.output);
  assert.equal(review.kind, "source_review"); assert.equal(result.metrics.mode, "source_review");
  assert.deepEqual(review.highlighted_citations, []);
  assert.equal(review.sources.matches.length, 1);
  const passage = review.sources.matches[0];
  assert(passage.quote.includes(fact)); assert.equal(passage.status, "draft");
  assert(readFileSync(join(f.root, passage.id), "utf8").includes(passage.quote));
  const plain = (await askLibrarian({ ...f.options, json: false })).output;
  assert(plain.includes("Additional retrieved sources (not highlighted):\n[1]"));
  assert(plain.includes("Recorded status: draft") && plain.includes(fact));
  assert(plain.includes("not an answer or a completeness verdict"));
  const original = JSON.parse((await askLibrarian({ ...f.options, allSources: false })).output);
  assert.equal(original.kind, "source_passages"); assert.deepEqual(original.matches, []);
  assert.deepEqual(snapshot(f.root), memory); assert.deepEqual(snapshot(f.repository), repository);
  const events = readFileSync(f.model + ".events", "utf8").trim().split("\n").map(JSON.parse);
  assert.equal(events.filter(e => e.path === "/completion").length, 3);
  for (const started of events.filter(e => e.start)) {
    assert.throws(() => process.kill(started.start, 0), { code: "ESRCH" });
    assert(!existsSync(dirname(started.args[started.args.indexOf("--api-key-file") + 1])));
  }
});

test("highlighted source is rendered once and empty retrieval skips the worker", async t => {
  const f = await fixture(t);
  const review = JSON.parse((await askLibrarian(f.options)).output);
  assert.deepEqual(review.highlighted_citations, [1]); assert.equal(review.sources.matches.length, 1);
  const plain = (await askLibrarian({ ...f.options, json: false })).output;
  assert(plain.includes("Highlighted sources:\n[1]"));
  assert.equal(plain.split(fact).length, 2);
  const empty = await askLibrarian({ ...f.options, question: "unfindableword", server: "/missing", model: "/missing" });
  assert.equal(empty.metrics.skipped, true);
  assert.deepEqual(JSON.parse(empty.output).sources.matches, []);
  const otherRepository = join(f.directory, "other-repository"); mkdirSync(otherRepository);
  const initialized = spawnSync(cli, ["--root", f.root, "init", "other-project"], { cwd: otherRepository, encoding: "utf8" });
  assert.equal(initialized.status, 0, initialized.stderr);
  const other = await askLibrarian({ ...f.options, project: "other-project" });
  assert.equal(JSON.parse(other.output).sources.project, "other-project");
  assert.deepEqual(JSON.parse(other.output).sources.matches, []);
  assert.equal(other.metrics.skipped, true);
  await assert.rejects(askLibrarian({ ...f.options, allSources: "yes" }), /Invalid all-sources/);
  for (const mode of ["answer", "provenance"]) await assert.rejects(askLibrarian({ ...f.options, [mode]: true }), /separately/);
});

test("complete-source CLI rejects invalid selection, altered packet and conflicting controls without stdout", async t => {
  const f = await fixture(t), memory = snapshot(f.root);
  const packet = JSON.parse(f.run(["--json", "librarian-prepare", f.options.question]).stdout);
  const packetPath = join(f.directory, "packet.json"), selectionPath = join(f.directory, "selection.json");
  writeFileSync(packetPath, JSON.stringify(packet));
  const args = ["librarian-check-sources", "--packet", packetPath, "--selection", selectionPath, "--all-sources"];
  for (const selection of ["bad", '{"selections":[{"citation":9}]}', '{"selections":[{"citation":1,"quote":"FlightPath"}]}']) {
    writeFileSync(selectionPath, selection);
    const refused = f.run(args, false); assert.notEqual(refused.status, 0); assert.equal(refused.stdout, "");
  }
  writeFileSync(selectionPath, '{"selections":[]}');
  const plain = f.run(args).stdout;
  assert(plain.includes(fact) && plain.includes("Additional retrieved sources"));
  packet.evidence[0].excerpt += "forged"; writeFileSync(packetPath, JSON.stringify(packet));
  for (const extra of [[], ["--provenance"]]) {
    const refused = f.run([...args, ...extra], false); assert.notEqual(refused.status, 0); assert.equal(refused.stdout, "");
  }
  assert.deepEqual(snapshot(f.root), memory);
});

test("complete-source requests cancel with owned cleanup", async t => {
  const f = await fixture(t, "hang"), memory = snapshot(f.root);
  const controller = new AbortController();
  const request = askLibrarian({ ...f.options, signal: controller.signal });
  const refused = assert.rejects(request, /cancelled|abort/iu);
  for (let i = 0; i < 200; i++) {
    if (existsSync(f.model + ".events") && readFileSync(f.model + ".events", "utf8").includes('"/completion"')) break;
    await delay(20);
  }
  assert(readFileSync(f.model + ".events", "utf8").includes('"/completion"'));
  controller.abort(new Error("cancelled complete-source request")); await refused;
  const started = JSON.parse(readFileSync(f.model + ".events", "utf8").split("\n")[0]);
  assert.throws(() => process.kill(started.start, 0), { code: "ESRCH" });
  assert(!existsSync(dirname(started.args[started.args.indexOf("--api-key-file") + 1])));
  await assert.rejects(askLibrarian({ ...f.options, signal: AbortSignal.abort() }));
  assert.deepEqual(snapshot(f.root), memory);
});
