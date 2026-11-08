# Bounded local investigator experiment

This development adapter lets a local model request follow-up evidence through a fixed read-only
tool menu. It returns original source passages and an explicit finished/incomplete status.
Relevance, answer support and production device budgets remain evaluation concerns. It does not
generate an authoritative answer or publish memory.

Optional `--expand-query` adds one question-only local planning step for synonyms and translations
into English, Russian and Simplified Chinese. It keeps the initial lexical candidates and adds a
core search with up to eight validated literal terms. The model then selects from the retained
sources. Expansion terms are retrieval hypotheses, never project facts. The expansion completion
sees no notes, titles, paths or registry. Its response, validated terms and timings appear in the
trace. Invalid output refuses; an empty term list keeps the initial results.

The step uses one of the existing three model decisions, and its nonempty search uses one of
the four retrieval calls. The 120-second deadline and input/evidence/source checks still apply.
This option adds cold inference cost and can produce irrelevant candidates; it is experimental.
For example, append `--expand-query --question "Что делать с истёкшим ключом доступа перед повторным соединением?"`
to an otherwise configured memory request. The original question stays bound to the core packet;
generated terms are passed as explicit literal substrings. Omit the flag for the previous behavior.

Run the independent fictional comparison, or replay its exact decisions without inference:

```sh
node scripts/benchmark-query-expansion.mjs --out /tmp/memory-retrieval-NEW --live \
  --server /path/to/llama-server --model /path/to/model.gguf \
  --model-sha256 EXPECTED_64_CHARACTER_LOWERCASE_SHA256 --gpu-layers 99
node scripts/benchmark-query-expansion.mjs --out /tmp/memory-replay-NEW \
  --replay /tmp/memory-retrieval-NEW
```

The benchmark separates initial-packet coverage, retained expanded evidence and final selected
passages. It saves exact decision inputs/outputs, checks hashes/physical source lines and source
inventory/modes, and rejects replay input drift. It measures source navigation on 12 synthetic
notes and 16 fixed questions; it does not measure factual answer generation or production use.

The 2026-11-08 fixed Qwen3.5-9B UD-Q4_K_XL study recovers 8/10 paraphrases versus 2/10 in
the initial lexical packets; necessary-span coverage across all positive cases is 12/14 versus
5/14. All sixteen runs finish, exact-match coverage is retained, the dedicated hostile-note
control selects only its relevant source, and both negative/scope controls select nothing.
All 25 fixture files remain exact. The English/Russian upload paraphrases still miss, and three
irrelevant final passages remain across the study. Median wall time is 80.38 seconds with the
existing cold workers. This is same-author synthetic validation; initial packet coverage is not
a separate no-expansion model-agent trial. Keep expansion opt-in and source relevance unverified.

The investigator now supports a direct `symbol` action for complete source of one exact,
unqualified ASCII identifier. With CodeGraph enabled, the host performs bounded JSON lookup,
requires one exact match, validates its repository file and fetches file-pinned symbol source.
It checks complete source-line coverage and returns the whole report only when it fits 4,000
Unicode code points. Missing, ambiguous, stale, incompatible and oversized results fail loudly;
the symbol is never silently clipped. Generic `explore` remains useful for discovering names
and call paths, with explicitly partial excerpts. Whole-symbol reports carry
`kind: "codegraph_symbol"`, `symbol` name/file/source lines, exact fingerprint/range and zero
omissions. These are CodeGraph-provided structural sources, not independently core-verified code.

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

For the selected Qwen3.5-9B UD-Q4_K_XL on an explicitly configured GPU-capable runtime, add
`--gpu-layers 99 --threads 4 --max-context 8192 --tool-catalog` and ask, for example,
`--question "How does tokenizeLibrarianPrompt reject transformed user payloads?"`.
Supply the actual local GGUF and its verified hash; no binary/model download is automatic.
The investigator exposes the same resource bounds as the selector: GPU layers `0..999`
(default `0`), threads `1..256` (default `4`), maximum context `4096|8192|16384|32768`
(default `8192`). Context is still selected from real prompt tokens and reserves output space.

For a reliable named-source request, add `--symbol tokenizeLibrarianPrompt`. The host loads that
exact validated complete source before inference, counts it as one of the existing four logical
actions, and records `initial_symbol` in the trace. This is explicit user-selected context; the
agent can still make bounded follow-up requests. Without the option, the model chooses whether
to use `symbol`; autonomous routing quality remains an evaluation concern.

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
neighbors, discover code, read one exact symbol, or finish by selecting known numeric source handles. All canonical retrieval and
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

Exploration selects exact query-focused windows within the existing 4,000-code-point total,
with a prefix fallback when no query word/identifier matches. It treats the report as opaque text.
Each passage adds `report_range` (half-open Unicode-code-point `start`, `end`, full `total`,
and `unit`), `selection`, and aggregate `omitted_code_points`. Separated passages have distinct
handles, even if their quoted text is identical. Plain output discloses ranges, omissions and
structural provenance. Selected full reports are re-requested once each, and the host also checks
passage text/metadata against retained originals. All metadata counts toward the existing byte
budgets; these checks establish report identity, not relevance or whole-function coverage.
The failed window experiments are retained as historical evidence. Complete function context
uses `symbol`: the JSON lookup and source request share one 30-second logical-action deadline,
with fixed executable, repository and file pinning. Neither filenames nor command options come
from the model. Metadata/file/report identity and complete line coverage are checked again
before display. A Unicode or qualified symbol name is outside this first action contract and
refuses; an ambiguous name is not resolved by guessing.

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
For an explicitly supplied GPU-capable runtime, the comparison also accepts `--gpu-layers N`
(`0..999`, default `0`) and records the requested offload count. Use the same setting/backend
across model candidates and measure actual device allocation; a requested count is not proof
of complete GPU placement.

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
