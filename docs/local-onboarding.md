# Local onboarding draft pilot

The desktop now provides **Review local onboarding** (toolbar **＋**) for captured drafts:

1. Load the configured root/project, choose **Review local onboarding**, then select the saved
   `packet.json` and `plan.json` (or an exported corrected plan). Each file is limited to 16,000 bytes.
2. Choose **Load / preview changes**. Rust checks current project/source identities before showing
   the captured excerpt catalog, metadata and claims. Supported excerpt-plan versions are 3/4/5.
3. Correct claim wording, kind, rationale and source choices. Browse captured excerpts to add a
   missing claim, or remove one within the existing bounds. Metadata and note identities stay fixed.
4. Preview again. Inspect exact proposed note/index/roadmap Markdown, affected actions and current
   citation/sampling/focus gaps. Separate discovery candidate/directory omissions remain in the
   capture's `discovery.json`; this import does not rescan or claim complete coverage.
5. Save or Discard unsaved note edits, paste the full displayed preview ID, and choose
   **Publish reviewed notes**. Rust recompiles and rechecks the exact approval before publication.
   Wait for the result: the modal prevents close/project changes during publication. Failed checks
   never retry or refresh approval automatically; an interrupted writer uses normal recovery.
6. The library reloads after success. Open **Librarian sources** to inspect saved passages,
   optional recorded code and freshly checked full notes. Recorded draft/unreviewed status remains.

Corrections stay in memory until library reload/project change. Closing the review clears approval
but preserves those edits in the same binding. Model generation/discovery and model provisioning
still use the development runner below; no model process is installed by this review feature.


The development adapter can draft initial project knowledge with an explicit local GGUF model
and return a Rust-validated proposal and preview. It shares the librarian's owned llama.cpp
transport. Existing external-agent onboarding (`akasha onboard`) remains available.

Build the CLI and initialize a disposable or chosen configured project first. Then select the
repository files or discovery directories the model should see; paths are relative to that
project's registered repository.

```sh
cargo build -p akasha-cli -p akasha-mcp
umask 077
inference_dir="$HOME/.local/share/akasha/local-inference"
node scripts/local-onboarding.mjs \
  --root /path/to/akasha-root --project example \
  --source README.md --source Cargo.toml --source src/lib.rs \
  --save-inputs /private/path/onboarding-review \
  --server "$inference_dir/runtimes/llama-b12110-vulkan/llama-server" \
  --model "$inference_dir/models/Qwen3.5-9B-UD-Q4_K_XL.gguf" \
  --model-sha256 6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293 \
  --gpu-layers auto --threads 4 --json > draft.json
```

`--gpu-layers auto` fits GPU offload to available memory using the pinned runtime and a
1536 MiB target free margin. Optional `--fit-target-mib N` accepts integers 256–16384 only
with automatic mode. Fixed integer offload remains available with fitting explicitly disabled;
omitting GPU controls retains the CPU default. Context limits, deadlines, independent prompts,
source checks and review requirements are unchanged. Component metrics report requested and
observed offload; the margin does not reserve memory against other applications. See
[runtime controls and lifecycle](local-librarian.md) for details. No automatic retry is added.

For multi-component coverage, replace the repeated `--source` options with
`--components components.json --worker-session`. The JSON file is an array of 1–3 groups:

```json
[
  {"name":"job-queue","sources":["src/queue.rs"],"note_type":"entity"},
  {"name":"capacity-settings","sources":["src/settings.rs"]},
  {"name":"summary-report","sources":["src/report.rs"]}
]
```

Each group gets one independent completion with only its complete selected sources and must
produce exactly one note with that name. Numeric source handles are scoped to each completion;
the host translates them back to the original packet before Rust compilation. Optional `note_type`
constrains each group to that configured type and is checked before combining notes. Groups may share
sources; the union still has at most eight files and the original 16,000-byte packet limit.
All notes are compiled and previewed together, after every completion succeeds. A later failure,
cancellation or observed drift produces no partial proposal or canonical writes.

`--worker-session` explicitly reuses one owned worker within this request, with independent
prompts and per-completion identity checks. The same 120-second overall deadline applies;
large groups can refuse rather than return incomplete coverage. Cold completions remain the
default. Metrics on stderr list each group's selected, cited and uncited sources; citation
coverage measures attribution, not factual support. Names and groups are explicit operator
choices, not a recursive scanner or a completeness guarantee. The sample fixture lives in
`tests/fixtures/local-components/`.

The development machine keeps manually provisioned model weights and complete runtimes in
`~/.local/share/akasha/local-inference/`, outside the Git checkout and temporary storage. The
example uses the selected main model and its pinned hash. `migration-manifest.json` and
`SHA256SUMS` there record local artifact identities. Other machines must supply their own
verified files and paths; the adapter still requires explicit arguments and downloads nothing.

Without `--json`, the command displays the note bodies, projections, proposal ID and preview ID
as bounded plain text. Metrics go to stderr. Failure returns nonzero with no normal stdout.
The draft contains private repository material; keep saved artifacts private.

The model returns a version-3 typed claim plan: a short name, configured string metadata,
claims and numeric source/excerpt handles. It never supplies quote text, source paths or line
numbers. Rust renders the body from those same labeled claims and supplies exact paths/hashes,
source line ranges, configured destinations and canonical navigation. Original projection bytes
are preserved and links appended for every new note. Absolute filesystem identities stay out of
inference input.

Rust divides each complete source into consecutive windows of up to 16 complete lines and
2,048 UTF-8 bytes. The model sees the whole source as numbered excerpts and may cite adjacent
windows together when a claim crosses a boundary. Handles are one-based within each source;
repeated text is unambiguous. Empty sources and files with a physical line over 2,048 bytes
refuse before inference; no lines are silently dropped. Catalog byte offsets and line bounds
belong to Rust and are revalidated with the complete packet before compilation.

The bounded `review` report contains all metadata, every claim, its rationale and fresh exact
source excerpts, plus selected-but-uncited paths. It always sets `requires_review: true`.
Plain text displays excerpts with physical line numbers. Review output is limited to 16,000
serialized bytes; overflow refuses.

Generation performs no canonical writes. Review every generated metadata value and claim against its supplied
source, including labeled inferences and unknowns. Required string fields do not enforce every
custom field's meaning; earlier live acceptance needed reviewed-date corrections. Configured `reviewed` fields are now
grammar-constrained to the supplied draft date; a generated date still does not attest human review. Source fingerprints and valid citations do not
establish factual entailment. The direct local workflow below uses the same shared-core
transaction. Alternatively, pass `draft.json`'s `proposal` field to the existing
onboarding MCP `akasha_onboarding_validate`, request `akasha_onboarding_preview`, review that
summary, and approve `akasha_onboarding_apply` through the connected agent's normal approval
flow. Validation, preview and apply must use one MCP server lifetime; the development command's
preview is not a retained MCP approval. Changed sources or project state require fresh review.

After approved publication, the [local librarian](local-librarian.md) can use optional
`--provenance` to trace uniquely quoted claims back to these saved repository references.
It preserves draft status and recorded evidence kinds, displays exact code only while whole-file
hashes match, and warns after source changes or deletion. It performs no automatic note refresh
or review approval; semantic corrections still use the explicit review/application workflow.

Model-free inspection and checking are also available:

```sh
target/debug/akasha --root /path/to/akasha-root --project example --json \
  local-onboarding-prepare --source README.md --source src/lib.rs > packet.json
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-preview --packet packet.json --proposal proposal.json
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-compile --packet packet.json --plan plan.json
```

`proposal.json` is the existing onboarding proposal shape: `notes` (each with `note_type`, `path`,
and complete Markdown `source`), `index`, and `roadmap`. Types, required fields, folders and
templates come from the selected configuration. Local drafts additionally require both
projections to link every proposed note and reject broken projection links.

`plan.json` has `schema_version: 3` and `notes`, each with `note_type`, `name` (1–80 lowercase
ASCII letters, digits or hyphens), `fields` (string-valued metadata) and `evidence`. Claims contain
`kind`, `claim`, `sources: [{"source": 1, "excerpt": 2}]` (one-based source/excerpt handles), and
`rationale` for inferences or unknowns. Unknowns have no sources. Metadata cannot replace
`schema_version` or `evidence`. Quote/path/range fields in version-3 references refuse.

Fresh preparation packets are version 2 and include source catalogs; old packets must be
regenerated. Draft output remains version 2. Version-2 exact-quote plans remain supported with
fresh packets for saved correction workflows: references use `{"source": 1, "quote": "exact unique
source text"}`, with a nonblank quote of at most 1,024 characters occurring uniquely, including
overlapping matches. Legacy arbitrary-body plans refuse. General external/MCP proposal and
persistent evidence formats remain unchanged.

Raw local preview also requires claim-rendered bodies and positive bounded source ranges;
editing only a body refuses. Correct the plan's claims/metadata or excerpt selection, recompile,
and review the fresh proposal/preview IDs. Source integrity does not establish semantic
entailment: a false paraphrase of a real comparison can still pass and needs review.

The pilot accepts 1–8 complete regular UTF-8 source files, at most 8 KiB each, and a 16,000-byte
serialized packet including templates, inventory and projections. Oversized inputs refuse;
nothing is silently truncated. Source paths must be normalized and cannot contain symlinks.
The generated batch is limited to 1–3 notes and 16,000 output bytes. The shared runtime's 1,024
output-token limit favors one compact note; incomplete generations refuse. Large templates,
inventories, projections or drafts may exceed this pilot's bounds.

Rust rebuilds the complete packet before and after preview. Altered/stale packets refuse. Every
persistent evidence reference must match a supplied path and whole-file hash before the general
onboarding preview may read it. The existing schema, line-bound, canonical-link, conflict and
project-state checks then run. These are observed-drift checks, not an atomic filesystem snapshot.

Runtime options match the [librarian transport](local-librarian.md): explicit pinned b12110
binary and GGUF/hash, CPU by default (`--gpu-layers 0`), up to four threads by default, `--max-context`
8,192 by default (4,096/8,192/16,384/32,768), authenticated numeric loopback, literal payload
tokens, a 120-second deadline and owned-worker teardown on completion/error/cancellation.
Ctrl-C cancels. No model download, cloud fallback, model tools or persistent worker is added.

Full autonomous repository onboarding, route preferences, model installation/distribution,
desktop integration and representative quality/device acceptance remain future work.

## Automatic initial-note planning

Add `--auto-plan --worker-session` to a `--discover` or repeated `--source` invocation to have
Rust choose unique note identities before inference. No component-plan JSON is needed:

```sh
node scripts/local-onboarding.mjs \
  --root /path/to/akasha-root --project example \
  --discover . --auto-plan --worker-session \
  --save-inputs /private/path/new-review \
  --server "$inference_dir/runtimes/llama-b12110-vulkan/llama-server" \
  --model "$inference_dir/models/Qwen3.5-9B-UD-Q4_K_XL.gguf" \
  --model-sha256 6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293 \
  --gpu-layers 99 --threads 4 --json > draft.json
```

Rust partitions selected files into up to three groups: context (overview, manifests, docs and
workflows), implementation, and verification (tests). Every selected file belongs to one group.
These filename roles do not establish components or complete repository coverage. Planning needs
exactly one configured entity-class type and recognized source roles; otherwise use explicit
`--components` with reviewed names/types. `--auto-plan` and `--components` cannot be combined.
Sampling remains an independent opt-in.

Inspect the same plan without inference or writes:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-plan --packet packet.json
```

Plain output lists slot, role, configured type, project-relative destination and original-packet
source handles. `--json` returns `{schema_version: 2, slots}` with these fields, host-owned
`fields` and any `unresolved_fields`. Filenames use
`onboarding-ROLE-HASH`, with a stable 128-bit path-set hash; content edits retain the identity.
Source-set/type changes can change names. Existing targets are never silently renamed or
suffixed: differing contents refuse at compilation. Review the exact proposed destinations.

One independent completion per slot supplies a version-5 plan: `notes` contain only `slot`
and `evidence`. Rust owns all metadata: `entity` is the unique source-group stem, `kind` is
`source-group`, `status` is `draft` and `reviewed` is `unreviewed`. The last value explicitly
records that no review date has been established. Required `project`/`type` values come from
configuration. Unknown required fields refuse before inference; use explicit groups or a
reviewed version-4 correction for custom metadata. The model cannot supply `fields`, names,
types or review dates, even as empty/null values. Its prompt contains role/scoped evidence and
focus intent, without metadata schemas, templates, inventory or opaque storage stems. The adapter
remaps local source handles to the original packet. Rust rebuilds the plan, requires all slots exactly once
and rejects citations outside a slot's group. All notes still need metadata/claim review and
full-ID application. Worker reuse retains the same overall deadline; a later failure returns no
partial proposal.

`--save-inputs` also preserves `identity-plan.json` privately before inference. Correct a copy of
saved version-5 `plan.json` using its original slot IDs and original-packet source handles, then
run ordinary `local-onboarding-compile` against `packet.json`. For explicit metadata corrections,
change the plan version to 4 and add `fields` to every note, initially copied from its
`identity-plan.json` slot. Edit those fields only after source review; `reviewed` should name an
actual verification date only when that review occurred. Names/types and group membership
still come from the packet. Version-4 reviewed corrections, existing version-2/3 plans and
explicit component groups retain their behavior. Draft/proposal/publication formats remain unchanged.

## Bounded directory discovery

Replace `--source`/`--components` with one or more `--discover DIRECTORY` arguments to let Rust
select complete files within explicit directories. For a small repository use `--discover .`;
for a larger one select narrow, non-overlapping directories, such as `--discover src/parser
--discover tests/parser`. Inspect selection without inference or writes first:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-discover --directory .
```

`--json` returns `{schema_version: 1, packet, coverage}`. `packet` is the existing version-2
preparation input and can be saved for ordinary compilation/preview/application. `coverage`
lists selected paths/roles, every omitted candidate/reason, excluded-entry count and up to
32 excluded examples. Additive `coverage.implementation_groups` rows contain `directory`,
`candidates` and `selected` counts, including zero-selected groups. Excluded directories count as one entry; their descendants are not scanned
or counted. The draft adapter includes coverage in stderr metrics and saves `discovery.json`
alongside private inputs. Plain generation shows a coverage summary; successful draft JSON
keeps its existing schema. Inference receives only selected sources and their roles.

Selection covers overview, manifest, implementation, tests, documentation and workflow roles
once, then prioritizes unrepresented implementation groups before repeated context files.
Groups use the nearest nested manifest directory found inside the authorized scan, falling back
to the implementation file's parent; a root manifest does not collapse all source directories.
Within implementation selection, literal-focus relevance stays ahead of group diversity, followed
by parent diversity and shallow lexical ordering. A selected file represents a filename group;
it does not establish complete component understanding. Plain discovery, draft summaries and
offline review expose groups with no selected implementation. Narrow scopes or prepare a
separate batch for missing groups. Files must fit the existing eight-file/8-KiB-per-file/
16,000-byte-packet bounds in full; larger files, invalid text, empty/overlong-line sources and
budget omissions are explicit. A later smaller candidate can still fit after an omission.
Every selected byte/hash/catalog is rebuilt through ordinary preparation before return and
checked again by compilation. Unselected content is not a source for generated claims.

The scan allows common code/Markdown extensions and named manifests (`Cargo.toml`, `package.json`,
`pyproject.toml`, `go.mod`, `CMakeLists.txt`, `Makefile`, `rust-toolchain.toml`). It excludes nested
hidden names, dependency/build directories, names containing `secret`, `credential` or
`private-key`, all symlinks and nonregular entries. These filename rules are not secret detection.
**Discovery does not interpret `.gitignore`.** Explicit directory scope authorizes inspection of
eligible files there, including ignored files; use narrow directories or explicit sources when
that distinction matters. Directory arguments themselves are explicit scope choices, so an
explicit hidden directory is allowed; hidden descendants remain excluded.
GitHub workflow `.yml`/`.yaml` files are recognized when `.github/workflows` is explicitly
scoped (for example, `--directory packages --directory .github/workflows`); a scan of `.` still
excludes `.github`. Explicit source preparation also recognizes `.gitlab-ci.yml` as a workflow.
Other YAML files remain ineligible for directory discovery. No workflow content is executed.

More than 2,048 inspected entries, 128 candidate files, eight levels below a requested directory,
or a discovered path over 1,024 UTF-8 bytes refuses the entire scan before inference. Absolute,
traversing, repeated, overlapping and symlinked scopes refuse. Filesystem errors refuse; there is
no partially scanned success. Source/projection drift also refuses. This is bounded source
selection, not a whole-repository completeness claim. Review generated meaning and metadata
through the same workflow above; discovery never approves or publishes notes.

```sh
node --test scripts/local-onboarding.test.mjs scripts/local-librarian.test.mjs \
  scripts/inference-session.test.mjs
cargo test -p akasha-core --test onboarding
```

The model-free adapter tests explicitly apply only inside disposable synthetic fixtures and
verify state, unchanged repository files, and librarian retrieval. They do not attest a human
approval interaction or production model quality.

To retain inputs for source review or correction, pass `--save-inputs /path/to/new-review-dir`.
Its parent must exist; the directory must be new and outside both the configured memory root
and registered repository, including symlinked parent aliases. The adapter creates it with
mode `0700`, saving the core `packet.json`, each raw `completion-N.json`, and the combined
`plan.json` with mode `0600` and no overwrite. Successful compilation also saves `draft.json`
(proposal, review and preview), `proposal.json` (the exact applicable proposal) and offline
`review.html`, in both plain
and JSON generation modes. Failed compilation leaves the original inputs
available; later-component failure may leave only the completed raw replies. No saved input is
an approval. Keep originals, edit a copy of `plan.json`, then use `local-onboarding-compile`
with the saved packet to obtain a fresh review. If source/configuration/project bytes changed,
prepare and review again. This opt-in writes private review artifacts; it never publishes notes.

## Focused source targeting

Use repeated literal `--focus` terms to onboard a particular subsystem without listing its files:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-discover --directory . --sample-large-files \
  --focus lease --focus deadline
```

The same flags work with `local-onboarding-prepare --source ...` and the development adapter:

```sh
node scripts/local-onboarding.mjs --root /path/to/akasha-root --project example \
  --discover . --focus lease --focus deadline --sample-large-files --auto-plan \
  --worker-session --save-inputs /private/path/lease-review \
  --server /path/to/llama-server --model /path/to/main.gguf \
  --model-sha256 THE_VERIFIED_SHA256 --gpu-layers 99
```

Terms are case-insensitive literal substrings, including paths; they are not regular expressions
or a semantic search. Supply 1..8 raw terms of at most 80 UTF-8 bytes each, without control
characters. Trimming, Unicode lowercasing and deduplication produce stable stored terms; the
normalized byte bound also applies. Changing term order/case does not change selection.

Within each existing filename role, discovery prefers sources matching more distinct terms.
Repeated occurrences add no weight. Directory diversity breaks relevance ties for implementation
files. Unmatched README/manifests remain eligible for orientation; other unmatched candidates are
reported as `focus_no_match`. A request fails if no matching source fits. Scope, exclusions,
inventory limits, packet/file limits and role balancing still apply. Focus does not widen scopes
or follow dependencies. A directory name matching a term also makes its descendants path matches;
choose specific terms or narrower scopes when that is too broad.

Focused discovery inspects **all eligible size-bounded candidate contents**, including files that
will not enter the model packet. At most 128 candidates of 8 KiB (or 256 KiB with sampling) are
read, plus bounded rebuild reads of selected sources. Excluded names/directories, symlinks and
oversized files are not opened. Coverage records successfully inspected source bytes, each
selected file's matching terms and every omitted candidate/reason. Invalid source bytes are not
included in that successful-byte counter. Only selected excerpts go to inference.

With large-file sampling, Rust retains the opening complete-line window and then the two windows
matching the most distinct terms, with original-order ties. Any remaining slots use middle/end
windows. No matches reproduce positional sampling. Each window remains at most 16 lines/2,048
bytes; partial coverage and omitted original line intervals remain explicit. Matching words can
occur in comments/examples; neither ranking nor an exact excerpt proves a claim or complete
function coverage. Without sampling, selected sources remain complete files under 8 KiB.

Focused input packets use experimental version 4 and store `{terms, sample_large_files}` in
`focus`. Core planning/compilation/review rebuild from these exact controls, checking all selected
whole-file fingerprints, including omitted lines. Version-2/3 packets remain supported and
serialize as before; the plan's version-4 slot format is a separate schema. Persistent notes,
MCP and approved-write formats are unchanged. Capture preserves packet/coverage/plan/proposal;
metrics expose `source_focus`. Review lists `uncited_focus_excerpts` (paths, original line ranges
and matching terms) for text-matching windows with no overlapping claim citation, even when
another window in that file is cited. Plain output highlights these gaps. A partial/incorrect
citation can still hide missing meaning; absence of gaps is not semantic approval.
Review metadata, claims and gaps before the ordinary explicit
full-ID publication. Unselected-file edits and new candidates do not invalidate a saved packet;
rediscover when the repository coverage needs refreshing.

## Large-file sampling

Add `--sample-large-files` to the development adapter or either core input command when useful
implementation files exceed 8 KiB. It works with `--source`, `--components` and `--discover`:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-discover --directory src --sample-large-files
target/debug/akasha --root /path/to/akasha-root --project example --json \
  local-onboarding-prepare --source src/parser.rs --sample-large-files
```

Sampling reads regular UTF-8 files up to 256 KiB. Files over 8 KiB supply their first, middle
and last catalog excerpts, each at most 16 complete lines/2,048 bytes. Smaller files retain
their complete text. The original eight-file/16,000-byte packet bounds still apply; selecting
many windows can still exceed the packet budget. A line over 2,048 bytes anywhere in a larger
file refuses explicit preparation and is an explicit discovery omission. No line is clipped.

The opt-in version-3 packet records whole-file SHA-256 and total line count, plus `sampling`
with total bytes and every omitted line interval. Its `text` packs selected windows only;
excerpt byte offsets address that packed text, while line ranges always name the original
file. The model receives separate numbered excerpts and partial-coverage metadata. Plain
preparation/discovery and final source review show omitted ranges; draft JSON adds
`review.sampled_sources`. Stderr metrics include `source_sampling`, and private packet/draft
artifacts preserve the same coverage. Inspect it alongside claims and metadata before apply.

Positional sampling may split a function or omit important behavior. Omitted lines are unknown;
absence from a window cannot establish absence from the file. Evidence references use excerpt
handles; quote plans are accepted only for complete sources. Raw proposals cannot cite a gap
or bridge nonadjacent windows. Edits anywhere in the original file, including omitted lines,
invalidate a saved packet. Reprepare using the same opt-in when refreshing inputs after apply.
Default version-2 complete-file packets, persisted evidence and external MCP remain compatible.

## Offline claim and source review

`--save-inputs /private/path/new-review` now also creates **`review.html`** after successful Rust
compilation. Open that file in a browser; it needs no server or network connection. Each claim
appears beside its exact source excerpts with physical line numbers. You can edit claim wording,
change fact/inference/unknown type and rationale, and choose different excerpt handles. Core-planned
source groups constrain the available choices. Metadata stays visible and read-only.

Use **Browse all captured source excerpts for this note** to inspect windows the model did not
cite. **Add claim citing this excerpt** creates a blank fact with that exact handle selected;
write the supported behavior in its claim field. **Add missing claim** starts without a source
so you can choose evidence or record an explicit unknown with a rationale. The page never
generates claim wording. You can **Remove claim** and **Undo removal** (the latest removal in
each note). The existing 1–4 claims per note still applies: at four claims, remove one before
adding another; the final claim cannot be removed. Other edits survive additions/removals.
New claims use the same type/rationale/source editor as original claims, and Rust compilation
remains authoritative for all content, source, size and schema checks.

The original core coverage report highlights uncited sources, sampled omissions and uncited focus
windows. It remains labeled as original while you edit: compile your export to obtain fresh checks.
Click **Export correction plan** to download `reviewed-plan.json`. Changes stay in the page until
exported; closing with unsaved corrections prompts you to keep them. The page preserves note
identities, fields and recorded draft/unreviewed status. Browsing only exposes the captured
packet; use a separate onboarding batch to inspect sources outside it. [Omitted-claim repair
demonstration](../akasha/Research/evidence/2026-11-10-onboarding-claim-repair/demo.md).

Turn the downloaded correction plan into a complete fresh review bundle with one model-free
command. Choose a new output directory outside the memory root and repository:

```sh
node scripts/local-onboarding-review.mjs prepare \
  --root /path/to/memory --project example \
  --capture /private/path/new-review --plan /private/path/reviewed-plan.json \
  --output /private/path/corrected-review
```

This snapshots bounded inputs, recompiles through Rust, refreshes the plain source/metadata
review and creates `packet.json`, `plan.json`, `draft.json`, `proposal.json`, `review.txt` and
`review.html`. Core-planned source groups and captured discovery gaps are retained when present.
The claim/citation review is fresh; discovery coverage still describes the original scan, not
new files added since then. The original capture is preserved. The new directory is create-only
and owner-only, as are its files. Preparation performs no inference or canonical writes.
Open the new `review.html` or read `review.txt`, then explicitly approve its full preview ID:

```sh
node scripts/local-onboarding-review.mjs apply \
  --root /path/to/memory --project example \
  --bundle /private/path/corrected-review \
  --approved-preview-id sha256:THE_FULL_REVIEWED_IDENTIFIER
```

Application snapshots the bundle's packet/proposal into private scratch and delegates to the
existing Rust approved writer. Changed claims, sources, relevant configuration or project state
refuse; no preview is approved implicitly and there is no automatic refresh/retry. `--json` on
prepare returns the bundle path, compiled draft and plain review; on apply it returns the existing
core batch result. Failures have nonzero status, stderr diagnostics and no normal stdout. SIGINT
and SIGTERM cancel subprocesses and clean scratch. If a signal arrives after publication starts,
inspect state and follow the recovery guidance below; cancellation cannot undo a committed batch.
This adapter supports the excerpt-based version-3/4/5 plans used by the offline editor.

Alternatively, compile the downloaded plan directly using the same root/project and saved packet:

```sh
target/debug/akasha --root /path/to/memory --project example --json \
  local-onboarding-compile --packet /private/path/new-review/packet.json \
  --plan /private/path/reviewed-plan.json > /private/path/reviewed-draft.json
```

Use the resulting proposal and **new** full preview ID with the ordinary reviewed-application
flow below. Exporting does not publish anything, update review dates or authorize an old preview
for changed content. Changed source/project/configuration state still requires fresh preparation.
The original private artifacts refuse overwrite and cannot be saved into the configured memory
root or repository. This is development review tooling, separate from the installed desktop UI.

## Direct local review and application

After approved publication, open the same configured root/project in the desktop and choose
**Librarian sources** from the toolbar or inspector. The [desktop source panel](local-librarian.md)
finds saved passages, opens freshly checked full notes and optionally traces recorded claims to
code. It preserves draft/unreviewed status. Review/correction and publication still use the
explicit handoff below; browsing sources does not approve or publish a plan.

After generation, review all metadata, claims, exact source excerpts, uncited sources and both
projection outputs. This model-free command displays them again without running inference:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-preview --packet /private/path/onboarding-review/packet.json \
  --proposal /private/path/onboarding-review/proposal.json
```

Stopping here cancels publication: the saved artifacts grant no write authority. To approve,
supply the **full `Preview: sha256:...` identifier** from the reviewed output:

```sh
target/debug/akasha --root /path/to/akasha-root --project example \
  local-onboarding-apply --packet /private/path/onboarding-review/packet.json \
  --proposal /private/path/onboarding-review/proposal.json \
  --approved-preview-id sha256:THE_FULL_REVIEWED_IDENTIFIER
```

This explicit command is the approval action; it has no model or automatic confirmation.
It rebuilds local inputs, repeats proposal/source review checks and checks the exact approved
preview before entering the shared writer. The writer acquires the project lock, revalidates
the proposal and snapshot, and uses the existing recoverable batch transaction. It creates
canonical notes, updates the index and roadmap, and publishes project state. Existing differing
notes refuse; no force/overwrite flag exists. Plain output lists created/unchanged notes,
updated projections and state; `--json` returns the shared-core batch result. Errors have nonzero
exit status, diagnostics on stderr and no normal stdout. A validation error may precede a stale
conflict when a concurrent edit leaves the project invalid.

For corrections, the review-bundle commands above handle proposal extraction and fresh preview.
For a manual CLI-only handoff, preserve the raw inputs, edit a copy of `plan.json`, then compile it with
`local-onboarding-compile --packet ... --plan reviewed-plan.json --json > reviewed-draft.json`.
Save its `proposal` object as a separate private `reviewed-proposal.json` and preview that file.
Use that proposal and its new full preview ID for application. The earlier ID cannot approve
changed content; editing an identifier is not a substitute for reviewing again.

After success or an uncertain/interrupted apply, inspect current state and run `validate`.
After a successful reviewed publication, the [local librarian's optional `--answer` mode](local-librarian.md)
can extract exact supporting quotations by question part with host-derived citations and status. Its answer-utility gate remains
unaccepted; source-only navigation remains the default. Use the same explicit
root/project and main-model options; questions use saved canonical knowledge, not a fresh scan
of the repository. Selection and uncertainty still need review. A published draft retains its
draft status and does not become automatically verified knowledge.

If a pending note journal leaves the project invalid, use the existing desktop load/retry or
F5 in a clean TUI session for recovery, following [the recovery runbook](recovery.md).
Local preview/apply first validates its packet and cannot recover an invalid partial project;
the named CLI still has no standalone note-journal recovery command. Preserve unexpected bytes
and the journal rather than deleting them or retrying inference.
The old packet/preview is normally stale. Prepare a fresh packet with the same `--source` paths
and preview the **exact saved proposal** against it. Exact already-published notes/projections
are shown as unchanged; approving that fresh preview is a no-op. Do not regenerate a model draft
just to retry publication. Human changes, new source bytes or conflicting notes require a fresh
plan and source review. Interruption during publication follows the existing core recovery
contract; a signal after commit does not guarantee that the batch was cancelled.
