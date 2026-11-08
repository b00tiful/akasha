import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { setup, snapshot, withServer } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding, POLICY } from "./local-onboarding.mjs";

const cli = resolve("target/debug/akasha");
function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "akasha-local-onboarding-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "run 世界 '$`"));
  return JSON.parse(readFileSync(join(f.directory, "run.json"), "utf8"));
}
function run(f, args) {
  return spawnSync(cli, ["--root", f.root, "--project", "first-run", ...args], { encoding: "utf8" });
}
function options(f) { return { root: f.root, project: "first-run", sources: ["README.md", "Cargo.toml", "src/lib.rs"], json: true }; }
function plan(packet) {
  return { notes: [{ note_type: "entity", name: "flight-path",
    fields: { entity: "flight-path", kind: "library", status: "active", reviewed: "2026-11-08" },
    body: "# Flight Path\n\nFlightPath stores waypoints in a vector.\n\n[[Projects/first-run/index|Index]] · [[Projects/first-run/roadmap|Roadmap]]\n",
    evidence: [{ kind: "fact", claim: "FlightPath stores waypoints in a vector.",
      sources: [{ source: packet.sources.findIndex(source => source.path === "src/lib.rs") + 1 }] }],
  }] };
}

test("local draft uses exact core input and preview; disposable MCP apply makes knowledge searchable", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  let calls = 0;
  const result = await draftLocalOnboarding(options(f), async (input, schema, policy) => {
    calls++;
    assert.equal(policy, POLICY);
    assert.equal(input.project, "first-run");
    assert.equal(input.sources.length, 3);
    assert(schema.properties.notes.items.oneOf.some(variant => variant.properties.note_type.const === "entity"));
    return { answer: JSON.stringify(plan(input)), metrics: { synthetic: true } };
  });
  assert.equal(calls, 1);
  const draft = JSON.parse(result.output);
  assert.equal(draft.preview.notes[0].evidence_claims, 1);
  assert.deepEqual(snapshot(f.root), before);
  assert.deepEqual(snapshot(f.repository), repoBefore);
  await withServer(f, async ({ tool }) => {
    const validated = await tool("akasha_onboarding_validate", { proposal: draft.proposal });
    const preview = await tool("akasha_onboarding_preview", { proposal_id: validated.proposal_id });
    assert.equal(preview.preview_id, draft.preview.preview_id);
    await tool("akasha_onboarding_apply", { proposal_id: validated.proposal_id, approved_preview_id: preview.preview_id });
  });
  assert.equal(run(f, ["validate"]).status, 0);
  const found = run(f, ["--json", "librarian-prepare", "FlightPath waypoints vector"]);
  assert.equal(found.status, 0, found.stderr);
  assert(JSON.parse(found.stdout).evidence.some(source => source.excerpt.includes("stores waypoints in a vector")));
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("plain and JSON core commands expose complete input and reviewable exact draft", async t => {
  const f = fixture(t);
  const plain = run(f, ["local-onboarding-prepare", "--source", "README.md"]);
  assert.equal(plain.status, 0, plain.stderr); assert.equal(plain.stderr, "");
  assert(plain.stdout.includes("complete files; no writes") && plain.stdout.includes("three-dimensional"));
  assert(!plain.stdout.includes("\x1b"));
  const result = await draftLocalOnboarding({ ...options(f), json: false }, async input => ({ answer: JSON.stringify(plan(input)) }));
  assert(result.output.includes("No writes. Factual support needs review."));
  assert(result.output.includes("stores waypoints in a vector") && result.output.includes("Preview: sha256:"));
  assert(!result.output.includes("\x1b"));
});

test("source drift after inference refuses and preserves memory", async t => {
  const f = fixture(t), before = snapshot(f.root);
  await assert.rejects(draftLocalOnboarding(options(f), async input => {
    writeFileSync(join(f.repository, "src/lib.rs"), "external edit\n");
    return { answer: JSON.stringify(plan(input)) };
  }), error => error.code === 5 && error.stderr.includes("stale or altered"));
  assert.deepEqual(snapshot(f.root), before);
});

test("plain multi-note review binds sorted preview paths to the correct original bodies", async t => {
  const f = fixture(t);
  const result = await draftLocalOnboarding({ ...options(f), json: false }, async input => {
    const value = plan(input);
    const zeta = { ...value.notes[0], name: "zeta", fields: { ...value.notes[0].fields, entity: "zeta" }, body: "# Zeta\n\nFlightPath stores waypoints.\n" };
    const alpha = { ...zeta, name: "alpha", fields: { ...zeta.fields, entity: "alpha" }, body: "# Alpha\n\nFlightPath stores waypoints.\n" };
    return { answer: JSON.stringify({ notes: [zeta, alpha] }) };
  });
  const positions = ["alpha.md (", "# Alpha", "zeta.md (", "# Zeta"].map(text => result.output.indexOf(text));
  assert(positions.every(position => position >= 0));
  assert.deepEqual([...positions].sort((a, b) => a - b), positions);
});

test("invalid model output and out-of-packet evidence refuse with no memory writes", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const make of [() => "not JSON", input => { const value = plan(input); value.notes[0].evidence[0].sources[0].source = 99; return JSON.stringify(value); }]) {
    await assert.rejects(draftLocalOnboarding(options(f), async input => ({ answer: make(input) })), error => error.code === 4);
    assert.deepEqual(snapshot(f.root), before);
  }
});

test("cancellation cannot emit a draft and path refusal happens before inference", async t => {
  const f = fixture(t), before = snapshot(f.root), controller = new AbortController();
  await assert.rejects(draftLocalOnboarding({ ...options(f), signal: controller.signal }, async input => {
    controller.abort(new Error("cancelled")); return { answer: JSON.stringify(plan(input)) };
  }), /cancelled/);
  let called = false;
  await assert.rejects(draftLocalOnboarding({ ...options(f), sources: ["../escape"] }, async () => { called = true; }));
  assert.equal(called, false);
  assert.deepEqual(snapshot(f.root), before);
});

test("actual command failures keep stdout empty and diagnostics on stderr", t => {
  const f = fixture(t);
  const result = spawnSync(process.execPath, [resolve("scripts/local-onboarding.mjs"), "--root", f.root, "--project", "first-run",
    "--source", "../escape", "--server", "/absent", "--model", "/absent", "--model-sha256", "0".repeat(64)], { encoding: "utf8" });
  assert.equal(result.status, 1); assert.equal(result.stdout, "");
  assert(result.stderr.includes("Local onboarding failed:") && result.stderr.includes("repository-relative"));
});
