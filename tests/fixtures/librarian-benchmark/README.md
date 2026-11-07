# Librarian retrieval benchmark

This fictional systems project has 33 canonical notes and 25 predeclared questions. It tests
exact lookup, paraphrases, long-note passage loss, historical/current decisions, recorded links,
hostile instructions in a note, Russian/Chinese text, missing information, metadata-only queries,
lexical distractors and project/Global exclusion. Six high-entropy diagnostic notes stress context
budgeting; they are deliberately artificial. This is broader synthetic coverage, not validation
on a private real project.

`corpus.json` stores the source bodies and labels. A required item names both a canonical identity
and an exact span that must survive excerpting. `allowed` lists relevant navigation sources; it
does not assert that those sources answer the question. Literal/anchor controls and the capacity
stress case are reported separately from unassisted questions. The missing-owner question scores
navigation only: its answer is absent. No result is scored as correct free-form answering.

Use `--corpus tests/fixtures/librarian-benchmark/holdout.json` for the 19-note/12-question release
project, authored and frozen before the ranked-passage correction. Its author knows the original
failure classes; this is new-domain validation, not an external blind benchmark. The default
corpus remains unchanged. `--cli /path/to/frozen-akasha` supports before/after executable comparison.

Build the normal CLI, then run without a model:

```sh
cargo build --offline -p akasha-cli
node scripts/benchmark-local-librarian.mjs --out /tmp/librarian-retrieval-NEW
```

Add `--live` and the same explicit runtime/model arguments used by the development runner:

```sh
node scripts/benchmark-local-librarian.mjs --out /tmp/librarian-live-NEW --live \
  --server /path/to/llama-server --model /path/to/model.gguf \
  --model-sha256 EXPECTED_64_CHARACTER_LOWERCASE_SHA256 --threads 4
```

CPU is default. `--gpu-layers 99` is explicit; `--repeats 3` repeats the unchanged set (1..5).
The harness never downloads or installs a model. Existing output directories refuse. All generated
notes, state and result files stay in the new output directory. It copies the portable resolution
fixture, establishes version-1 synthetic state baselines before measurement, and preserves the
configured event/record/entity classes. The original fixture and corpus remain unchanged.

Each case saves the preparation packet, raw source selection and Rust-checked passages. With no
model, all candidates are selected to expose retrieval's upper bound. Scores distinguish required
span coverage from source precision; a correct file with the wrong excerpt fails span coverage.
An independent byte/hash/line/scope check supplements the actual Rust checker. The harness freezes
the CLI, runner and labels by hash and checks all generated root files for changes.

Metrics include preparation/check wall time and local-worker phases: initial model verification,
runtime verification, worker startup, template/tokenization, context-restart shutdown, completion,
post-generation drift verification and cleanup. `total_ms` includes local setup and cleanup;
the older `elapsed_ms` retains its narrower historical meaning. Worker starts include restarts.
Repeated processes can benefit from filesystem caches; these are not cold-cache measurements.
The harness's CLI-hash checks and evidence-file writes add overhead outside local-worker phases.

Run `node --test scripts/benchmark-local-librarian.test.mjs scripts/local-librarian.test.mjs`.
Low relevance is a recorded research result, not a harness execution failure. Checker, hash or
scope failures stop the run with a nonzero status. Preserve the original failed results before
changing a retrieval algorithm or model profile.
