#!/usr/bin/env node
// Synthetic child-process fixture: no model weights or real inference.
import http from "node:http";
import { readFileSync, appendFileSync, writeFileSync } from "node:fs";
const args = process.argv.slice(2);
if (args.includes("--version")) { process.stderr.write("build 12110, commit cd959bae9\n"); process.exit(0); }
const value = name => args[args.indexOf(name) + 1];
const model = value("--model");
const scenario = readFileSync(model, "utf8").slice(4);
if (scenario === "startup-failure") process.exit(2);
const key = readFileSync(value("--api-key-file"), "utf8").trim();
appendFileSync(model + ".events", JSON.stringify({ start: process.pid, args }) + "\n");
const server = http.createServer(async (request, response) => {
  if (request.headers.authorization !== `Bearer ${key}`) { response.writeHead(401); response.end("{}"); return; }
  const chunks = []; for await (const chunk of request) chunks.push(chunk);
  const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined;
  appendFileSync(model + ".events", JSON.stringify({ path: request.url, body }) + "\n");
  let result;
  if (request.url === "/health") result = { status: "ok" };
  if (request.url === "/props") result = { total_slots: 1, default_generation_settings: { n_ctx: Number(value("--ctx-size")) } };
  if (request.url === "/apply-template") result = { prompt: JSON.stringify(body.messages) };
  if (request.url === "/tokenize") result = { tokens: Array(scenario === "larger" ? 3500 : scenario === "too-large" ? 8000 : 200).fill(1) };
  if (request.url === "/completion") {
    if (scenario === "hang") return;
    if (scenario === "drift") writeFileSync(model, "GGUFchanged");
    result = { stop: true, truncated: scenario === "truncated", stop_type: scenario === "limit" ? "limit" : "eos",
      content: '{"selections":[]}', timings: { predicted_n: 8 } };
  }
  response.end(JSON.stringify(result));
});
server.listen(Number(value("--port")), "127.0.0.1");
process.on("SIGTERM", () => {
  appendFileSync(model + ".events", JSON.stringify({ stopped: process.pid }) + "\n");
  server.closeAllConnections(); server.close(() => process.exit(0));
});
