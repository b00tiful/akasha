import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { lstatSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { setup, snapshot } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding } from "./local-onboarding.mjs";
import { prepareReviewedOnboarding, applyReviewedOnboarding } from "./local-onboarding-review.mjs";

async function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "akasha-review-handoff-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "run"));
  const capture = join(directory, "capture"), output = join(directory, "reviewed 世界 '$`");
  await draftLocalOnboarding({ root: f.root, project: "first-run", discover: ["src"],
    autoPlan: true, saveInputs: capture, json: true }, async input => ({ answer: JSON.stringify({ schema_version: 5,
      notes: [{ slot: input.planned_note.slot, evidence: [{ kind: "fact", claim: "FlightPath stores waypoints in a list.",
        sources: [{ source: 1, excerpt: 1 }] }] }] }) }));
  const plan = JSON.parse(readFileSync(join(capture, "plan.json")));
  plan.notes[0].evidence[0].claim = "FlightPath stores waypoints in a vector.";
  const planPath = join(directory, "reviewed-plan.json"); writeFileSync(planPath, JSON.stringify(plan));
  return { ...f, directory, capture, output, plan: planPath, project: "first-run" };
}

test("review bundle refreshes corrected preview, remains private/read-only and publishes retrievable memory", async t => {
  const f = await fixture(t), before = snapshot(f.root), repo = snapshot(f.repository), raw = snapshot(f.capture);
  const result = await prepareReviewedOnboarding(f);
  assert.deepEqual(snapshot(f.root), before); assert.deepEqual(snapshot(f.repository), repo); assert.deepEqual(snapshot(f.capture), raw);
  assert.equal(lstatSync(f.output).mode & 0o777, 0o700);
  for (const name of readdirSync(f.output)) assert.equal(lstatSync(join(f.output, name)).mode & 0o777, 0o600);
  assert(readFileSync(join(f.output, "review.html"), "utf8").includes("in a vector"));
  assert.deepEqual(JSON.parse(readFileSync(join(f.output, "discovery.json"))), JSON.parse(readFileSync(join(f.capture, "discovery.json"))));
  assert(result.review.includes(result.draft.preview.preview_id));
  const original = JSON.parse(readFileSync(join(f.capture, "draft.json")));
  assert.notEqual(result.draft.preview.preview_id, original.preview.preview_id);
  await assert.rejects(applyReviewedOnboarding({ ...f, bundle: f.output, approvedPreviewId: original.preview.preview_id }));
  assert.deepEqual(snapshot(f.root), before);
  const applied = await applyReviewedOnboarding({ ...f, bundle: f.output, approvedPreviewId: result.draft.preview.preview_id, json: true });
  assert(JSON.parse(applied)); assert.deepEqual(snapshot(f.repository), repo);
  const run = args => spawnSync(resolve("target/debug/akasha"), ["--root", f.root, "--project", f.project, ...args], { encoding: "utf8" });
  assert.equal(run(["validate"]).status, 0);
  const packet = run(["--json", "librarian-prepare", "How are FlightPath waypoints stored?"]);
  assert.equal(packet.status, 0, packet.stderr);
  assert(JSON.parse(packet.stdout).evidence.some(source => source.excerpt.includes("in a vector")));
  const published = snapshot(f.root);
  // Reusing an old packet after publication requires fresh inspection, never implicit retry.
  await assert.rejects(applyReviewedOnboarding({ ...f, bundle: f.output, approvedPreviewId: result.draft.preview.preview_id }));
  assert.deepEqual(snapshot(f.root), published);
});

test("source drift and project mismatch refuse before review output or canonical writes", async t => {
  const f = await fixture(t), before = snapshot(f.root);
  await assert.rejects(prepareReviewedOnboarding({ ...f, project: "another-project" }));
  assert.throws(() => lstatSync(f.output), { code: "ENOENT" });
  writeFileSync(join(f.repository, "src/lib.rs"), "// changed after source review\n");
  await assert.rejects(prepareReviewedOnboarding(f));
  assert.throws(() => lstatSync(f.output), { code: "ENOENT" });
  assert.deepEqual(snapshot(f.root), before);
});

test("prepared bundles refuse changed proposal, selected source and configuration", async t => {
  const f = await fixture(t), result = await prepareReviewedOnboarding(f), before = snapshot(f.root);
  const options = { ...f, bundle: f.output, approvedPreviewId: result.draft.preview.preview_id };
  const path = join(f.output, "proposal.json"), original = readFileSync(path);
  const proposal = JSON.parse(original); proposal.notes[0].source += "\nChanged after approval.\n";
  writeFileSync(path, JSON.stringify(proposal));
  await assert.rejects(applyReviewedOnboarding(options)); assert.deepEqual(snapshot(f.root), before);
  writeFileSync(path, original);
  const source = join(f.repository, "src/lib.rs"), sourceBytes = readFileSync(source);
  writeFileSync(source, "// source drift\n");
  await assert.rejects(applyReviewedOnboarding(options)); assert.deepEqual(snapshot(f.root), before);
  writeFileSync(source, sourceBytes);
  writeFileSync(join(f.root, "akasha.toml"), readFileSync(join(f.root, "akasha.toml"), "utf8")
    .replace('required_fields = ["entity",', 'required_fields = ["owner", "entity",'));
  const changed = snapshot(f.root);
  await assert.rejects(applyReviewedOnboarding(options)); assert.deepEqual(snapshot(f.root), changed);
});

test("create-only bundles reject root/repository destinations, symlink parents and overwrite", async t => {
  const f = await fixture(t), before = snapshot(f.root), repo = snapshot(f.repository);
  for (const output of [join(f.root, "review"), join(f.repository, "review")]) {
    await assert.rejects(prepareReviewedOnboarding({ ...f, output }), /outside/);
  }
  const alias = join(f.directory, "alias"); symlinkSync(f.repository, alias);
  await assert.rejects(prepareReviewedOnboarding({ ...f, output: join(alias, "review") }), /outside/);
  await prepareReviewedOnboarding(f); const saved = snapshot(f.output);
  await assert.rejects(prepareReviewedOnboarding(f), { code: "EEXIST" });
  assert.deepEqual(snapshot(f.output), saved); assert.deepEqual(snapshot(f.root), before); assert.deepEqual(snapshot(f.repository), repo);
});

test("bounded inputs reject symlinks, oversized JSON and cancellation without publication", async t => {
  const f = await fixture(t), before = snapshot(f.root);
  const alias = join(f.directory, "alias-plan.json"); symlinkSync(f.plan, alias);
  await assert.rejects(prepareReviewedOnboarding({ ...f, plan: alias }), { code: "ELOOP" });
  writeFileSync(alias + ".big", " ".repeat(16_001));
  await assert.rejects(prepareReviewedOnboarding({ ...f, plan: alias + ".big" }), /at most 16000/);
  await assert.rejects(prepareReviewedOnboarding({ ...f, signal: AbortSignal.abort() }), /abort/iu);
  await assert.rejects(applyReviewedOnboarding({ ...f, bundle: f.capture, approvedPreviewId: "sha256:abc" }), /full approved/);
  writeFileSync(join(f.capture, "discovery.json"), " ".repeat(2_097_153));
  await assert.rejects(prepareReviewedOnboarding(f), /at most 2097152/);
  assert.throws(() => lstatSync(f.output), { code: "ENOENT" }); assert.deepEqual(snapshot(f.root), before);
});

test("complete long-path coverage fits the handoff without changing the checked proposal", async t => {
  const f = await fixture(t), before = snapshot(f.root), repo = snapshot(f.repository);
  const path = join(f.capture, "discovery.json"), discovery = JSON.parse(readFileSync(path));
  // Exercise the scanner's maximum descriptive counts independently of the small source packet.
  const directory = "nested/" + "\u0007".repeat(990);
  discovery.coverage.implementation_groups = Array.from({ length: 128 }, (_, i) => ({ directory: `${directory}${i}`, candidates: 1, selected: 0 }));
  discovery.coverage.omitted = Array.from({ length: 128 }, (_, i) => ({ path: `${directory}${i}/a.rs`, reason: "file_limit" }));
  discovery.coverage.excluded_examples = Array.from({ length: 32 }, (_, i) => ({ path: `${directory}${i}/.hidden`, reason: "excluded_name" }));
  const bytes = JSON.stringify(discovery);
  assert(Buffer.byteLength(bytes) > 1_048_576 && Buffer.byteLength(bytes) < 2_097_152);
  writeFileSync(path, bytes);
  const result = await prepareReviewedOnboarding(f);
  assert.equal(result.draft.review.requires_review, true);
  assert.deepEqual(JSON.parse(readFileSync(join(f.output, "discovery.json"))), discovery);
  assert.deepEqual(snapshot(f.root), before); assert.deepEqual(snapshot(f.repository), repo);
});

test("command failures have nonzero exit, stderr diagnostics and empty stdout", () => {
  for (const args of [["apply", "--bundle", "/missing"], ["prepare", "--approved-preview-id", "bad"], ["unexpected"]]) {
    const result = spawnSync(process.execPath, ["scripts/local-onboarding-review.mjs", ...args], { encoding: "utf8" });
    assert.notEqual(result.status, 0); assert.equal(result.stdout, ""); assert(result.stderr.length > 0);
  }
});
