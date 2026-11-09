import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { lstatSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import vm from "node:vm";
import test from "node:test";
import { setup, snapshot } from "./first-run-onboarding.mjs";
import { draftLocalOnboarding } from "./local-onboarding.mjs";
import { renderOnboardingReview } from "./onboarding-review.mjs";

// Minimal DOM for testing the exact exported page program, including browser download bytes.
function openPage(html) {
  const all = [], ids = new Map(), blobs = [];
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.listeners = {}; this.attributes = {}; this.value = ""; all.push(this); }
    append(node) { this.children.push(node); }
    setAttribute(key, value) { this.attributes[key] = value; }
    addEventListener(key, callback) { this.listeners[key] = callback; }
    replaceChildren() { this.children = []; }
    remove() {}
    click() { this.listeners.click?.(); }
  }
  const document = { body: new Element("body"), createElement: tag => new Element(tag), getElementById(id) {
    if (!ids.has(id)) ids.set(id, new Element("div")); return ids.get(id);
  } };
  document.getElementById("review-data").textContent = html.match(/<script id="review-data" type="application\/json">(.*?)<\/script>/su)[1];
  const script = html.match(/<script>([\s\S]*?)<\/script>/u)[1];
  const token = createHash("sha256").update(script).digest("base64");
  assert(html.includes(`script-src 'sha256-${token}'`));
  vm.runInNewContext(script, { document, TextEncoder, TextDecoder, structuredClone, Blob,
    URL: { createObjectURL(blob) { blobs.push(blob); return "blob:local-test"; }, revokeObjectURL() {} },
    window: { addEventListener() {} }, setTimeout: fn => fn() });
  return { all, document, blobs };
}

test("offline correction export recompiles and applies through the checked Rust transaction", async t => {
  const directory = mkdtempSync(join(tmpdir(), "akasha-offline-review-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = setup(join(directory, "run"));
  const cli = resolve("target/debug/akasha"), saved = join(directory, "review");
  const before = snapshot(f.root), repository = snapshot(f.repository);
  const draft = await draftLocalOnboarding({ root: f.root, project: "first-run", sources: ["src/lib.rs"], autoPlan: true, saveInputs: saved, json: true }, async input => ({
    answer: JSON.stringify({ schema_version: 5, notes: [{ slot: input.planned_note.slot,
      evidence: [{ kind: "fact", claim: "FlightPath stores waypoints in a list.", sources: [{ source: 1, excerpt: 1 }] }] }] }),
  }));
  assert.deepEqual(snapshot(f.root), before); assert.deepEqual(snapshot(f.repository), repository);
  assert.equal(lstatSync(join(saved, "review.html")).mode & 0o777, 0o600);
  const page = openPage(readFileSync(join(saved, "review.html"), "utf8"));
  assert(page.all.some(node => node.tagName === "pre" && node.textContent.includes("waypoints: Vec")));
  const textarea = page.all.find(node => node.attributes["aria-label"] === "Claim text for note 1 claim 1");
  textarea.value = "FlightPath stores waypoints in a vector."; textarea.listeners.input();
  page.document.getElementById("export").click();
  const exported = JSON.parse(await page.blobs[0].text());
  assert.equal(exported.notes[0].evidence[0].claim, textarea.value);
  assert.deepEqual(exported.notes[0].evidence[0].sources, [{ source: 1, excerpt: 1 }]);
  assert.equal(exported.schema_version, 5); assert(!("fields" in exported.notes[0]));
  const plan = join(saved, "reviewed-plan.json"), proposal = join(saved, "reviewed-proposal.json");
  writeFileSync(plan, JSON.stringify(exported));
  const run = args => spawnSync(cli, ["--root", f.root, "--project", "first-run", ...args], { encoding: "utf8" });
  const compiled = run(["--json", "local-onboarding-compile", "--packet", join(saved, "packet.json"), "--plan", plan]);
  assert.equal(compiled.status, 0, compiled.stderr); const corrected = JSON.parse(compiled.stdout);
  writeFileSync(proposal, JSON.stringify(corrected.proposal));
  assert.notEqual(corrected.preview.preview_id, JSON.parse(draft.output).preview.preview_id);
  const args = ["local-onboarding-apply", "--packet", join(saved, "packet.json"), "--proposal", proposal, "--approved-preview-id"];
  const stale = run([...args, JSON.parse(draft.output).preview.preview_id]);
  assert.notEqual(stale.status, 0); assert.deepEqual(snapshot(f.root), before);
  const applied = run([...args, corrected.preview.preview_id]); assert.equal(applied.status, 0, applied.stderr);
  assert.deepEqual(snapshot(f.repository), repository);
  const packet = JSON.parse(readFileSync(join(saved, "packet.json")));
  const note = corrected.proposal.notes[0];
  const body = readFileSync(join(packet.preparation.project_dir, packet.preparation.note_types[note.note_type].folder, note.path), "utf8");
  assert(body.includes("vector")); assert(!body.includes("in a list"));
  const prepared = run(["--json", "librarian-prepare", "How are FlightPath waypoints stored?"]);
  assert.equal(prepared.status, 0, prepared.stderr); assert(JSON.parse(prepared.stdout).evidence.some(e => e.excerpt.includes("in a vector")));
});

test("untrusted source/claim text cannot break out of the page or change executable code", async () => {
  const hostile = '</script><script>globalThis.PWNED=true</script><img src="https://example.invalid/leak">';
  const claim = { kind: "fact", claim: hostile, sources: [{ source: 1, excerpt: 1 }] };
  const packet = { preparation: { project: hostile }, sources: [{ path: hostile, fingerprint: "test", text: hostile,
    excerpts: [{ byte_start: 0, byte_end: Buffer.byteLength(hostile), line_start: 1, line_end: 1 }] }] };
  const plan = { schema_version: 3, notes: [{ name: hostile, evidence: [claim] }] };
  const draft = { review: { requires_review: true, notes: [{ path: hostile, fields: { reviewed: "unreviewed" } }], uncited_sources: [] } };
  const html = renderOnboardingReview({ packet, plan, draft });
  assert(!html.includes(hostile)); assert(html.includes("connect-src 'none'"));
  assert.equal((html.match(/<script>/gu) ?? []).length, 1);
  const page = openPage(html);
  assert.equal(page.document.getElementById("project").textContent, hostile);
  assert(page.all.some(node => node.tagName === "pre" && node.textContent.includes(hostile)));
  page.document.getElementById("export").click();
  assert.equal(JSON.parse(await page.blobs[0].text()).notes[0].evidence[0].claim, hostile);
  const kind = page.all.find(node => node.tagName === "select"); kind.value = "unknown"; kind.listeners.change();
  const rationale = page.all.find(node => node.attributes["aria-label"] === "Rationale for note 1 claim 1");
  rationale.value = "No trustworthy supporting evidence."; rationale.listeners.input();
  page.document.getElementById("export").click();
  const unknown = JSON.parse(await page.blobs[1].text()).notes[0].evidence[0];
  assert.equal(unknown.kind, "unknown"); assert.deepEqual(unknown.sources, []); assert.equal(unknown.rationale, rationale.value);
});
