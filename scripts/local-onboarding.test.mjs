import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { setup, snapshot, withServer } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding, planSchema, POLICY } from "./local-onboarding.mjs";

const cli = resolve("target/debug/akasha");
function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "akasha-local-onboarding-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "run 世界 '$`"));
  return JSON.parse(readFileSync(join(f.directory, "run.json"), "utf8"));
}
function run(f, args, project = "first-run") {
  return spawnSync(cli, ["--root", f.root, "--project", project, ...args], { encoding: "utf8" });
}
function options(f) { return { root: f.root, project: "first-run", sources: ["README.md", "Cargo.toml", "src/lib.rs"], json: true }; }
function plan(packet) {
  return { schema_version: 3, notes: [{ note_type: "entity", name: "flight-path",
    fields: { entity: "flight-path", kind: "library", status: "active", reviewed: "2026-11-08" },
    evidence: [{ kind: "fact", claim: "FlightPath stores waypoints in a vector.",
      sources: [{ source: Math.max(0, packet.sources.findIndex(source => source.path === "src/lib.rs")) + 1, excerpt: 1 }] }],
  }] };
}

function slottedReply(input) {
  const original = plan(input).notes[0];
  const implementation = input.planned_note.role === "implementation";
  return { schema_version: 5, notes: [{ slot: input.planned_note.slot,
    evidence: implementation ? original.evidence : [{ kind: "unknown", claim: "Runtime behavior needs further review.",
      rationale: "This test uses a role group without interpreting its source text.", sources: [] }],
  }] };
}

test("plain coverage names missing implementation groups without executing terminal controls", async t => {
  const f = fixture(t);
  const directory = "src/missing\x1b[2J\u202e";
  mkdirSync(join(f.repository, directory));
  writeFileSync(join(f.repository, directory, "other.rs"), "pub const OTHER: usize = 1;\n");
  const result = await draftLocalOnboarding({ root: f.root, project: "first-run", discover: ["src"],
    focus: ["FlightPath"], autoPlan: true, json: false }, async input => ({ answer: JSON.stringify(slottedReply(input)) }));
  assert(result.output.includes("Implementation directories represented: 1/2"));
  assert(result.output.includes("No implementation file selected:"));
  assert(result.output.includes("\\u001b[2J\\u{202e}"));
  assert(!result.output.includes("\x1b") && !result.output.includes("\u202e"));
  const plain = run(f, ["local-onboarding-discover", "--directory", "src", "--focus", "FlightPath"]);
  assert.equal(plain.status, 0, plain.stderr);
  assert(plain.stdout.includes("Implementation group [unrepresented]"));
  assert(!plain.stdout.includes("\x1b") && !plain.stdout.includes("\u202e"));
});

test("focused onboarding selects off-position windows through sources/groups/discovery and reviewed application", async t => {
  const f = fixture(t);
  const lines = Array.from({ length: 400 }, (_, i) => i === 81
    ? "pub const LEASE_SECONDS: usize = 37; // lease deadline\r\n"
    : `// ordinary multilingual 世界 filler ${i + 1}\r\n`);
  writeFileSync(join(f.repository, "src/lib.rs"), lines.join(""));
  writeFileSync(join(f.repository, "src/a-unrelated.rs"), "pub const OTHER: usize = 1;\n");
  const before = snapshot(f.root), repoBefore = snapshot(f.repository);
  let draft, saved;
  for (const [index, selection] of [
    { sources: ["src/lib.rs"] },
    { components: [{ name: "lease-core", note_type: "entity", sources: ["src/lib.rs"] }] },
    { discover: ["src"], autoPlan: true },
  ].entries()) {
    saved = join(f.directory, `focused-${index}`);
    const result = await draftLocalOnboarding({ root: f.root, project: "first-run", ...selection, focus: [" Lease ", "deadline"],
      sampleLargeFiles: true, saveInputs: saved, json: index !== 2 }, async (input, _schema, policy) => {
      assert.deepEqual(input.source_focus, ["deadline", "lease"]);
      assert(policy.includes("literal-focus ranked") && input.focus_rule.includes("unknown"));
      assert(!JSON.stringify(input).includes(f.root));
      assert(!input.sources.some(s => s.path.includes("unrelated")));
      const source = input.sources.find(s => s.path === "src/lib.rs");
      const excerpt = source.excerpts.find(e => e.text.includes("LEASE_SECONDS"));
      assert(excerpt && excerpt.line_start <= 82 && excerpt.line_end >= 82);
      const note = input.planned_note ? slottedReply(input).notes[0] : plan(input).notes[0];
      if (input.component && !input.planned_note) note.name = input.component;
      note.evidence = [{ kind: "fact", claim: "The lease duration constant is 37 seconds.", sources: [{ source: source.handle, excerpt: excerpt.handle }] }];
      return { answer: JSON.stringify({ schema_version: input.planned_note ? 5 : 3, notes: [note] }) };
    });
    const packet = JSON.parse(readFileSync(join(saved, "packet.json")));
    assert.equal(packet.schema_version, 4);
    assert.deepEqual(packet.focus, { terms: ["deadline", "lease"], sample_large_files: true });
    assert.deepEqual(result.metrics.source_focus, packet.focus);
    draft = JSON.parse(readFileSync(join(saved, "draft.json")));
    assert.equal(draft.review.requires_review, true);
    assert(JSON.stringify(draft.review).includes("LEASE_SECONDS"));
    assert(!draft.review.uncited_focus_excerpts);
    if (index === 2) {
      assert(result.output.includes("Source discovery:") && result.output.includes("37 seconds"));
      const plain = run(f, ["local-onboarding-discover", "--directory", "src", "--sample-large-files", "--focus", "lease", "--focus", "deadline"]);
      assert.equal(plain.status, 0, plain.stderr);
      assert(plain.stdout.includes("Literal focus: deadline, lease") && plain.stdout.includes("Matched focus terms:") && plain.stdout.includes("focus_no_match"));
      assert(!plain.stdout.includes("\x1b"));
    }
    assert.deepEqual(snapshot(f.root), before);
    assert.deepEqual(snapshot(f.repository), repoBefore);
  }
  const neglected = await draftLocalOnboarding({ root: f.root, project: "first-run", sources: ["src/lib.rs"],
    focus: ["lease"], sampleLargeFiles: true }, async input => {
    const p = plan(input);
    p.notes[0].evidence = [{ kind: "unknown", claim: "Behavior is not summarized in this review test.", rationale: "Exercise uncited matching window visibility.", sources: [] }];
    return { answer: JSON.stringify(p) };
  });
  assert(neglected.output.includes("Uncited focus excerpt: src/lib.rs:81-96"));
  const applied = run(f, ["--json", "local-onboarding-apply", "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json"), "--approved-preview-id", draft.preview.preview_id]);
  assert.equal(applied.status, 0, applied.stderr);
  assert.equal(run(f, ["validate"]).status, 0);
  const packet = run(f, ["--json", "librarian-prepare", "lease duration seconds"]);
  assert.equal(packet.status, 0, packet.stderr);
  assert(JSON.parse(packet.stdout).evidence.some(s => s.excerpt.includes("37 seconds")));
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("focus failures refuse before inference with no stdout or canonical writes", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const focus of [[], "lease", [null], [""], ["x\ny"], ["x".repeat(81)], Array(9).fill("lease"), ["absent-subsystem"]]) {
    await assert.rejects(draftLocalOnboarding({ ...optionsForDiscovery(f), focus }, () => assert.fail("invalid focus must refuse before inference")));
  }
  const plain = run(f, ["local-onboarding-discover", "--directory", ".", "--focus", "absent-subsystem"]);
  assert.equal(plain.status, 4); assert.equal(plain.stdout, ""); assert(plain.stderr.includes("no focus matches"));
  const saved = join(f.directory, "focus-drift");
  await assert.rejects(draftLocalOnboarding({ ...options(f), focus: ["FlightPath"], saveInputs: saved }, async input => {
    writeFileSync(join(f.repository, "src/lib.rs"), "pub struct Changed;\n");
    return { answer: JSON.stringify(plan(input)) };
  }), error => error.code === 5);
  assert(existsSync(join(saved, "completion-1.json")) && !existsSync(join(saved, "proposal.json")));
  assert.deepEqual(snapshot(f.root), before);
});

test("automatic Rust planning fixes unique slots and remaps source handles through reviewed publication", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const saved = join(f.directory, "planned-review");
  let calls = 0;
  const result = await draftLocalOnboarding({ ...optionsForDiscovery(f), autoPlan: true, saveInputs: saved }, async (input, schema, policy) => {
    calls++;
    assert(input.planned_note && policy.includes("host supplies all metadata"));
    assert.equal(schema.properties.schema_version.const, 5);
    const variant = schema.properties.notes.items.oneOf[0];
    assert.deepEqual(Object.keys(variant.properties).sort(), ["evidence", "slot"]);
    assert.equal(variant.properties.slot.const, input.planned_note.slot);
    assert.equal(schema.properties.notes.items.oneOf.length, 1);
    assert(!Object.hasOwn(input, "component"));
    assert(!Object.hasOwn(input, "note_types") && !Object.hasOwn(input, "templates") && !Object.hasOwn(input, "date"));
    assert(input.planning_rule.includes("defaults, return values and boundaries"));
    assert(!JSON.stringify(input).includes(f.root));
    if (input.planned_note.role === "implementation") assert.deepEqual(input.sources.map(s => s.path), ["src/lib.rs"]);
    return { answer: JSON.stringify(slottedReply(input)) };
  });
  assert.equal(calls, 2);
  const draft = JSON.parse(result.output), identities = JSON.parse(readFileSync(join(saved, "identity-plan.json")));
  assert.equal(new Set(draft.proposal.notes.map(n => n.path)).size, 2);
  assert(draft.proposal.notes.every(n => n.note_type === "entity" && n.path.startsWith("onboarding-")));
  for (const [index, note] of draft.review.notes.entries()) {
    assert.equal(note.fields.entity, identities.slots[index].name);
    assert.equal(note.fields.kind, "source-group");
    assert.equal(note.fields.status, "draft");
    assert.equal(note.fields.reviewed, "unreviewed");
  }
  assert.deepEqual(identities, result.metrics.identity_plan);
  const planFile = JSON.parse(readFileSync(join(saved, "plan.json")));
  assert.equal(planFile.schema_version, 5);
  assert(!planFile.notes.some(note => Object.hasOwn(note, "fields")));
  assert.equal(planFile.notes[1].evidence[0].sources[0].source, 3);
  const plain = run(f, ["local-onboarding-plan", "--packet", join(saved, "packet.json")]);
  assert.equal(plain.status, 0, plain.stderr);
  assert(plain.stdout.includes("semantic review required") && plain.stdout.includes(identities.slots[0].path) && !plain.stdout.includes("\x1b"));
  if (process.platform !== "win32") assert.equal(statSync(join(saved, "identity-plan.json")).mode & 0o777, 0o600);
  assert.deepEqual(snapshot(f.root), before);
  assert.deepEqual(snapshot(f.repository), repoBefore);
  const applied = run(f, ["--json", "local-onboarding-apply", "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json"), "--approved-preview-id", draft.preview.preview_id]);
  assert.equal(applied.status, 0, applied.stderr);
  assert.equal(run(f, ["validate"]).status, 0);
  const librarian = run(f, ["--json", "librarian-prepare", "FlightPath waypoints vector"]);
  assert.equal(librarian.status, 0, librarian.stderr);
  assert(JSON.parse(librarian.stdout).evidence.some(s => s.excerpt.includes("stores waypoints in a vector")));
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("automatic plan refuses slot/name injection, later-group failure and source drift without publication", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const mutation of [
    p => { p.notes[0].slot = 99; },
    p => { p.notes[0].name = "escape"; },
    p => { p.notes[0].note_type = "task"; },
    p => { p.notes[0].fields = { status: "active", reviewed: "2026-11-09" }; },
    p => { p.notes[0].fields = null; },
  ]) await assert.rejects(draftLocalOnboarding({ ...options(f), autoPlan: true }, async input => {
    const p = slottedReply(input); mutation(p); return { answer: JSON.stringify(p) };
  }), /Component must produce/);
  let calls = 0;
  const saved = join(f.directory, "partial-planning");
  await assert.rejects(draftLocalOnboarding({ ...options(f), autoPlan: true, saveInputs: saved }, async input => {
    if (++calls === 2) throw new Error("later group failed");
    return { answer: JSON.stringify(slottedReply(input)) };
  }), /later group failed/);
  assert.equal(calls, 2);
  assert(existsSync(join(saved, "identity-plan.json")) && existsSync(join(saved, "completion-1.json")));
  assert(!existsSync(join(saved, "proposal.json")));
  await assert.rejects(draftLocalOnboarding({ ...options(f), autoPlan: true }, async input => {
    if (input.planned_note.role === "implementation") writeFileSync(join(f.repository, "src/lib.rs"), "pub struct Changed;\n");
    return { answer: JSON.stringify(slottedReply(input)) };
  }), error => error.code === 5);
  assert.deepEqual(snapshot(f.root), before);
});

test("automatic plan rejects incompatible options and supports sampled explicit sources", async t => {
  const f = fixture(t);
  for (const options of [
    { ...optionsForDiscovery(f), autoPlan: "yes" },
    { root: f.root, project: "first-run", components: [{ name: "one", sources: ["README.md"] }], autoPlan: true },
  ]) await assert.rejects(draftLocalOnboarding(options, () => assert.fail("must refuse before inference")), /auto-plan/);
  writeFileSync(join(f.repository, "src/lib.rs"), "// filler for bounded samples\n".repeat(400));
  const result = await draftLocalOnboarding({ root: f.root, project: "first-run", sources: ["src/lib.rs"], autoPlan: true, sampleLargeFiles: true, json: true }, async input => {
    assert(input.sources[0].sampling);
    const p = slottedReply(input);
    p.notes[0].evidence = [{ kind: "fact", claim: "The selected source contains filler comments.", sources: [{ source: 1, excerpt: 3 }] }];
    return { answer: JSON.stringify(p) };
  });
  assert.equal(result.metrics.identity_plan.slots.length, 1);
  assert.equal(JSON.parse(result.output).review.sampled_sources.length, 1);
});

test("automatic metadata refuses custom required fields before inference while explicit corrections remain available", async t => {
  const f = fixture(t);
  const configPath = join(f.root, "akasha.toml");
  const config = readFileSync(configPath, "utf8");
  assert(config.includes('"entity", "kind", "status", "reviewed"'));
  writeFileSync(configPath, config.replace('"entity", "kind", "status", "reviewed"', '"entity", "kind", "status", "reviewed", "owner"'));
  const before = snapshot(f.root);
  await assert.rejects(draftLocalOnboarding({ ...options(f), autoPlan: true }, () => assert.fail("must refuse before inference")), /required fields owner/);
  assert.deepEqual(snapshot(f.root), before);
  const result = await draftLocalOnboarding(options(f), async input => {
    const p = plan(input); p.notes[0].fields.owner = "unassigned";
    return { answer: JSON.stringify(p) };
  });
  assert.equal(JSON.parse(result.output).review.notes[0].fields.owner, "unassigned");
  assert.deepEqual(snapshot(f.root), before);
});

test("large-file sampling supports sources, groups and discovery through reviewed apply and retrieval", async t => {
  const f = fixture(t);
  const text = "// multilingual filler 世界 for bounded coverage\r\n".repeat(399) + "pub const TAIL: usize = 37;";
  writeFileSync(join(f.repository, "src/lib.rs"), text);
  const before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const defaultFailure = run(f, ["--json", "local-onboarding-prepare", "--source", "src/lib.rs"]);
  assert.notEqual(defaultFailure.status, 0); assert.equal(defaultFailure.stdout, "");
  assert(defaultFailure.stderr.includes("8192"));
  const plain = run(f, ["local-onboarding-prepare", "--sample-large-files", "--source", "src/lib.rs"]);
  assert.equal(plain.status, 0, plain.stderr);
  assert(plain.stdout.includes("partial coverage") && plain.stdout.includes("Omitted lines:") && plain.stdout.includes("400 | pub const TAIL"));
  let draft, saved;
  for (const selection of [
    { sources: ["src/lib.rs"] },
    { components: [{ name: "sampled-core", note_type: "entity", sources: ["src/lib.rs"] }] },
    { discover: ["src"] },
  ]) {
    saved = join(f.directory, `sampled-review-${Object.keys(selection)[0]}`);
    const result = await draftLocalOnboarding({ root: f.root, project: "first-run", ...selection, sampleLargeFiles: true, saveInputs: saved, json: true }, async (input, schema, policy) => {
      assert(policy.includes("omitted lines are unknown"));
      const source = input.sources[0];
      assert.equal(source.path, "src/lib.rs");
      assert.equal(source.sampling.total_lines, 400);
      assert.equal(source.sampling.total_bytes, Buffer.byteLength(text));
      assert(source.sampling.omitted_lines.length > 0);
      assert.equal(source.excerpts.length, 3);
      assert.equal(source.excerpts[2].line_end, 400);
      assert(source.excerpts[2].text.endsWith("pub const TAIL: usize = 37;"));
      assert.deepEqual(schema.properties.notes.items.oneOf[0].properties.evidence.items.oneOf[0].properties.sources.items.oneOf[0].properties.excerpt.enum, [1, 2, 3]);
      return { answer: JSON.stringify({ schema_version: 3, notes: [{ note_type: "entity", name: "sampled-core",
        fields: { entity: "sampled-core", kind: "subsystem", status: "active", reviewed: "2026-11-09" },
        evidence: [{ kind: "fact", claim: "The tail constant is 37.", sources: [{ source: 1, excerpt: 3 }] }],
      }] }) };
    });
    draft = JSON.parse(result.output);
    assert.equal(draft.review.sampled_sources[0].total_lines, 400);
    assert.equal(result.metrics.source_sampling[0].total_lines, 400);
    assert.equal(JSON.parse(readFileSync(join(saved, "packet.json"))).schema_version, 3);
    const review = run(f, ["local-onboarding-preview", "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json")]);
    assert.equal(review.status, 0, review.stderr);
    assert(review.stdout.includes("partial coverage") && review.stdout.includes("Selected lines:") && review.stdout.includes("Omitted lines:"));
    assert.deepEqual(snapshot(f.root), before);
    assert.deepEqual(snapshot(f.repository), repoBefore);
  }
  const discovery = run(f, ["local-onboarding-discover", "--directory", "src", "--sample-large-files"]);
  assert.equal(discovery.status, 0, discovery.stderr);
  assert(discovery.stdout.includes("partial coverage") && discovery.stdout.includes("Omitted lines:"));
  const applied = run(f, ["--json", "local-onboarding-apply", "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json"), "--approved-preview-id", draft.preview.preview_id]);
  assert.equal(applied.status, 0, applied.stderr);
  assert.equal(run(f, ["validate"]).status, 0);
  const librarian = run(f, ["--json", "librarian-prepare", "tail constant 37"]);
  assert.equal(librarian.status, 0, librarian.stderr);
  assert(JSON.parse(librarian.stdout).evidence.some(source => source.excerpt.includes("tail constant is 37")));
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("omitted-line edits invalidate sampled drafts and invalid controls fail before inference", async t => {
  const f = fixture(t), before = snapshot(f.root);
  const text = "// stable filler for sampled coverage\n".repeat(399) + "pub const TAIL: usize = 37;\n";
  writeFileSync(join(f.repository, "src/lib.rs"), text);
  const options = { root: f.root, project: "first-run", discover: ["src"], sampleLargeFiles: true, json: true };
  await assert.rejects(draftLocalOnboarding({ ...options, sampleLargeFiles: "yes" }, () => assert.fail("inference must not run")));
  await assert.rejects(draftLocalOnboarding(options, async input => {
    const lines = text.split("\n");
    lines[99] = "// edited filler for sampled coverage";
    writeFileSync(join(f.repository, "src/lib.rs"), lines.join("\n"));
    return { answer: JSON.stringify(plan(input)) };
  }), error => error.code === 5);
  assert.deepEqual(snapshot(f.root), before);
});

test("directory discovery feeds a fresh core packet, saves coverage and publishes searchable facts", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const saved = join(f.directory, "discovered-review");
  const plain = run(f, ["local-onboarding-discover", "--directory", "."]);
  assert.equal(plain.status, 0, plain.stderr);
  assert(plain.stdout.includes("Selected [implementation]: src/lib.rs"));
  assert(plain.stdout.includes("Coverage is limited") && !plain.stdout.includes("\x1b"));
  const result = await draftLocalOnboarding({ root: f.root, project: "first-run", discover: ["."], saveInputs: saved, json: true }, async input => {
    assert(input.selected_source_roles.some(s => s.path === "src/lib.rs" && s.role === "implementation"));
    assert(!JSON.stringify(input).includes(f.repository));
    return { answer: JSON.stringify(plan(input)) };
  });
  const draft = JSON.parse(result.output);
  const discovery = JSON.parse(readFileSync(join(saved, "discovery.json")));
  assert.deepEqual(discovery.packet, JSON.parse(readFileSync(join(saved, "packet.json"))));
  assert.deepEqual(discovery.coverage, result.metrics.source_discovery);
  assert.equal(discovery.coverage.candidates, discovery.coverage.selected.length + discovery.coverage.omitted.length);
  if (process.platform !== "win32") assert.equal(statSync(join(saved, "discovery.json")).mode & 0o777, 0o600);
  assert.deepEqual(snapshot(f.root), before);
  assert.deepEqual(snapshot(f.repository), repoBefore);
  const applied = run(f, ["--json", "local-onboarding-apply", "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json"), "--approved-preview-id", draft.preview.preview_id]);
  assert.equal(applied.status, 0, applied.stderr);
  const librarian = run(f, ["--json", "librarian-prepare", "FlightPath waypoints vector"]);
  assert.equal(librarian.status, 0, librarian.stderr);
  assert(JSON.parse(librarian.stdout).evidence.some(s => s.excerpt.includes("stores waypoints in a vector")));
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("discovery refuses invalid scope and conflicting modes before inference and checks selected source drift", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const options of [
    { ...optionsForDiscovery(f), sources: ["README.md"] },
    { ...optionsForDiscovery(f), components: [{ name: "one", sources: ["README.md"] }] },
    { ...optionsForDiscovery(f), discover: [".."] },
    { ...optionsForDiscovery(f), discover: [".", "src"] },
  ]) await assert.rejects(draftLocalOnboarding(options, () => assert.fail("must refuse before inference")));
  const failed = run(f, ["--json", "local-onboarding-discover", "--directory", ".."]);
  assert.notEqual(failed.status, 0); assert.equal(failed.stdout, ""); assert(failed.stderr.length);
  await assert.rejects(draftLocalOnboarding(optionsForDiscovery(f), async input => {
    writeFileSync(join(f.repository, "src/lib.rs"), readFileSync(join(f.repository, "src/lib.rs"), "utf8") + "\n// drift\n");
    return { answer: JSON.stringify(plan(input)) };
  }), error => error.code === 5);
  assert.deepEqual(snapshot(f.root), before);
});

function optionsForDiscovery(f) { return { root: f.root, project: "first-run", discover: ["."], json: true }; }

test("local draft uses exact core input and preview; disposable MCP apply makes knowledge searchable", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  let calls = 0;
  const result = await draftLocalOnboarding(options(f), async (input, schema, policy) => {
    calls++;
    assert.equal(policy, POLICY);
    assert.equal(input.project, "first-run");
    assert.equal(input.sources.length, 3);
    assert(input.sources.every(source => source.excerpts.length && !Object.hasOwn(source, "text")));
    assert.equal(schema.properties.schema_version.const, 3);
    assert(schema.properties.notes.items.oneOf[0].properties.evidence.items.oneOf[0].properties.sources.items.oneOf[0].properties.excerpt);
    assert(schema.properties.notes.items.oneOf.some(variant => variant.properties.note_type.const === "entity"));
    return { answer: JSON.stringify(plan(input)), metrics: { synthetic: true } };
  });
  assert.equal(calls, 1);
  const draft = JSON.parse(result.output);
  assert.equal(draft.preview.notes[0].evidence_claims, 1);
  assert.equal(draft.schema_version, 2);
  assert.equal(draft.review.requires_review, true);
  assert(draft.review.notes[0].claims[0].sources[0].excerpt.includes("pub struct FlightPath"));
  assert.deepEqual(draft.review.uncited_sources, ["Cargo.toml", "README.md"]);
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
  assert(result.output.includes("Source review (exact excerpts") && result.output.includes("pub struct FlightPath"));
  assert(result.output.includes("Selected but uncited: Cargo.toml, README.md"));
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
    const zeta = { ...value.notes[0], name: "zeta", fields: { ...value.notes[0].fields, entity: "zeta" } };
    const alpha = { ...zeta, name: "alpha", fields: { ...zeta.fields, entity: "alpha" } };
    return { answer: JSON.stringify({ schema_version: 3, notes: [zeta, alpha] }) };
  });
  const positions = ["alpha.md (", "# alpha", "zeta.md (", "# zeta"].map(text => result.output.indexOf(text));
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

test("component drafts remap local handles, preview together and retrieve each applied note", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const components = [{ name: "library", sources: ["src/lib.rs"] }, { name: "manifest", sources: ["Cargo.toml"] }];
  let calls = 0;
  const result = await draftLocalOnboarding({ root: f.root, project: "first-run", components, json: true }, async (input, schema) => {
    assert.equal(input.sources.length, 1);
    assert.equal(input.sources[0].handle, 1);
    assert.equal(schema.properties.notes.maxItems, 1);
    assert.equal(schema.properties.notes.items.oneOf[0].properties.name.const, input.component);
    calls++;
    const note = plan(input).notes[0];
    note.name = input.component; note.fields.entity = input.component;
    note.evidence[0].sources = [{ source: 1, excerpt: 1 }];
    note.evidence[0].claim = input.component === "library" ? "FlightPath stores waypoints." : "The manifest declares a Rust package.";
    return { answer: JSON.stringify({ schema_version: 3, notes: [note] }), metrics: { fake: true } };
  });
  assert.equal(calls, 2);
  const draft = JSON.parse(result.output);
  assert.equal(draft.proposal.notes.length, 2);
  assert(draft.proposal.notes[0].source.includes('"path": "src/lib.rs"'));
  assert(draft.proposal.notes[1].source.includes('"path": "Cargo.toml"'));
  assert.equal(result.metrics.components.length, 2);
  assert.deepEqual(result.metrics.components.map(step => step.uncited_sources), [[], []]);
  assert.deepEqual(snapshot(f.root), before);
  await withServer(f, async ({ tool }) => {
    const validated = await tool("akasha_onboarding_validate", { proposal: draft.proposal });
    const preview = await tool("akasha_onboarding_preview", { proposal_id: validated.proposal_id });
    await tool("akasha_onboarding_apply", { proposal_id: validated.proposal_id, approved_preview_id: preview.preview_id });
  });
  for (const term of ["waypoints", "manifest"]) {
    const found = run(f, ["--json", "librarian-prepare", term]);
    assert.equal(found.status, 0, found.stderr);
    assert(JSON.parse(found.stdout).evidence.some(source => source.excerpt.toLowerCase().includes(term)));
  }
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("component bounds and duplicate names refuse before inference", async t => {
  const f = fixture(t);
  const component = { name: "one", sources: ["src/lib.rs"] };
  for (const components of [[], [component, component], Array(4).fill(component), [{ ...component, sources: ["src/lib.rs", "src/lib.rs"] }], [{ ...component, extra: true }]]) {
    await assert.rejects(draftLocalOnboarding({ root: f.root, project: "first-run", components }, () => assert.fail("must not infer")), /component/i);
  }
});

test("later component failure, out-of-group handles and drift never publish a partial batch", async t => {
  const f = fixture(t), before = snapshot(f.root);
  const components = [{ name: "library", sources: ["src/lib.rs"] }, { name: "manifest", sources: ["Cargo.toml"] }];
  for (const mode of ["failure", "handle", "name", "drift", "cancel"]) {
    const controller = new AbortController(); let calls = 0;
    await assert.rejects(draftLocalOnboarding({ root: f.root, project: "first-run", components, signal: controller.signal }, async input => {
      calls++;
      const note = plan(input).notes[0]; note.name = input.component;
      note.evidence[0].sources = [{ source: 1, excerpt: 1 }];
      if (calls === 2) {
        if (mode === "failure") throw new Error("later failure");
        if (mode === "handle") note.evidence[0].sources[0].source = 2;
        if (mode === "name") note.name = "wrong";
        if (mode === "drift") writeFileSync(join(f.repository, "Cargo.toml"), readFileSync(join(f.repository, "Cargo.toml"), "utf8") + "\n");
        if (mode === "cancel") controller.abort(new Error("cancelled"));
      }
      return { answer: JSON.stringify({ schema_version: 3, notes: [note] }) };
    }));
    assert.equal(calls, 2);
    assert.deepEqual(snapshot(f.root), before);
  }
});


test("configured reviewed fields are constrained to the supplied draft date", () => {
  const schema = planSchema({ sources: [{path:"one", excerpts: [{}]}], preparation: { project:"test", note_types: { entity: { required_fields: ["reviewed", "entity"] } } } }, "2026-11-08");
  assert.deepEqual(schema.properties.notes.items.oneOf[0].properties.fields.properties.reviewed, { const: "2026-11-08" });
});

test("excerpt prompt formatting preserves complete Unicode/CRLF bytes across windows", async t => {
  const f = fixture(t);
  const text = Array.from({ length: 34 }, (_, i) => `// Привет 世界 ${i}\r\n`).join("") + "// final λ";
  writeFileSync(join(f.repository, "unicode.rs"), text);
  const result = await draftLocalOnboarding({ ...options(f), sources: ["unicode.rs"] }, async input => {
    assert.equal(input.sources[0].excerpts.length, 3);
    assert.equal(input.sources[0].excerpts.map(e => e.text).join(""), text);
    const value = plan(input);
    value.notes[0].evidence[0].claim = "The source includes multilingual comments.";
    value.notes[0].evidence[0].sources = [{ source: 1, excerpt: 3 }];
    return { answer: JSON.stringify(value) };
  });
  const reviewed = JSON.parse(result.output).review.notes[0].claims[0].sources[0];
  assert.equal(reviewed.line_start, 33);
  assert.equal(reviewed.line_end, 35);
  assert(reviewed.excerpt.endsWith("// final λ"));
});

test("explicit configured component class constrains schema and refuses model class drift", async t => {
  const f = fixture(t), before = snapshot(f.root);
  const components = [{ name: "library", sources: ["src/lib.rs"], note_type: "entity" }];
  const opts = { root: f.root, project: "first-run", components, json: true };
  for (const wrongType of [false, true]) {
    const draft = draftLocalOnboarding(opts, async (input, schema) => {
      assert.deepEqual(schema.properties.notes.items.oneOf.map(v => v.properties.note_type.const), ["entity"]);
      const value = plan(input);
      value.notes[0].name = input.component;
      if (wrongType) value.notes[0].note_type = "decision";
      return { answer: JSON.stringify(value) };
    });
    if (wrongType) await assert.rejects(draft, /named note/);
    else assert.equal(JSON.parse((await draft).output).proposal.notes[0].note_type, "entity");
  }
  await assert.rejects(draftLocalOnboarding({ ...opts, components: [{ ...components[0], note_type: "absent" }] }, () => assert.fail("must not infer")), /configured component note_type/);
  assert.deepEqual(snapshot(f.root), before);
});

test("empty or unselectable complete-line catalogs refuse before inference", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const text of ["", "x".repeat(2049)]) {
    writeFileSync(join(f.repository, "uncitable"), text);
    await assert.rejects(draftLocalOnboarding({ ...options(f), sources: ["uncitable"] }, () => assert.fail("must not infer")), /no complete excerpt catalog/);
  }
  assert.deepEqual(snapshot(f.root), before);
});

test("out-of-group excerpt handles refuse without returning a partial component batch", async t => {
  const f = fixture(t), before = snapshot(f.root);
  await assert.rejects(draftLocalOnboarding({ root: f.root, project: "first-run", components: [{ name: "manifest", sources: ["Cargo.toml"] }] }, async input => {
    const value = plan(input); value.notes[0].name = input.component;
    value.notes[0].evidence[0].sources[0].excerpt = 99;
    return { answer: JSON.stringify(value) };
  }), /Unknown component excerpt handle/);
  assert.deepEqual(snapshot(f.root), before);
});


test("quote failures and legacy body plans fail through the actual adapter without writes", async t => {
  const f = fixture(t), before = snapshot(f.root);
  for (const mode of ["quote", "body", "version"]) {
    await assert.rejects(draftLocalOnboarding(options(f), async input => {
      const value = plan(input);
      if (mode === "quote") value.notes[0].evidence[0].sources[0].quote = "invented source code";
      if (mode === "body") value.notes[0].body = "Unsupported complexity prose";
      if (mode === "version") value.schema_version = 1;
      return { answer: JSON.stringify(value) };
    }), error => error.code === 4);
    assert.deepEqual(snapshot(f.root), before);
  }
});


test("saved inputs preserve rejected plans privately and refuse overwrite or memory/repository destinations", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const saved = join(f.directory, "review-inputs");
  await assert.rejects(draftLocalOnboarding({ ...options(f), saveInputs: saved }, async input => {
    const value = plan(input); value.notes[0].evidence[0].sources[0].quote = "invented quote";
    return { answer: JSON.stringify(value) };
  }), error => error.code === 4);
  assert.equal(JSON.parse(readFileSync(join(saved, "plan.json"))).schema_version, 3);
  assert.equal(readFileSync(join(saved, "completion-1.json"), "utf8"), readFileSync(join(saved, "plan.json"), "utf8"));
  assert.equal(JSON.parse(readFileSync(join(saved, "packet.json"))).sources.length, 3);
  if (process.platform !== "win32") {
    assert.equal(statSync(saved).mode & 0o777, 0o700);
    assert.equal(statSync(join(saved, "plan.json")).mode & 0o777, 0o600);
  }
  for (const saveInputs of [saved, join(f.root, "private-inputs"), join(f.repository, "private-inputs")]) {
    await assert.rejects(draftLocalOnboarding({ ...options(f), saveInputs }, () => assert.fail("must refuse before inference")));
  }
  if (process.platform !== "win32") {
    symlinkSync(f.root, join(f.directory, "memory-alias"));
    await assert.rejects(draftLocalOnboarding({ ...options(f), saveInputs: join(f.directory, "memory-alias", "private-inputs") }, () => assert.fail("must refuse symlink parent")), /outside/);
  }
  const successful = join(f.directory, "successful-inputs");
  const result = await draftLocalOnboarding({ ...options(f), saveInputs: successful }, async input => ({ answer: JSON.stringify(plan(input)) }));
  assert(JSON.parse(result.output).review.requires_review);
  assert(existsSync(join(successful, "plan.json")));
  assert.deepEqual(JSON.parse(readFileSync(join(successful, "proposal.json"))), JSON.parse(result.output).proposal);
  assert.equal(readFileSync(join(successful, "draft.json"), "utf8"), result.output);
  if (process.platform !== "win32") {
    for (const name of ["packet.json", "completion-1.json", "plan.json", "draft.json", "proposal.json"]) {
      assert.equal(statSync(join(successful, name)).mode & 0o777, 0o600);
    }
  }
  assert.deepEqual(snapshot(f.root), before);
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("saved plain drafts support source review, corrected approval, CLI publication and fresh no-op review", async t => {
  const f = fixture(t), before = snapshot(f.root), repoBefore = snapshot(f.repository);
  const saved = join(f.directory, "local-review");
  const result = await draftLocalOnboarding({ ...options(f), json: false, saveInputs: saved }, async input => ({ answer: JSON.stringify(plan(input)) }));
  const original = JSON.parse(readFileSync(join(saved, "draft.json")));
  assert(result.output.includes(original.preview.preview_id));
  assert(result.output.includes("pub struct FlightPath"));
  assert.deepEqual(snapshot(f.root), before); // Stopping here is cancellation: no publication.
  const packet = join(saved, "packet.json"), proposal = join(saved, "proposal.json");
  const apply = ["local-onboarding-apply", "--packet", packet, "--proposal", proposal];
  const missing = run(f, apply);
  assert.equal(missing.status, 2); assert.equal(missing.stdout, "");
  const wrong = run(f, [...apply, "--approved-preview-id", "yes"]);
  assert.equal(wrong.status, 5); assert.equal(wrong.stdout, "");
  assert.deepEqual(snapshot(f.root), before);
  // Review an explicit correction against the same sources, without a second model run.
  const corrected = JSON.parse(readFileSync(join(saved, "plan.json")));
  corrected.notes[0].evidence[0].claim = "FlightPath stores waypoints in a Vec<[f64; 3]>.";
  const correctedPlan = join(saved, "reviewed-plan.json");
  writeFileSync(correctedPlan, JSON.stringify(corrected), { mode: 0o600, flag: "wx" });
  const compiled = run(f, ["--json", "local-onboarding-compile", "--packet", packet, "--plan", correctedPlan]);
  assert.equal(compiled.status, 0, compiled.stderr);
  const reviewed = JSON.parse(compiled.stdout), reviewedProposal = join(saved, "reviewed-proposal.json");
  writeFileSync(reviewedProposal, JSON.stringify(reviewed.proposal), { mode: 0o600, flag: "wx" });
  const correctedApply = ["local-onboarding-apply", "--packet", packet, "--proposal", reviewedProposal, "--approved-preview-id"];
  const obsolete = run(f, [...correctedApply, original.preview.preview_id]);
  assert.equal(obsolete.status, 5); assert.equal(obsolete.stdout, "");
  assert.deepEqual(snapshot(f.root), before);
  const applied = run(f, [...correctedApply, reviewed.preview.preview_id]);
  assert.equal(applied.status, 0, applied.stderr); assert.equal(applied.stderr, "");
  assert(applied.stdout.includes("Local onboarding applied") && applied.stdout.includes("Created:"));
  assert(!applied.stdout.includes("\x1b"));
  assert.equal(run(f, ["validate"]).status, 0);
  const found = run(f, ["--json", "librarian-prepare", "FlightPath waypoints"]);
  assert.equal(found.status, 0, found.stderr);
  assert(JSON.parse(found.stdout).evidence.some(e => e.excerpt.includes("Vec<[f64; 3]>")));
  const after = snapshot(f.root);
  const replay = run(f, [...correctedApply, reviewed.preview.preview_id]);
  assert.equal(replay.status, 5); assert.equal(replay.stdout, "");
  assert.deepEqual(snapshot(f.root), after);
  // Fresh preparation/review of the exact saved proposal is an explicit no-op rerun.
  const freshPacket = join(saved, "rerun-packet.json");
  const prepare = run(f, ["--json", "local-onboarding-prepare", ...options(f).sources.flatMap(s => ["--source", s])]);
  assert.equal(prepare.status, 0, prepare.stderr);
  writeFileSync(freshPacket, prepare.stdout, { mode: 0o600, flag: "wx" });
  const preview = run(f, ["--json", "local-onboarding-preview", "--packet", freshPacket, "--proposal", reviewedProposal]);
  assert.equal(preview.status, 0, preview.stderr);
  const rerun = run(f, ["--json", "local-onboarding-apply", "--packet", freshPacket, "--proposal", reviewedProposal,
    "--approved-preview-id", JSON.parse(preview.stdout).preview.preview_id]);
  assert.equal(rerun.status, 0, rerun.stderr); assert.equal(rerun.stderr, "");
  const outcome = JSON.parse(rerun.stdout);
  assert.equal(outcome.created_notes.length, 0); assert.equal(outcome.unchanged_notes.length, 1);
  assert.equal(outcome.updated_projections.length, 0);
  assert.deepEqual(snapshot(f.root), after);
  assert.deepEqual(snapshot(f.repository), repoBefore);
});

test("CLI local apply preserves concurrent source, projection and canonical changes", async t => {
  for (const mode of ["source", "projection", "note", "project"]) {
    const f = fixture(t), saved = join(f.directory, `review-${mode}`);
    const result = await draftLocalOnboarding({ ...options(f), saveInputs: saved }, async input => ({ answer: JSON.stringify(plan(input)) }));
    const draft = JSON.parse(result.output);
    if (mode === "source") writeFileSync(join(f.repository, "src/lib.rs"), "human code change\n");
    if (mode === "projection") writeFileSync(join(f.root, "Projects/first-run/index.md"), "# Human index change\n");
    if (mode === "note") writeFileSync(join(f.root, "Projects/first-run/entities/flight-path.md"), "human note bytes\n");
    if (mode === "project") {
      const repository = join(f.directory, "other-repo");
      mkdirSync(repository);
      const init = spawnSync(cli, ["--root", f.root, "init", "other"], { cwd: repository, encoding: "utf8" });
      assert.equal(init.status, 0, init.stderr);
    }
    const before = snapshot(f.root), repoBefore = snapshot(f.repository);
    const rejected = run(f, ["--json", "local-onboarding-apply",
      "--packet", join(saved, "packet.json"), "--proposal", join(saved, "proposal.json"), "--approved-preview-id", draft.preview.preview_id], mode === "project" ? "other" : "first-run");
    assert.notEqual(rejected.status, 0, mode); assert.equal(rejected.stdout, "", mode);
    assert(rejected.stderr.length > 0);
    assert.deepEqual(snapshot(f.root), before, mode);
    assert.deepEqual(snapshot(f.repository), repoBefore, mode);
  }
});
