import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { chmod, copyFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import http from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { localChat, requestLocal, chooseContext } from "./local-librarian.mjs";
const packet = { question: "What failed?", evidence: [{ citation: 1, excerpt: "The cache failed." }] };
async function fixture(t, scenario = "normal") {
  const directory = await mkdtemp(join(tmpdir(), "akasha-llama-test-"));
  const model = join(directory, "model.gguf"); const server = join(directory, "worker");
  await writeFile(model, "GGUF" + scenario);
  await copyFile(new URL("./fixtures/librarian-worker.mjs", import.meta.url), server); await chmod(server, 0o700);
  t.after(() => rm(directory, { recursive: true, force: true }));
  return { model, server, modelSha256: createHash("sha256").update("GGUF" + scenario).digest("hex"),
    events: async () => (await readFile(model + ".events", "utf8")).trim().split("\n").map(JSON.parse) };
}
function stopped(events) {
  const starts = events.filter(event => event.start);
  assert(starts.length > 0);
  for (const event of starts) {
    assert(events.some(end => end.stopped === event.start));
    assert.throws(() => process.kill(event.start, 0), { code: "ESRCH" });
  }
}
test("owned authenticated worker uses local-only arguments, tokenized schema request and exits", async t => {
  const f = await fixture(t); const result = await localChat(packet, f); const events = await f.events();
  assert.equal(result.metrics.context, 4096); assert.equal(result.metrics.prompt_tokens, 200);
  const args = events.find(event => event.start).args;
  for (const flag of ["--offline", "--no-webui", "--no-agent", "--no-context-shift"]) assert(args.includes(flag));
  assert(!args.includes("--tools")); assert.equal(args[args.indexOf("--cache-ram") + 1], "0");
  const rendered = events.find(event => event.path === "/apply-template").body;
  assert.equal(rendered.messages[1].content, JSON.stringify(packet));
  const sent = events.find(event => event.path === "/completion").body;
  assert.equal(sent.prompt.length, 200); assert.equal(sent.n_predict, 1024); assert(sent.json_schema);
  assert.equal(sent.cache_prompt, false); stopped(events);
});
test("larger prompts restart at exact budget and release both workers", async t => {
  const f = await fixture(t, "larger"); const result = await localChat(packet, f); const events = await f.events();
  assert.equal(result.metrics.context, 8192); assert.equal(events.filter(event => event.start).length, 2); stopped(events);
});
test("oversized token budget refuses before generation and stops worker", async t => {
  const f = await fixture(t, "too-large"); await assert.rejects(localChat(packet, f), /context budget/);
  const events = await f.events(); assert(!events.some(event => event.path === "/completion")); stopped(events);
});
test("context arithmetic reserves output and distinguishes bytes from tokens", () => {
  assert.equal(chooseContext(303), 4096); assert.equal(chooseContext(3040), 4096);
  assert.equal(chooseContext(3041), 8192); assert.equal(chooseContext(9000, 16384), 16384);
  assert.throws(() => chooseContext(8000), /context budget/); assert.throws(() => chooseContext(1, 0), /Invalid/);
});
test("no matches skip all model access; hash and remote-path failures precede worker startup", async t => {
  assert.equal((await localChat({ evidence: [] }, {})).metrics.skipped, true);
  const f = await fixture(t);
  await assert.rejects(localChat(packet, { ...f, modelSha256: "0".repeat(64) }), /SHA-256 mismatch/);
  await assert.rejects(localChat(packet, { ...f, model: "https://example.invalid/model" }), /local binary/);
  await assert.rejects(localChat({ evidence: ["x".repeat(16001)] }, f), /16000/);
  await assert.rejects(readFile(f.model + ".events"), { code: "ENOENT" });
});
for (const scenario of ["truncated", "limit", "drift"]) test(`${scenario} refuses and stops owned worker`, async t => {
  const f = await fixture(t, scenario);
  await assert.rejects(localChat(packet, f), scenario === "drift" ? /changed/ : /incomplete/); stopped(await f.events());
});
test("cancellation stops generation and releases worker", async t => {
  const f = await fixture(t, "hang"); const controller = new AbortController();
  const waiting = localChat(packet, { ...f, signal: controller.signal });
  const rejection = assert.rejects(waiting, /abort/i);
  let generating = false;
  for (let i = 0; i < 200; i++) {
    try { generating = (await f.events()).some(event => event.path === "/completion"); } catch { /* not started */ }
    if (generating) break; await delay(10);
  }
  assert(generating); controller.abort(); await rejection; stopped(await f.events());
});
test("absolute startup/request deadline stops worker", async t => {
  const f = await fixture(t, "hang"); await assert.rejects(localChat(packet, { ...f, timeoutMs: 1000 }), /abort|timeout/i);
  stopped(await f.events());
});
test("worker startup failure and wrong build fail without generation", async t => {
  const f = await fixture(t, "startup-failure"); await assert.rejects(localChat(packet, f), /failed to start/);
  await writeFile(f.server, '#!/usr/bin/env node\nconsole.log("wrong build")\n');
  await assert.rejects(localChat(packet, f), /pinned/);
});
test("HTTP redirects, malformed JSON and oversized responses refuse", async t => {
  const server = http.createServer((req, res) => {
    if (req.url === "/redirect") { res.writeHead(302, { Location: "https://example.invalid" }); res.end(); }
    else res.end(req.url === "/large" ? "x".repeat(2 ** 20 + 1) : "not JSON");
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  t.after(() => { server.closeAllConnections(); server.close(); });
  const port = server.address().port;
  await assert.rejects(requestLocal(port, "/redirect"), /redirects/);
  await assert.rejects(requestLocal(port, "/bad"), /JSON/);
  await assert.rejects(requestLocal(port, "/large"), /1 MiB/);
});
