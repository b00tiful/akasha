import assert from "node:assert/strict";
import { once } from "node:events";
import http from "node:http";
import test from "node:test";
import { localChat, requestLocal } from "./local-librarian.mjs";

const packet = { question: "What failed?", evidence: [{ citation: 1, excerpt: "The cache failed." }] };
const metadata = { name: "test:local", digest: "abc", size: 1234 };
async function worker(t, override = () => undefined) {
  const calls = [];
  const server = http.createServer(async (request, response) => {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined;
    calls.push({ path: request.url, body });
    if (override(request, response, body, calls)) return;
    const result = request.url === "/api/tags" ? { models: [metadata] }
      : request.url === "/api/show" ? { model_info: { architecture: "test" } }
      : request.url === "/api/chat" ? { done: true, done_reason: "stop", message: { content: '{"insufficient_evidence":true,"claims":[]}' } }
      : { done: true };
    response.writeHead(200); response.end(JSON.stringify(result));
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  t.after(() => { server.closeAllConnections(); server.close(); });
  return { port: server.address().port, calls };
}

test("local-only preflight, bounded settings, exact evidence, unload and no tools", async t => {
  const w = await worker(t); const result = await localChat(packet, { ...w, model: "test:local" });
  assert.equal(result.metrics.digest, "abc");
  const sent = w.calls.find(call => call.path === "/api/chat").body;
  assert.equal(sent.messages[1].content, JSON.stringify(packet));
  assert.equal(sent.think, false); assert.equal(sent.keep_alive, 0); assert.equal(sent.tools, undefined);
  assert.equal(w.calls.at(-1).path, "/api/generate");
});

test("remote metadata is rejected before any private evidence request", async t => {
  const w = await worker(t, (request, response) => {
    if (request.url === "/api/show") { response.end(JSON.stringify({ remote_host: "https://example.invalid", model_info: {} })); return true; }
  });
  await assert.rejects(localChat(packet, { ...w, model: "test:local" }), /Remote models/);
  assert(!w.calls.some(call => call.path === "/api/chat"));
});

test("no matches skip the worker, cloud tags and oversized evidence refuse", async () => {
  const result = await localChat({ evidence: [] }, { model: "test:local", port: 1 });
  assert.equal(result.metrics.skipped, true);
  await assert.rejects(localChat(packet, { model: "test:cloud" }), /local model/);
  await assert.rejects(localChat({ evidence: ["x".repeat(16001)] }, { model: "test:local" }), /16000/);
});

test("truncated responses refuse and still unload", async t => {
  const w = await worker(t, (request, response) => {
    if (request.url === "/api/chat") { response.end(JSON.stringify({ done: true, done_reason: "length", message: { content: "{}" } })); return true; }
  });
  await assert.rejects(localChat(packet, { ...w, model: "test:local" }), /incomplete/);
  assert.equal(w.calls.at(-1).path, "/api/generate");
});

test("redirects are never followed and malformed JSON refuses", async t => {
  const w = await worker(t, (request, response) => {
    if (request.url === "/redirect") { response.writeHead(302, { Location: "https://example.invalid" }); response.end(); return true; }
    if (request.url === "/bad") { response.end("not JSON"); return true; }
  });
  await assert.rejects(requestLocal(w.port, "/redirect"), /redirects/);
  await assert.rejects(requestLocal(w.port, "/bad"), /JSON/);
});

test("absolute timeout and cancellation terminate a stuck response", async t => {
  const w = await worker(t, (request) => request.url === "/hang");
  await assert.rejects(requestLocal(w.port, "/hang", undefined, undefined, 40), /abort/i);
  const controller = new AbortController();
  const waiting = requestLocal(w.port, "/hang", undefined, controller.signal);
  controller.abort(); await assert.rejects(waiting, /abort/i);
});
