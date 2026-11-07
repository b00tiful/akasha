# Local librarian pilot

The development runner uses **llama.cpp**, selected on 2026-11-07. Rust retrieves saved project
notes and validates the generated answer; a short-lived, owned `llama-server` process performs
inference. The desktop panel, name, model installer, local onboarding and privacy mode remain
later work. The development runner requires Node; ordinary core/CLI use does not.

Use Node 22 and the official llama.cpp **b12110 / commit cd959bae9** build (the v0.9.0 release
points to this nightly). Download a build appropriate for your machine from the
[official release](https://github.com/ggml-org/llama.cpp/releases/tag/b12110), verify the archive's
SHA-256 against its GitHub release metadata, and keep its executable and libraries together.
The runner checks the build identifier and records binary/model hashes. It does not download or
install anything. Model weights have their own license; use a trusted local GGUF and its expected
SHA-256. The existing Qwen3 1.7B Q4_K_M artifact is an evaluation model, not a final quality choice.

Build the CLI:

```sh
cargo build --offline -p akasha-cli
```

Inspect retrieval without inference:

```sh
target/debug/akasha --root /path/to/memory --project example \
  librarian-prepare "What caused the cache failure?" --term cache
```

Run the local librarian:

```sh
node scripts/local-librarian.mjs --root /path/to/memory --project example \
  --server /path/to/llama-server --model /path/to/model.gguf \
  --model-sha256 EXPECTED_64_CHARACTER_LOWERCASE_SHA256 \
  --question "What caused the cache failure?" --term cache
```

CPU is the default, with up to four threads. A compatible GPU build can use `--gpu-layers 99`;
`--threads N` is explicit. The runner starts at 4K context, applies the model's chat template with
thinking disabled, tokenizes the complete prompt and reserves 1,024 output tokens plus a safety
margin. It restarts at a larger power-of-two context only when needed. Default maximum is 8K;
`--max-context` accepts 4096, 8192, 16384 or 32768. Oversized prompts refuse before generation;
narrow the terms or explicitly increase the maximum. Evidence bytes are not token counts.

The worker has one request slot, F16 cache, bounded batch sizes, no prompt cache/checkpoints or
context shifting, offline mode, disabled Web UI/agent integration and no model tools. Its
numeric-loopback channel uses a random authentication key kept in an owner-only temporary file.
The runner supplies a restricted environment and verifies authenticated worker properties before
sending evidence. It checks GGUF magic, expected model hash, pinned worker version and observed
binary/model drift. A trusted binary/model is still required; these measures are not OS egress
isolation or the deferred project-wide privacy mode.

Completion, refusal, timeout and Ctrl-C stop the owned worker, escalating from termination to
kill if needed, and remove its scratch directory. No-match retrieval skips all model access.
A hard kill of the parent can leave a worker or scratch files; production host supervision is
still future work. There is no persistent inference service, saved chat history or canonical
note mutation in this slice. The earlier Ollama `--port` and model-tag interface has been replaced
by explicit `--server`, GGUF `--model` and `--model-sha256` arguments. Model files stay in place.

Repeat `--term` up to eight times for literal matching; omit it for simple query-word overlap.
Optional `--note Projects/example/entities/cache.md` prioritizes that exact canonical note and
its outgoing neighbors in the selected project. Paths follow root configuration. The packet
contains at most six excerpts of 1,000 Unicode characters within 16,000 serialized bytes, with
status, source hashes, line ranges and omission/truncation counts. It is not a complete codebase
analysis. Projections, Global notes and foreign project bodies are excluded; shared validation
can inspect foreign link-target metadata. The filesystem reads do not form an atomic snapshot.

Rust rebuilds the packet and checks answer shape, source drift and citation membership. It
rejects invalid JSON, unknown fields, invalid/duplicate citations, controls and excess text.
**A valid citation does not prove that its passage supports the claim.** The small evaluation
model can hallucinate; answers remain drafts, and representative quality is an open gate. An
official Qwen3 4B Q4_K_M trial also failed: the final expanded profile passed 8/16 synthetic
cases and 0/8 missing-information cases. It is not a recommended production answer model.
Use `librarian-prepare` to inspect original evidence independently of generated claims.

`librarian-check --packet FILE --answer FILE` exposes the check independently. Plain output is
default; `--json` returns checked claims/evidence. Runner metrics go to stderr. Errors are nonzero
and never produce a successful answer. Temporary answer files are owner-only and normally removed;
local logs, swap and backups still need the later privacy policy.

Onboarding retains the external/cloud-assisted route. Local onboarding and a changeable route
preference are accepted later work; this runner does not implement that selector. Privacy mode
is also explicitly deferred and will permit local model options only.

Run `node --test scripts/local-librarian.test.mjs` for transport/lifecycle/context tests (temporary
loopback sockets are needed), and `cargo test --workspace` for the Rust core/CLI. Real-model
acceptance is separate and does not establish general quality or hardware support.
