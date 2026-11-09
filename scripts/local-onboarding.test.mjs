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
