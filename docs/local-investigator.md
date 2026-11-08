# Bounded local investigator experiment

This development adapter lets a local model request follow-up evidence through a fixed read-only
tool menu. It returns original source passages and an explicit finished/incomplete status.
Relevance, answer support and production device budgets remain evaluation concerns. It does not
generate an authoritative answer or publish memory.

The first same-model Qwen3 4B comparison fails utility acceptance: affirmative memory coverage is
1/4 versus deterministic 3/4, and the model never chooses CodeGraph for the code tasks. The host
passes 28 tests/eight exact decision replays. Keep this as an explicit experiment; further planner
profiles require a new direction after the two preserved failed utility profiles.

The directed follow-up adds an opt-in `--tool-catalog` control to both commands below. It puts
fixed action descriptions and the exact currently enforced JSON action schemas into the model's
input. Available note IDs/source handles and optional CodeGraph access come from the host;
exhausted budgets advertise only finish. The catalog counts toward the same 16,000-byte input
limit and is retained with each decision in the trace. Omit the flag to retain the previous
payload and decoder behavior. This is a JSON-action framing experiment, not native function calling.
Its same-model usefulness/resource evaluation is separate from operational test acceptance.
The completed catalog profile finishes 8/8 and retains 3/4 affirmative memory cases, matching
deterministic navigation. It still selects no CodeGraph actions (0/14 decisions), covers 0/2 code
tasks and retains five irrelevant memory passages. Median memory wall time is 33.92 s versus
0.68 s deterministic and 17.61 s selector in that run. It fails utility acceptance and remains
opt-in; these are synthetic single-host/historical-comparison observations. Final 31 tests,
eight default replays and eight core-backed catalog replays pass. Empty CodeGraph output now
refuses before becoming source evidence.

Build the existing normal CLI, then use the same pinned llama.cpp b12110 and explicit GGUF/hash
as the [source-selector pilot](local-librarian.md):

```sh
node scripts/local-investigator.mjs --root /path/to/memory --project example \
  --server /path/to/llama-server --model /path/to/model.gguf \
  --model-sha256 EXPECTED_64_CHARACTER_LOWERCASE_SHA256 \
  --question "Find the recorded connection between the incident and its repair" \
  --trace /path/to/new-private-trace.json
```

Plain text is default. Explicit `--json` returns `kind: "investigation_sources"`, `status`,
`reason`, selected `matches`, inspected handles and the bounded action/observation trace. A
finished run means the model stopped searching; it does not establish that the question is
answered. Empty matches mean no source was selected, not that the knowledge does not exist.
Incomplete results exit 2 with guidance on stderr. Runtime, source, scope and action failures
exit nonzero without successful output. Optional trace files use create-only mode and Unix
owner-only permissions; treat their source excerpts like project code. No automatic chat history
is written. Traces preserve unfinished work for inspection; automatic resumption is not supported.
Questions and tool queries are limited to 160 control-free characters in this experiment.

The model can search project notes, inspect a previously discovered note and its recorded
neighbors, or finish by selecting known numeric source handles. All canonical retrieval and
source revalidation go through the existing Rust core CLI. There are no write, shell, Git,
network-fetch or user-selected filesystem tools. Each whole user payload retains literal
tokenization; source text cannot introduce conversation-control tokens.

Optionally add `--codegraph /path/to/codegraph` to expose one structural exploration action.
The repository comes from core resolution, and an existing non-symlink `.codegraph/` directory
is required. The adapter never initializes or synchronizes indexes. Only a query is model-selected;
the executable, repository and two-file bound are application-owned. Exact reports are bounded,
hashed, labeled `codegraph_report`, and re-requested before display. Their hash establishes report
identity, not independent verification of code facts or call-edge accuracy. Truncated reports
are explicit. Existing CodeGraph trust, index freshness and its own telemetry policy still apply;
this is not a project-wide privacy mode. Omit the option for memory-only work.

Every run has a 120-second deadline, at most three model decisions and four retrieval calls
including the initial search. Each tool has a 30-second limit. Decision payloads are capped at
16,000 bytes, retained evidence at 18 entries/64 KiB, and final selection at six handles. Working
context contains the latest passages, bounded preceding actions and an inventory of earlier
handles. When the step/tool budget cannot support another read, the advertised menu contains
only source selection. Repeated actions, exhausted budgets and overflow remain explicit unfinished
states. Selected source
packets, CodeGraph reports and project/repository binding are checked before display. Source
drift refuses; there is no silent fallback. Per-decision workers preserve the existing cleanup
contract and add repeated model-loading/verification cost. CPU/four threads are the experiment
baseline; model/device packaging and persistent workers remain separate work.

Run the frozen comparison with a new output directory:

```sh
node scripts/benchmark-local-investigator.mjs --out /tmp/investigator-NEW \
  --server /path/to/llama-server --model /path/to/model.gguf \
  --model-sha256 EXPECTED_64_CHARACTER_LOWERCASE_SHA256 \
  --codegraph /path/to/codegraph
```

Add `--tool-catalog` for the explicit-catalog profile. Each new comparison directory includes
`profile.json` recording the flag, model identity and CPU-thread setting, alongside source pins.

The six memory tasks reuse fictional notes and predeclared labels. Two separate code smoke tasks
query this workspace's existing CodeGraph index; when CodeGraph is omitted they are explicitly
skipped. Output preserves frozen source/task/binary hashes, the exact historical selector packet,
raw decisions, core-checked passages and completion/coverage scores. The deterministic path uses
initial search plus two discovered-note anchor reads. Code tasks additionally get one direct
CodeGraph query and are reported separately from memory quality. Missing-owner cases score
navigation rather than an ownership answer. This is same-author synthetic/smoke evidence, not
an independent real-project benchmark.

```sh
node --test scripts/local-investigator.test.mjs scripts/local-librarian.test.mjs \
  scripts/benchmark-local-librarian.test.mjs
```
