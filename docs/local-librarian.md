# Local librarian: source passages and cited answers

The desktop now includes **Librarian sources** (the `§` toolbar button, also available in the note
inspector). Open a configured root/project, enter specific names or words recorded in its notes,
and choose **Find sources**. The panel shows exact bounded passages with physical lines, recorded
status, partial-body flags and retrieval omissions. It uses deterministic saved-memory retrieval
and needs no model installation. No match is explicit; try more specific recorded terms.

Enable **Inspect recorded repository evidence** to see onboarding claims' fact/inference/unknown
labels, rationales and original code windows. This explicitly reads recorded references in the
selected project's registered repository; it defaults off. Changed or unavailable code is labeled
and withheld. Exact bytes and matching hashes do not establish that a claim is correct.

**Open** rechecks the citation packet and full note fingerprint before loading the existing
editor. Stale/altered packets or pending journals refuse without recovery or writes. Inspection
uses saved files even with unsaved edits; choose Save or Discard before opening another note.
Cancel, query edits, closing or changing projects suppress obsolete results immediately; already
running bounded native reads may finish. Query/results and repository preference are not persisted.
Desktop **Review local onboarding** now prepares scoped repository sources, retains their exact
packet for local CLI generation, and corrects/publishes the resulting draft into this librarian;
see [the workflow](local-onboarding.md). Model-backed chat and installed generation remain separate work.
[Interactive offline demonstration](../akasha/Research/evidence/2026-11-10-desktop-librarian/demo.html)
uses the actual component with saved native-adapter results.

Use `--all-sources` for a complete review of the bounded retrieval packet. The original model
shortlist becomes **highlights**; Rust also checks and displays every other prepared passage in
an **Additional retrieved sources** section. A valid empty shortlist still shows retrieved
sources. This prevents selection from hiding evidence already in the packet, including the
previously reproduced publication omission. It adds no model completion or larger input.

Add the flag to the ordinary source-selection command below. Plain output keeps exact quotes,
source identities, physical lines, recorded statuses, partial-body flags and scoped links.
`--json --all-sources` returns experimental version-1 `kind: "source_review"`, with
`highlighted_citations`, `scanned_notes`, `matched_notes`, `omitted_notes` and `sources` (the existing
checked source-passage object containing all prepared sources, once each, in packet order).
The plain view groups highlights first. Neither section is an answer or a verdict of support;
additional sources can be irrelevant. Matching notes outside the packet and partial bodies
remain explicit. Missing retrieval and factual errors in saved memory remain possible.

For model-free inspection of a saved packet, supply a citation-only selection (use
`{"selections":[]}` for no highlights):

```sh
target/debug/akasha --root /path/to/memory --project example \
  librarian-check-sources --packet packet.json --selection selection.json --all-sources
```

Combine **`--all-sources --provenance`** to retain every retrieved passage and inspect its
recorded repository evidence in the same request. This includes sources the model did not
highlight, even an empty shortlist. Plain output includes the existing highlight/additional
sections followed by recorded claim kinds, original code lines and freshness warnings.
The version-1 `source_review` JSON adds `provenance: {association, notes}` only with both flags;
source passages are not duplicated. Metrics use mode `source_review_provenance`.

The explicit provenance flag authorizes the same bounded repository reads as standalone
provenance. All retained passages share its existing file/claim/reference limits and the whole
combined report stays within 65,536 serialized bytes. Overflow refuses without partial stdout;
narrow retrieval with terms or a selected note. Changed files show warnings and withhold stale
code. Ordinary notes can lack recorded evidence. This adds no model completion or semantic verdict.

The core rebuilds the packet and refuses observed drift in any source, including unhighlighted
ones. Invalid/duplicate/unknown IDs and narrower quote selections refuse; complete review always
uses full prepared excerpts. `--all-sources` remains separate from `--answer`.
Omitting it preserves existing output and selection behavior. [Acceptance and demonstration](
../akasha/Research/complete_librarian_source_review.md).

Experimental `--answer` uses the selected main Qwen3.5-9B model in two steps: split the
question into independent parts, then copy exact supporting quotations for each part. The
source-only default remains the normal navigation path: the frozen acceptance still fails database
abstention and explicit proposal recognition. This experiment is not accepted for dependable answers:

```sh
inference_dir="$HOME/.local/share/akasha/local-inference"
node scripts/local-librarian.mjs --root /path/to/memory --project example \
  --answer --question "What is the queue capacity and how are duplicates handled?" \
  --server "$inference_dir/runtimes/llama-b12110-vulkan/llama-server" \
  --model "$inference_dir/models/Qwen3.5-9B-UD-Q4_K_XL.gguf" \
  --model-sha256 6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293 \
  --gpu-layers auto --threads 4
```

This opt-in development mode returns up to eight **exact supporting quotations**. One question-only
completion plans one to four parts; a second independent completion reads the original question,
all parts and bounded sources. Each part must return an explicit sufficiency decision and at most
two single-line quotes up to 600 Unicode characters. Useful known parts survive alongside unknown
parts. Any insufficient part or uncovered detail of the original question marks the complete answer
as insufficient. Model judgments about support and completeness still need evaluation.

The host derives citations from unique exact quote membership across the prepared sources. The
model cannot assign displayed citation identities or factual prose. Altered, ambiguous, duplicated
within a part, control-bearing, heading-only or clipped-ending quotes refuse. Identical support
across question parts is displayed once. Rust's existing `librarian-check` rebuilds the packet,
rejects observed source drift and checks citation/claim bounds. Plain output shows exact quotes,
canonical IDs, source passages, physical lines, recorded status and partial-body flags. Saved
statements may still be wrong or irrelevant; drafts stay drafts and proposals stay proposals.

`Insufficient evidence for a complete answer` permits useful supported quotes; no claims means
none was selected. It does not establish that the repository lacks an answer. Empty retrieval skips
all model access. `--json` keeps the checked-answer shape
`{project, answer: {insufficient_evidence, claims}, evidence}`. Stderr metrics include the protocol,
planning/reading timings, per-part sufficiency/quote counts and closed inference-session metrics.
No persisted schema or onboarding approval changes. Omit `--answer` for source-passage behavior.

Both completions reuse one request-owned worker with independent literal prompts, disabled prompt
cache and full before/after identity checks on each completion. The command's absolute 120-second
deadline includes cancellable core preparation/checking. Ctrl-C cancels, and no successful output
is returned until source revalidation, worker reap and private scratch cleanup finish. A startup
failure now includes a bounded, redacted pre-evidence stderr tail plus phase/exit status, without
automatic retries. Successful startup discards that buffer before sending any project evidence.

The previous generated-prose and line-handle protocols failed support/usefulness and are stopped.
[Current evidence](../akasha/Research/local_answer_support.md) preserves their replacement's
known/unknown/multipart results separately from broad retrieval, desktop and device readiness.
It reaches 10/12 expected outcomes; the two failures remain visible and this tuning line is stopped.

The separate [bounded investigator experiment](local-investigator.md) can request scoped follow-up
reads. This source-selector command retains its existing one-packet behavior and policy.

By default the development runner uses llama.cpp to select useful **source IDs**. Rust supplies the exact
quoted passages, recorded status, physical line ranges and scoped wikilinks. The model cannot
supply an explanation, rewrite a quote or invent a displayed relationship through this interface.
Relevance still needs review: a passage can be real without answering the question. This is a
source-navigation prototype; the desktop panel, final name, model installer, complete autonomous onboarding
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
the strongest initial result keeps that ranking. Later results favor query terms not yet present
in displayed excerpts, identities or status, so repeated matches on one topic do not automatically
consume all six slots. Collection retains bounded candidates per term before choosing the final
six. This remains lexical retrieval; repeated notes can still crowd out a fact about the same
topic. Optional
`--note Projects/example/entities/cache.md` prioritizes that exact canonical note and its outgoing
neighbors in the selected project. Folder paths follow configuration. Keep the original question
or useful terms even when selecting a note; unrelated notes are not evidence of a relationship.

Packets contain at most six 1,000-character excerpts within 16,000 serialized bytes. They carry
status, source hashes, lines and omission/partial flags. Projections, Global and other projects'
bodies are excluded. Shared validation can inspect foreign link-target metadata. Reads are not
an atomic filesystem snapshot. Regenerate disposable packets after retrieval changes; old
packets that no longer match current evidence refuse as stale.

Bodies fitting the 1,000-character source cap stay whole. Longer bodies score contiguous windows
of complete lines within a 640-character context target, favoring distinct query-term coverage
and then the shortest window. This can retain adjacent timeout and capacity facts together.
The excerpt begins at the winning window and includes nearby complete lines. A long winning line
and no-body-match fallback can still use the 1,000-character hard cap. Excerpts preserve source
bytes and physical lines; partial coverage is explicit. Filename weighting and lexical windows
can still select an irrelevant passage or miss synonyms. A term beyond the visible portion of
a clipped long line cannot win passage selection. Regenerate older disposable evidence packets
when selection changes; the wire schema and normal checking commands are unchanged.

At equal lexical strength, filenames containing fewer non-query words rank first. This keeps
a focused component such as `delivery-queue.md` ahead of repeated `delivery-queue-log-01.md`
notes without merging their different values or recorded statuses. Exact note and neighbor
priorities, query diversity and source limits still apply. Filename specificity is a heuristic;
it cannot establish relevance or truth.

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

CPU is default, using up to four threads. With a compatible GPU package, use `--gpu-layers auto`
to fit offload to currently available device memory. `--fit-target-mib N` optionally changes the
target free margin (default 1536 MiB; integer 256–16384, valid only with `auto`). This uses the
pinned runtime's fitter before loading; every context-growth restart fits again. The host still
requires its exact requested context and slot count before sending project evidence. The margin
is a runtime estimate, not a reservation against other programs; startup can still refuse.
There is no automatic retry or model substitution. Integer settings such as `--gpu-layers 24`
explicitly disable fitting and preserve fixed offload. `--threads N` is explicit.

Stderr completion metrics include `resources`: requested layers, fit target and observed
`offloaded_layers` / `total_layers`. Missing observations are `null`, never an invented count.
Automatic mode enables startup trace logging to observe the actual layer count; only bounded
pre-evidence logs are inspected, numeric summaries survive success, and subsequent logs are
drained without capture. Existing bounded/redacted failure diagnostics remain.

The runner applies the model's chat template, disables
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

Add `--provenance` to the ordinary local librarian command to inspect repository evidence
recorded by onboarding. The selector still sees only saved memory. After the worker closes,
Rust attaches each uniquely quoted recorded claim's fact/inference/unknown label, rationale,
original repository path/line range and whole-file hash comparison. Unchanged sources can show
exact code excerpts. Changed or missing sources are explicit and supply no stale code excerpt.
An unrelated edit anywhere in the source file also marks it changed. This is a freshness check;
it does not establish that a saved claim accurately describes the code.

Saved packet/selection inspection needs no model:

```sh
target/debug/akasha --root /path/to/root --project example \
  librarian-check-sources --packet packet.json --selection selection.json --provenance
```

Use `--json` before the command for an experimental `source_provenance` report, with existing
checked passages nested under `sources` and per-citation recorded claims under `notes`. The
flag explicitly adds read-only access to the selected project's registered repository; default
retrieval scope/output stays unchanged. Missing or invalid recorded evidence and unmatched or
ambiguous quote text are explicit. Symlinks, escaping paths and special files are not read.
Inspection is bounded to eight files of at most 256 KiB, 24 claims, 48 references, 2,048 bytes
per code excerpt and 64 KiB serialized output; overflow refuses or marks an unavailable excerpt
as specified in the report. This flag cannot be combined with experimental `--answer`.

`librarian-check --packet FILE --answer FILE` also remains directly usable for saved draft
checking; `--answer` invokes that same checker after host compilation of exact supporting quotations. Valid citations do not prove factual support. The prior
Qwen3 4B free-form trial failed (8/16 expanded cases, 0/8 missing-information cases); source
selection does not overturn that result or establish answerability. The runner now uses the
source-passage checker by default.

External/cloud onboarding remains available alongside the bounded [local draft/review/publication
workflow](local-onboarding.md). Changeable route preference, complete autonomous onboarding and
local-only privacy mode remain later work. Run `node --test scripts/local-librarian.test.mjs`
plus `node --test scripts/librarian-answer.test.mjs scripts/librarian-support.test.mjs` for the onboarding/answer integration and
`cargo test --workspace` for Rust verification. Live model
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
[`--worker-session`](local-investigator.md); the source-only command keeps its original lifetime, while `--answer` owns one
session for its planning/reading pair. Every completion independently renders/tokenizes its full policy/input,
disables prompt caching and checks complete model/binary fingerprints before and after inference.
Session lifetime is absolute (at most 120 seconds), including idle tool reads. Failed/closed
sessions refuse further calls; idle cancellation also reaps the worker. Context can grow but
never shrinks during the request. Runtime identity verification remains deliberately repeated.
