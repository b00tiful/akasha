#!/usr/bin/env node
// Model-free handoff from exported corrections to the existing Rust approved writer.
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { chmod, mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual, parseArgs, promisify } from "node:util";
import { renderOnboardingReview } from "./onboarding-review.mjs";

const execute = promisify(execFile);
const workspace = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Snapshot bounded regular inputs before handing private copies to subprocesses.
async function loadJson(path, limit) {
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > limit) throw new Error(`Expected a regular JSON file of at most ${limit} bytes: ${path}`);
    const bytes = Buffer.alloc(limit + 1);
    let length = 0;
    while (length < bytes.length) {
      const { bytesRead } = await file.read(bytes, length, bytes.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > limit) throw new Error(`JSON input exceeds ${limit} bytes: ${path}`);
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, length)));
  } finally { await file.close(); }
}

async function withCore(options, action) {
  if (!options.root || !options.project) throw new Error("Required root and project");
  const signal = AbortSignal.any([options.signal ?? new AbortController().signal, AbortSignal.timeout(120_000)]);
  signal.throwIfAborted();
  const cli = resolve(options.cli ?? join(workspace, "target/debug/akasha"));
  const identity = async () => createHash("sha256").update(await readFile(cli)).digest("hex");
  const cliHash = await identity();
  const temporary = await mkdtemp(join(tmpdir(), "akasha-onboarding-review-"));
  try {
    await chmod(temporary, 0o700);
    const save = async (name, value) => {
      signal.throwIfAborted();
      const path = join(temporary, name);
      await writeFile(path, JSON.stringify(value), { flag: "wx", mode: 0o600 });
      return path;
    };
    const run = async args => {
      signal.throwIfAborted();
      if (await identity() !== cliHash) throw new Error("CLI changed during onboarding review");
      const { stdout } = await execute(cli, ["--root", resolve(options.root), "--project", options.project, ...args],
        { signal, timeout: 30_000, maxBuffer: 2 ** 20 });
      if (await identity() !== cliHash) throw new Error("CLI changed during onboarding review");
      signal.throwIfAborted();
      return stdout;
    };
    return await action({ signal, save, run });
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

export async function prepareReviewedOnboarding(options) {
  if (!options.capture || !options.plan || !options.output) throw new Error("Required capture, plan and output paths");
  return withCore(options, async ({ signal, save, run }) => {
    const packet = await loadJson(join(options.capture, "packet.json"), 16_000);
    const plan = await loadJson(options.plan, 16_000);
    let discovery;
    try { discovery = await loadJson(join(options.capture, "discovery.json"), 262_144); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    if (discovery && (!isDeepStrictEqual(discovery.packet, packet) || !Array.isArray(discovery.coverage?.omitted)
      || !Array.isArray(discovery.coverage?.selected) || !Number.isSafeInteger(discovery.coverage?.excluded_entries))) {
      throw new Error("Captured discovery does not describe this source packet");
    }
    const packetPath = await save("packet.json", packet), planPath = await save("plan.json", plan);
    const draft = JSON.parse(await run(["--json", "local-onboarding-compile", "--packet", packetPath, "--plan", planPath]));
    const proposalPath = await save("proposal.json", draft.proposal);
    const review = await run(["local-onboarding-preview", "--packet", packetPath, "--proposal", proposalPath]);
    // Rebuild source groups in Rust; do not trust optional captured identity artifacts.
    const identityPlan = [4, 5].includes(plan.schema_version)
      ? JSON.parse(await run(["--json", "local-onboarding-plan", "--packet", packetPath])) : undefined;
    const html = renderOnboardingReview({ packet, plan, draft, identityPlan, discovery });
    const requested = resolve(options.output);
    const directory = join(await realpath(dirname(requested)), basename(requested));
    for (const boundary of [packet.preparation.root, packet.preparation.repository_dir]) {
      const location = relative(boundary, directory);
      if (!location || (location !== ".." && !location.startsWith(".." + sep) && !isAbsolute(location))) {
        throw new Error("Save review bundles outside the configured memory root and repository");
      }
    }
    signal.throwIfAborted();
    await mkdir(directory, { mode: 0o700 }); // Create-only: preserve every earlier review.
    try {
      const artifacts = { "packet.json": JSON.stringify(packet), "plan.json": JSON.stringify(plan),
        "draft.json": JSON.stringify(draft), "proposal.json": JSON.stringify(draft.proposal),
        "review.txt": review, "review.html": html };
      if (identityPlan) artifacts["identity-plan.json"] = JSON.stringify(identityPlan);
      if (discovery) artifacts["discovery.json"] = JSON.stringify(discovery);
      for (const [name, value] of Object.entries(artifacts)) {
        signal.throwIfAborted();
        await writeFile(join(directory, name), value, { flag: "wx", mode: 0o600 });
      }
      signal.throwIfAborted();
    } catch (error) {
      await rm(directory, { recursive: true, force: true });
      throw error;
    }
    return { directory, draft, review };
  });
}

export async function applyReviewedOnboarding(options) {
  if (!options.bundle || !/^sha256:[a-f0-9]{64}$/u.test(options.approvedPreviewId ?? "")) {
    throw new Error("Required bundle and full approved preview ID (sha256: followed by 64 lowercase hex digits)");
  }
  return withCore(options, async ({ save, run }) => {
    const packet = await loadJson(join(options.bundle, "packet.json"), 16_000);
    const proposal = await loadJson(join(options.bundle, "proposal.json"), 16_000);
    const packetPath = await save("packet.json", packet), proposalPath = await save("proposal.json", proposal);
    // No implicit approval, inference, refresh or retry. Rust checks current sources, scope,
    // proposal and approved preview again, including under the shared writer's lock.
    return run([...(options.json ? ["--json"] : []), "local-onboarding-apply", "--packet", packetPath,
      "--proposal", proposalPath, "--approved-preview-id", options.approvedPreviewId]);
  });
}

async function main() {
  const { values, positionals } = parseArgs({ allowPositionals: true, options: {
    root: { type: "string" }, project: { type: "string" }, capture: { type: "string" }, plan: { type: "string" },
    output: { type: "string" }, bundle: { type: "string" }, "approved-preview-id": { type: "string" },
    json: { type: "boolean", default: false },
  } });
  if (positionals.length !== 1 || !["prepare", "apply"].includes(positionals[0])) {
    throw new Error("Usage: local-onboarding-review.mjs prepare|apply --root ROOT --project PROJECT; see docs/local-onboarding.md");
  }
  const preparing = positionals[0] === "prepare";
  const allowed = preparing ? ["root", "project", "capture", "plan", "output", "json"]
    : ["root", "project", "bundle", "approved-preview-id", "json"];
  if (Object.keys(values).some(key => !allowed.includes(key))) throw new Error("Option does not belong to this review operation");
  const controller = new AbortController();
  const cancel = () => controller.abort(new Error("Onboarding review cancelled; inspect current state if application had started"));
  process.once("SIGINT", cancel); process.once("SIGTERM", cancel);
  try {
    const options = { ...values, approvedPreviewId: values["approved-preview-id"], signal: controller.signal };
    if (preparing) {
      const result = await prepareReviewedOnboarding(options);
      process.stdout.write(values.json ? JSON.stringify(result) + "\n"
        : `${result.review}\nReview bundle: ${result.directory}\nOpen review.html or read review.txt before applying with the full preview ID.\n`);
    } else process.stdout.write(await applyReviewedOnboarding(options));
  } finally { process.removeListener("SIGINT", cancel); process.removeListener("SIGTERM", cancel); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
}
