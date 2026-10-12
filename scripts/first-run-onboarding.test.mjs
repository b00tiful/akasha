import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { launchArguments, probe, setup, snapshot, verify, withServer } from "./first-run-onboarding.mjs";

function fixture(t) {
  const parent = mkdtempSync(join(tmpdir(), "akasha-first-run-test-"));
  t.after(() => rmSync(parent, { recursive: true, force: true }));
  const created = setup(join(parent, "run 世界 '$`"));
  return JSON.parse(readFileSync(join(created.directory, "run.json"), "utf8"));
}

function proposal(f) {
  const fingerprint = `sha256:${createHash("sha256").update(readFileSync(join(f.repository, "src/lib.rs"))).digest("hex")}`;
  return {
    notes: [{ note_type: "entity", path: "core.md", source: `---
schema_version: 1
entity: core
kind: subsystem
status: active
reviewed: 2026-10-12
evidence:
  - kind: fact
    claim: FlightPath stores waypoints in a vector.
    sources:
      - path: src/lib.rs
        fingerprint: "${fingerprint}"
  - kind: inference
    claim: The API is intended for three-dimensional paths.
    rationale: Waypoint exposes x, y and z coordinates, and its documentation describes three dimensions.
    sources:
      - path: src/lib.rs
        fingerprint: "${fingerprint}"
  - kind: unknown
    claim: Production deployment is unknown.
    rationale: The supplied repository contains no deployment instructions.
---

# Core

Observed: FlightPath stores waypoints in a vector.
Inference: The API is intended for three-dimensional paths.
Unknown: Production deployment.

[[Projects/first-run/index|Index]] · [[Projects/first-run/roadmap|Roadmap]]
` }],
    index: "# Index\n\n[[Projects/first-run/entities/core|Core]]\n\n[[Projects/first-run/roadmap|Roadmap]]\n",
    roadmap: "# Roadmap\n\nNo inferred tasks.\n\n[[Projects/first-run/index|Index]]\n",
  };
}

test("real empty-start stdio review and cancellation preserve exact snapshots", async (t) => {
  const f = fixture(t);
  assert.equal(f.handoff.root, f.root);
  assert.equal(f.handoff.repository, f.repository);
  const before = snapshot(f.root);
  await withServer(f, async ({ call, tool }) => {
    const discovered = (await call("tools/list", {})).tools;
    assert.equal(discovered.length, 4);
    const prepared = (await tool("akasha_onboarding_prepare")).preparation;
    assert.equal(prepared.templates.length, 7);
    const validated = await tool("akasha_onboarding_validate", { proposal: proposal(f) });
    const preview = await tool("akasha_onboarding_preview", { proposal_id: validated.proposal_id });
    assert.equal(preview.notes[0].evidence_claims, 3);
    assert.equal(preview.notes[0].action, "create");
    assert(preview.approval_summary.includes("create 1 notes"));
    // Stop without apply. This is deterministic protocol evidence, not a host denial.
  });
  assert.deepEqual(snapshot(f.root), before);
  assert.equal((await verify(f.directory, "cancel")).host_approval, "not_observed");
});

test("deterministic apply can be verified read-only with persisted evidence", async (t) => {
  const f = fixture(t);
  await withServer(f, async ({ tool }) => {
    const validated = await tool("akasha_onboarding_validate", { proposal: proposal(f) });
    const preview = await tool("akasha_onboarding_preview", { proposal_id: validated.proposal_id });
    await tool("akasha_onboarding_apply", { proposal_id: validated.proposal_id, approved_preview_id: preview.preview_id });
  });
  const before = snapshot(f.root);
  const result = await verify(f.directory, "apply");
  assert.equal(result.filesystem_verification, "passed");
  assert.equal(result.host_approval, "not_observed");
  assert.equal(result.human_claim_review, "required");
  assert.deepEqual(snapshot(f.root), before);
  await assert.rejects(verify(f.directory, "cancel"), /Cancellation changed/);
  const note = join(f.root, "Projects/first-run/entities/core.md");
  writeFileSync(note, readFileSync(note, "utf8").replace(/sha256:[a-f0-9]{64}/g, `sha256:${"0".repeat(64)}`));
  await assert.rejects(verify(f.directory, "apply"), /canonical sources changed/);
});

test("setup refuses an existing run without deleting or changing it", (t) => {
  const f = fixture(t);
  const before = snapshot(f.directory);
  assert.throws(() => setup(f.directory), /EEXIST/);
  assert.deepEqual(snapshot(f.directory), before);
});

test("verification refuses source, entry type and permission drift", async (t) => {
  const f = fixture(t);
  const source = join(f.repository, "README.md");
  const original = readFileSync(source);
  writeFileSync(source, "changed source\n");
  await assert.rejects(verify(f.directory, "cancel"), /Source repository changed/);
  writeFileSync(source, original);
  const config = join(f.root, "akasha.toml");
  chmodSync(config, 0o640);
  await assert.rejects(verify(f.directory, "cancel"), /Cancellation changed/);
  chmodSync(config, 0o600);
  symlinkSync(source, join(f.root, "foreign-link"));
  await assert.rejects(verify(f.directory, "cancel"), /Unexpected symlink/);
});

test("changed client policy or manifest identity refuses before a live launch", async (t) => {
  const f = fixture(t);
  const config = join(f.clientHome, "config.toml");
  const original = readFileSync(config);
  writeFileSync(config, 'approval_policy = "never"\n');
  await assert.rejects(verify(f.directory, "cancel"), /Client policy changed/);
  writeFileSync(config, original);
  f.repository = "/";
  writeFileSync(join(f.directory, "run.json"), JSON.stringify(f));
  await assert.rejects(verify(f.directory, "cancel"), /Changed repository/);
});

test("live command isolates the daemon and cannot run without a human terminal", (t) => {
  const f = fixture(t);
  const args = launchArguments(f);
  assert(args.includes("--no-daemon"));
  assert(args.includes("--strict-config"));
  assert(args.includes("read-only"));
  assert(args.at(-1).includes("If denied, stop without retrying"));
  const result = spawnSync(process.execPath, ["scripts/first-run-onboarding.mjs", "run", f.directory], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /interactive terminal/);
});

test("preflight refuses inherited project configuration before launching Codex", async (t) => {
  const f = fixture(t);
  mkdirSync(join(f.directory, ".codex"));
  writeFileSync(join(f.directory, ".codex/config.toml"), "# unwanted parent configuration\n");
  const before = snapshot(f.root);
  await assert.rejects(probe(f.directory), /Inherited project context/);
  assert.deepEqual(snapshot(f.root), before);
});

test("observed one-time TUI marker preserves review without admitting policy drift", async (t) => {
  const f = fixture(t);
  const path = join(f.clientHome, "config.toml");
  const original = readFileSync(path, "utf8");
  const marker = '\n[tui]\nscreen_reader_detection_done = true\n';
  writeFileSync(path, original + marker);
  const before = snapshot(f.root);
  assert.equal((await verify(f.directory, "cancel")).filesystem_verification, "passed");
  assert.equal(readFileSync(path, "utf8"), original + marker);
  assert.deepEqual(snapshot(f.root), before);
  for (const changed of [
    original.replace('approvals_reviewer = "user"', 'approvals_reviewer = "auto_review"') + marker,
    original.replace('approval_mode = "prompt"', 'approval_mode = "auto"') + marker,
    original + marker + '\n[mcp_servers.foreign]\ncommand = "foreign-server"\n',
    original + marker.replace('true', 'false'),
  ]) {
    writeFileSync(path, changed);
    await assert.rejects(verify(f.directory, "cancel"), /Client policy changed/);
    assert.deepEqual(snapshot(f.root), before);
  }
});
