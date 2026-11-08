import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { scoreSources, verifySources } from "./benchmark-local-librarian.mjs";

for (const file of ["corpus.json", "holdout.json", "multilingual.json"]) test(`${file}: labels refer to real source text and scoped identities`, async () => {
  const corpus = JSON.parse(await readFile(new URL(`../tests/fixtures/librarian-benchmark/${file}`, import.meta.url)));
  const notes = new Map(corpus.notes.map(note => ["Projects/example/" + note.path, note.body]));
  assert.equal(notes.size, corpus.notes.length); assert.equal(new Set(corpus.cases.map(item => item.id)).size, corpus.cases.length);
  for (const item of corpus.cases) {
    for (const want of item.required) { assert(notes.get(want.id)?.includes(want.text), item.id); assert(item.allowed.includes(want.id)); }
    for (const id of item.allowed) assert(notes.has(id), item.id);
    if (item.expected_link) for (const id of item.expected_link) assert(notes.has(id));
  }
});

test("rubric separates missing passage, irrelevant selection and empty negative", () => {
  const expected = { required: [{ id: "a", text: "final value" }], allowed: ["a", "related"] };
  assert.equal(scoreSources(expected, [{ id: "a", excerpt: "old value" }]).complete, false);
  const mixed = scoreSources(expected, [{ id: "a", quote: "final value" }, { id: "noise", quote: "unrelated" }]);
  assert.equal(mixed.complete, true); assert.equal(mixed.precision, 0.5); assert.equal(mixed.irrelevant, 1);
  const negative = { required: [], allowed: [] };
  assert.equal(scoreSources(negative, []).complete, true);
  assert.equal(scoreSources(negative, [{ id: "noise", quote: "anything" }]).complete, false);
});

test("independent integrity check refuses rewritten text, forged hashes, lines and foreign IDs", async t => {
  const root = await mkdtemp(join(tmpdir(), "akasha-benchmark-test-")); t.after(() => rm(root, { recursive: true, force: true }));
  const id = "Projects/example/entities/test.md"; await mkdir(join(root, "Projects/example/entities"), { recursive: true });
  const source = "header\n\nExact Ω passage.\n"; await writeFile(join(root, id), source);
  const packet = { evidence: [{ id }] };
  const match = { id, fingerprint: "sha256:" + createHash("sha256").update(source).digest("hex"),
    quote: "Exact Ω passage.\n", line_start: 3, line_end: 3, links: [] };
  await verifySources(root, packet, { kind: "source_passages", matches: [match] });
  for (const change of [{ quote: "Invented Ω passage.\n" }, { fingerprint: "sha256:forged" }, { line_start: 2 }, { line_end: 4 }, { id: "Projects/foreign/entities/test.md" }]) {
    await assert.rejects(verifySources(root, packet, { kind: "source_passages", matches: [{ ...match, ...change }] }));
  }
});
