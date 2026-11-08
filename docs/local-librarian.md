# Local librarian source passages

The separate [bounded investigator experiment](local-investigator.md) can request scoped follow-up
reads. This source-selector command retains its existing one-packet behavior and policy.

The development runner uses llama.cpp to select useful **source IDs**. Rust supplies the exact
quoted passages, recorded status, physical line ranges and scoped wikilinks. The model cannot
supply an explanation, rewrite a quote or invent a displayed relationship through this interface.
Relevance still needs review: a passage can be real without answering the question. This is a
source-navigation prototype; the desktop panel, final name, model installer, local onboarding
and privacy mode remain later work. Ordinary core/CLI use does not require Node or a model.

Use Node 22 and the official llama.cpp **b12110 / commit cd959bae9** build from the
[official release](https://github.com/ggml-org/llama.cpp/releases/tag/b12110). Verify the archive's
SHA-256 against the GitHub release metadata and keep the executable with its libraries. The
runner checks the build and records executable/model hashes; it does not install or download.
Use a trusted local GGUF and its expected SHA-256. Model and runtime packages have separate
licenses. The existing Qwen3 4B Q4_K_M artifact is an evaluation candidate.

Build and inspect retrieval without inference:

```sh
cargo build --offline -p akasha-cli
target/debug/akasha --root /path/to/memory --project example \
  librarian-prepare "Find notes about the cache failure" --term cache
```

Run local source selection. On this development machine, persistent weights and complete
runtimes live outside the checkout in `~/.local/share/akasha/local-inference/`. Use durable
storage on other machines too; `/tmp` is suitable only for request scratch. The selected main
model example below requires these already provisioned files and never downloads them.

```sh
inference_dir="$HOME/.local/share/akasha/local-inference"
node scripts/local-librarian.mjs --root /path/to/memory --project example \
  --server "$inference_dir/runtimes/llama-b12110-vulkan/llama-server" \
  --model "$inference_dir/models/Qwen3.5-9B-UD-Q4_K_XL.gguf" \
  --model-sha256 6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293 \
  --question "Find notes about the cache failure" --term cache
```

Plain text is default; `--json` returns `kind: "source_passages"` and `matches`. Each match contains
its canonical ID, source fingerprint, recorded status, exact `quote`, line range, partial-body
flag and recorded links. Empty matches mean no passage was selected, not that the project lacks
an answer. Metrics go to stderr; errors are nonzero and do not display a successful result.

Repeat `--term` up to eight times for literal substring matching. Without explicit terms,
automatic queries discard common English lookup/function words and match full alphanumeric tokens
in bodies, canonical identities and recorded status. Filename/status matches receive additional
weight; selected notes and their neighbors retain priority. Status values have no hardcoded order;
this is not semantic or multilingual word segmentation. Optional
`--note Projects/example/entities/cache.md` prioritizes that exact canonical note and its outgoing
neighbors in the selected project. Folder paths follow configuration. Keep the original question
or useful terms even when selecting a note; unrelated notes are not evidence of a relationship.

Packets contain at most six 1,000-character excerpts within 16,000 serialized bytes. They carry
status, source hashes, lines and omission/partial flags. Projections, Global and other projects'
bodies are excluded. Shared validation can inspect foreign link-target metadata. Reads are not
an atomic filesystem snapshot. Regenerate disposable packets after retrieval changes; old
packets that no longer match current evidence refuse as stale.

Bodies fitting 640 characters stay whole. Longer bodies use the line with greatest query-term
overlap, then nearby complete lines within a 640-character context target. A long winning line
and no-body-match fallback can still use the 1,000-character hard cap. Excerpts preserve source
bytes and physical lines; partial coverage is explicit. Filename weighting and lexical windows
can still select an irrelevant passage or miss synonyms.

The worker returns only `{"selections":[{"citation":1}]}` using a grammar restricted to supplied
citation IDs. Rust rebuilds the packet, rejects drift/duplicate/unknown IDs and supplies each
selected excerpt. The independent checker also accepts an optional narrower `quote`, which must
be nonempty, occur exactly once in that excerpt (including overlapping occurrences), and match
its bytes exactly. Invented text, extra claims, controls and oversized inputs refuse.

```sh
target/debug/akasha --root /path/to/memory --project example \
  librarian-check-sources --packet packet.json --selection selection.json
```

Links come from the full original Markdown parser context. Only complete wikilinks inside the
quote targeting another identity in the current evidence packet, or a local fragment of the same
note, are displayed. Inline/fenced code, comments and partial links cannot create graph edges.
At most 20 distinct links per passage are shown; scoped/capped omissions are counted. A recorded
link is not proof of ownership or causality. The compact checked object is capped at 65,536 bytes.
JSON preserves quote bytes; terminal output sanitizes controls and prefixes source lines.

CPU is default, using up to four threads. `--gpu-layers 99` requests offload with a compatible
GPU package; `--threads N` is explicit. The runner applies the model's chat template, disables
thinking and tokenizes the user packet as literal data with special-token parsing disabled.
Trusted template controls retain special-token parsing. A random placeholder identifies the
payload boundary; exact template substitution and harmless-template token equality must hold,
or the runner refuses before generation. Templates that transform payload text or merge tokens
across this boundary are unsupported. Source quotes remain unchanged. This prevents note text
from supplying role-control tokens; ordinary-language instruction following and relevance still
need evaluation. The runner reserves 1,024 output tokens plus a margin and starts with
4K context. It restarts at a larger context only if needed, up to an 8K default maximum.
`--max-context` accepts 4096, 8192, 16384 or 32768. Overflow refuses without silent truncation.

Each call owns an authenticated numeric-loopback worker with a restricted environment, offline
mode, one slot, F16 cache, bounded batches, disabled Web UI/agent tools, no prompt cache or
context shifting. Expected GGUF hash, pinned worker build and observed executable/model drift
are checked. Completion/refusal/timeout/Ctrl-C stop the worker and remove owner-only scratch;
no-match retrieval skips model access. Abrupt parent death can leave a worker or scratch files.
Trusted packages remain necessary; this is not OS egress isolation or the deferred privacy mode.
No chat history or canonical writes are performed. Weights remain installed between calls.

The older `librarian-check --packet FILE --answer FILE` remains an explicitly invoked draft
checker for compatibility/research. Its valid citations do not prove factual support. The prior
Qwen3 4B free-form trial failed (8/16 expanded cases, 0/8 missing-information cases); source
selection does not overturn that result or establish answerability. The runner now uses the
source-passage checker by default.

External/cloud onboarding remains available. Local onboarding, changeable route preference and
local-only privacy mode are accepted later work. Run `node --test scripts/local-librarian.test.mjs`
for local transport/lifecycle tests and `cargo test --workspace` for Rust verification. Live model
checks must distinguish quote integrity, topical relevance and actual answer support.

The repeatable [retrieval benchmark](../tests/fixtures/librarian-benchmark/README.md) contains
fictional long notes, distractors, conflicting policies, multilingual questions and hostile note
instructions. It records retrieval coverage separately from model selection. No private project
is sent to a model by that harness, and it never downloads a runtime or weights.

Successful requests now report `phase_ms`, `worker_starts` and `total_ms` alongside the existing
metrics on stderr. Phases separate initial model verification, runtime verification, worker startup,
template/tokenization, any context-restart shutdown, completion, drift verification and cleanup.
`total_ms` includes local setup/cleanup; historical `elapsed_ms` excludes initial verification and
final cleanup. The command also reports `core_prepare_ms`, `core_check_ms` and `command_ms`.
No-match requests still skip all model access. Timing instrumentation does not relax hash,
authentication, token-budget or cleanup checks.


Trusted development adapters can use `createInferenceSession(options)` to own several sequential
completions in one bounded request, always closing it in `finally`. The original `localCompletion`
function remains the one-completion owner. The investigator exposes optional request reuse as
[`--worker-session`](local-investigator.md); this one-packet source-selector command keeps its
original lifetime. Every completion independently renders/tokenizes its full policy/input,
disables prompt caching and checks complete model/binary fingerprints before and after inference.
Session lifetime is absolute (at most 120 seconds), including idle tool reads. Failed/closed
sessions refuse further calls; idle cancellation also reaps the worker. Context can grow but
never shrinks during the request. Runtime identity verification remains deliberately repeated.
