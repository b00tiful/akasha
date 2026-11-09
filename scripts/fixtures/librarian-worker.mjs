#!/usr/bin/env node
// Synthetic child-process fixture: no model weights or real inference.
import http from "node:http";
import { readFileSync, appendFileSync, writeFileSync } from "node:fs";
const args = process.argv.slice(2);
if (args.includes("--version")) { process.stderr.write("build 12110, commit cd959bae9\n"); process.exit(0); }
const value = name => args[args.indexOf(name) + 1];
const model = value("--model");
const scenario = readFileSync(model, "utf8").slice(4);
const key = readFileSync(value("--api-key-file"), "utf8").trim();
appendFileSync(model + ".events", JSON.stringify({ start: process.pid, args }) + "\n");
if (value("--gpu-layers") === "auto" && value("--log-verbosity") === "4") {
  process.stderr.write("load_tensors: offloaded 12/");
  await new Promise(accept => setTimeout(accept, 10));
  process.stderr.write("33 layers to GPU\n");
}
if (scenario === "startup-failure") {
  process.stderr.write("界".repeat(5000) + `\n\u001b[31mstartup failed ${key} ${model} ${value("--api-key-file")}\u0000\n`);
  process.exit(2);
}
let grow = false;
const server = http.createServer(async (request, response) => {
  if (request.headers.authorization !== `Bearer ${key}`) { response.writeHead(401); response.end("{}"); return; }
  const chunks = []; for await (const chunk of request) chunks.push(chunk);
  const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined;
  appendFileSync(model + ".events", JSON.stringify({ path: request.url, body }) + "\n");
  let result;
  if (request.url === "/health" && (scenario === "startup-hang" || scenario === "restart-hang" && value("--ctx-size") === "8192")) return;
  if (request.url === "/health") result = { status: "ok" };
  if (request.url === "/props") result = { total_slots: 1, default_generation_settings: {
    n_ctx: Number(value("--ctx-size")) - (scenario === "fit-context-shrink" ? 512 : 0) } };
  if (request.url === "/apply-template" && !body.messages[1].content.startsWith("AKASHA_PAYLOAD_")) grow = body.messages[1].content.includes("grow-context");
  if (request.url === "/apply-template") result = { prompt: body.messages.map(message =>
    `<|im_start|>${message.role}\n${scenario === "template-transform" ? message.content.replaceAll("<", "&lt;") : message.content}<|im_end|>\n`).join("") + "<|im_start|>assistant\n" };
  if (request.url === "/tokenize") {
    const tokens = body.add_special ? Array((scenario === "larger" || scenario === "restart-hang" || scenario === "session-grow" && grow) ? 3500 : scenario === "too-large" ? 8000 : 0).fill(1) : [];
    let content = body.content;
    while (content.length) {
      const special = body.parse_special && content.match(/^<\|im_(start|end)\|>/);
      if (special) { tokens.push(special[1] === "start" ? 151644 : 151645); content = content.slice(special[0].length); }
      else { const point = content.codePointAt(0); tokens.push(point); content = content.slice(point > 65535 ? 2 : 1); }
    }
    if (scenario === "boundary-merge" && body.add_special) tokens.push(999999);
    result = { tokens: scenario === "invalid-tokens" ? ["invalid"] : tokens };
  }
  if (request.url === "/completion") {
    if (scenario === "hang") return;
    if (scenario === "drift") writeFileSync(model, "GGUFchanged");
    result = { stop: true, truncated: scenario === "truncated", stop_type: scenario === "limit" ? "limit" : "eos",
      content: scenario === "investigator-invalid" ? '{"action":"invalid"}' : scenario === "investigator"
        ? JSON.stringify(body.json_schema.properties?.terms ? { terms: [] } : { action: "finish", handles: [] })
        : scenario === "provenance" ? '{"selections":[{"citation":1}]}'
        : scenario === "answer" ? JSON.stringify(body.json_schema.properties.parts?.items.type === "string"
          ? { parts: ["How does FlightPath store waypoints?"] }
          : { uncovered_question: false, parts: [{ part: 1, sufficient: true,
            quotes: ["FlightPath stores waypoints in a vector."] }] })
        : '{"selections":[]}', timings: { predicted_n: 8 } };
  }
  response.end(JSON.stringify(result));
});
server.listen(Number(value("--port")), "127.0.0.1");
process.on("SIGTERM", () => {
  if (scenario === "ignore-term") return;
  appendFileSync(model + ".events", JSON.stringify({ stopped: process.pid }) + "\n");
  server.closeAllConnections(); server.close(() => process.exit(0));
});
