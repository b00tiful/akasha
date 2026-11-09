import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { setup, snapshot } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding } from "./local-onboarding.mjs";
import { askLibrarian } from "./local-librarian.mjs";

const cli = resolve("target/debug/akasha");
const fact = "FlightPath stores waypoints in a vector.";
async function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "akasha-provenance-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "review"));
  const run = (args, okay = true) => {
    const result = spawnSync(cli, ["--root", f.root, "--project", "first-run", ...args], { encoding: "utf8" });
    if (okay) assert.equal(result.status, 0, result.stderr);
    return result;
  };
  const draft = JSON.parse((await draftLocalOnboarding({ root: f.root, project: "first-run",
    sources: ["src/lib.rs"], autoPlan: true, json: true }, async input => ({
    answer: JSON.stringify({ schema_version: 5, notes: [{ slot: input.planned_note.slot, evidence: [
      { kind: "fact", claim: fact, sources: [{ source: 1, excerpt: 1 }] },
      { kind: "inference", claim: "The vector suggests insertion-order storage.",
        rationale: "This is an interpretation of the field representation.", sources: [{ source: 1, excerpt: 1 }] },
      { kind: "unknown", claim: "Deployment is unspecified.", rationale: "No deployment source was selected.", sources: [] },
    ] }] }),
  }))).output);
  const packetPath = join(directory, "onboarding.json"), proposalPath = join(directory, "proposal.json");
  writeFileSync(packetPath, run(["--json", "local-onboarding-prepare", "--source", "src/lib.rs"]).stdout);
  writeFileSync(proposalPath, JSON.stringify(draft.proposal));
  run(["local-onboarding-apply", "--packet", packetPath, "--proposal", proposalPath, "--approved-preview-id", draft.preview.preview_id]);
  const packet = JSON.parse(run(["--json", "librarian-prepare", "FlightPath vector"]).stdout);
  const packetFile = join(directory, "packet.json"), selectionFile = join(directory, "selection.json");
  writeFileSync(packetFile, JSON.stringify(packet));
  writeFileSync(selectionFile, '{"selections":[{"citation":1}]}');
  const inspect = (extra = ["--json"]) => run([...extra, "librarian-check-sources", "--packet", packetFile,
    "--selection", selectionFile, "--provenance"]);
  return { ...f, directory, run, packet, packetFile, selectionFile, inspect };
}

test("onboarding claims retain kind, rationale, physical source lines and draft status through provenance", async t => {
  const f = await fixture(t), memory = snapshot(f.root), repository = snapshot(f.repository);
  const report = JSON.parse(f.inspect().stdout);
  assert.equal(report.kind, "source_provenance");
  assert.equal(report.sources.matches[0].status, "draft");
  const note = report.notes[0];
  assert.equal(note.evidence_state, "recorded"); assert.equal(note.unmatched_claims, 0);
  assert.deepEqual(note.claims.map(c => c.recorded_kind), ["fact", "inference", "unknown"]);
  const source = note.claims[0].sources[0];
  assert.equal(source.freshness, "unchanged"); assert.equal(source.excerpt_state, "complete");
  assert.equal(source.recorded_fingerprint, source.observed_fingerprint);
  const text = readFileSync(join(f.repository, source.path), "utf8");
  assert.equal(source.excerpt, text.split(/(?<=\n)/u).slice(source.line_start - 1, source.line_end).join(""));
  assert.equal(source.recorded_fingerprint, "sha256:" + createHash("sha256").update(text).digest("hex"));
  const saved = readFileSync(join(f.root, report.sources.matches[0].id), "utf8");
  assert(saved.split("\n")[note.claims[0].note_line - 1].includes(fact));
  assert.equal(note.claims[2].sources.length, 0); assert(note.claims[1].rationale.includes("interpretation"));
  const plain = f.inspect([]).stdout;
  assert.match(plain, /Repository source: "src\/lib.rs" lines \d+–\d+/u);
  assert(plain.includes("Freshness: unchanged") && plain.includes("Recorded inference") && plain.includes("Recorded unknown"));
  assert(plain.includes("unchanged bytes do not establish factual support"));
  const defaultOutput = JSON.parse(f.run(["--json", "librarian-check-sources", "--packet", f.packetFile, "--selection", f.selectionFile]).stdout);
  assert.equal(defaultOutput.kind, "source_passages"); assert(!("notes" in defaultOutput));
  assert.deepEqual(snapshot(f.root), memory); assert.deepEqual(snapshot(f.repository), repository);
});

test("source edits/deletion warn without showing stale ranges; exact quoted claims alone are inspected", async t => {
  const f = await fixture(t), memory = snapshot(f.root), path = join(f.repository, "src/lib.rs");
  writeFileSync(path, "// changed repository source\n");
  let report = JSON.parse(f.inspect().stdout);
  let source = report.notes[0].claims[0].sources[0];
  assert.equal(source.freshness, "changed"); assert.equal(source.excerpt, null);
  assert.notEqual(source.observed_fingerprint, source.recorded_fingerprint);
  assert(f.inspect([]).stdout.includes("Freshness: changed"));
  unlinkSync(path);
  report = JSON.parse(f.inspect().stdout); source = report.notes[0].claims[0].sources[0];
  assert.equal(source.freshness, "missing"); assert.equal(source.excerpt, null);
  writeFileSync(f.selectionFile, JSON.stringify({ selections: [{ citation: 1, quote: fact }] }));
  report = JSON.parse(f.inspect().stdout);
  assert.equal(report.notes[0].claims.length, 1); assert.equal(report.notes[0].unmatched_claims, 2);
  const forged = structuredClone(f.packet); forged.evidence[0].excerpt += "invented";
  writeFileSync(f.packetFile, JSON.stringify(forged));
  const refused = f.run(["librarian-check-sources", "--packet", f.packetFile, "--selection", f.selectionFile, "--provenance"], false);
  assert.notEqual(refused.status, 0); assert.equal(refused.stdout, "");
  assert.deepEqual(snapshot(f.root), memory);
});

test("local librarian --provenance uses the owned selector, core inspection and cleanup", async t => {
  const f = await fixture(t), model = join(f.directory, "model.gguf"), server = join(f.directory, "worker");
  writeFileSync(model, "GGUFprovenance"); copyFileSync(new URL("./fixtures/librarian-worker.mjs", import.meta.url), server); chmodSync(server, 0o700);
  const memory = snapshot(f.root), repository = snapshot(f.repository);
  const options = { root: f.root, project: "first-run", question: "FlightPath vector", server, model,
    modelSha256: createHash("sha256").update("GGUFprovenance").digest("hex"), provenance: true, json: true };
  const result = await askLibrarian(options);
  assert.equal(result.metrics.mode, "provenance");
  assert.equal(JSON.parse(result.output).notes[0].claims[0].sources[0].freshness, "unchanged");
  const events = readFileSync(model + ".events", "utf8").trim().split("\n").map(JSON.parse);
  const started = events.find(e => e.start);
  assert.throws(() => process.kill(started.start, 0), { code: "ESRCH" });
  assert(!existsSync(dirname(started.args[started.args.indexOf("--api-key-file") + 1])));
  assert.deepEqual(snapshot(f.root), memory); assert.deepEqual(snapshot(f.repository), repository);
  await assert.rejects(askLibrarian({ ...options, answer: true }), /separate experimental/);
  await assert.rejects(askLibrarian({ ...options, provenance: "yes" }), /Invalid provenance/);
});
