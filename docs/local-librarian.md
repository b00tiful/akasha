# Local librarian pilot

This experimental command-line slice retrieves saved project notes and asks an installed local
model for a cited answer. The desktop chat panel, final name, model installer, local onboarding
and project-wide privacy mode are later work. It never edits canonical notes. Core/CLI usage
does not require Node or Ollama; those are requirements of the development runner only.

Verified with Node v22.22.2, Ollama 0.20.5 and the `qwen3:1.7b` Q4_K_M artifact. This is a
compatibility candidate, not the final model choice. In two synthetic GPU runs it answered a
recorded status correctly and abstained on an unrecorded assignee. The server reported about
4.9 GiB allocation with the pilot's context setting; performance on typical or CPU-only devices
is not established. Current Qwen3.5:2b requires a newer runtime than the one used for this pilot.

Build with `cargo build --offline -p akasha-cli`. Inspect retrieval without any model:

```sh
target/debug/akasha --root /path/to/memory --project example \
  librarian-prepare "What caused the cache failure?" --term "cache"
```

Add `--json` for the version-1 packet. Repeat `--term` up to eight times to guide literal matching,
or omit it for simple alphanumeric query-word overlap. This is not semantic search. Optional
`--note Projects/example/entities/cache.md` includes that exact canonical note and prioritizes
its outgoing neighbors only when they belong to the same project. Paths follow root configuration;
the example spelling is not a hardcoded requirement.

The packet carries at most six excerpts of 1,000 Unicode characters each and must fit 16,000
serialized bytes. It reports scanned/matched/omitted notes, exact source fingerprints, line ranges,
status and partial-excerpt flags. Line ranges can end part-way through a line. It is not an atomic
filesystem snapshot or a complete repository analysis. Projections, Global notes and other
projects are excluded from model evidence. Shared validation may inspect linked target metadata
outside the project; this is not yet a strict no-cross-project-filesystem-access privacy mode.

For a local run, use a trusted local Ollama worker with cloud features disabled. For example,
in a separate terminal, with an explicit cache directory and unused loopback port:

```sh
OLLAMA_NO_CLOUD=1 OLLAMA_HOST=127.0.0.1:11435 \
  OLLAMA_MODELS=/path/to/local-model-cache ollama serve
```

Install a compatible model explicitly in that worker (`OLLAMA_HOST=127.0.0.1:11435 ollama pull
qwen3:1.7b` for the initial compatibility candidate), then:

```sh
node scripts/local-librarian.mjs --root /path/to/memory --project example \
  --model qwen3:1.7b --port 11435 \
  --question "What caused the cache failure?" --term cache
```

The runner calls only numeric loopback, rejects redirects and remote-model metadata, requires
already-installed weights and never pulls a model automatically. It sends one bounded request
with no model tools, thinking disabled, 32,768 context and 1,024 output-token limits. No-match
retrieval abstains without contacting Ollama. Ctrl-C aborts the request; completion/cancellation
attempts a model unload. Stop the owned worker if unloading is not confirmed. The independently
started server and cached weights remain until the operator stops/removes them. Requests time out;
errors never produce a successful answer. A trusted local server is a prerequisite: these checks
are not OS egress isolation, and a compromised or concurrently reconfigured worker is outside the
pilot's privacy guarantee.

The model returns `insufficient_evidence` and an array of `{text, citations}` claims. Rust rebuilds
the evidence packet and refuses observed drift, unknown fields, uncited/duplicate/out-of-range
references, controls, oversized text and invalid JSON before displaying anything. These checks
prove shape and citation membership, **not that the cited passage entails the claim**. Answers
remain drafts for review. No automatic retry hides a refusal.

`librarian-check --packet FILE --answer FILE` exposes the same check independently. Plain text is
default; `--json` returns typed checked evidence and claims. The runner uses owner-only temporary
files and removes them on normal completion/error; abrupt process death can leave sensitive scratch
files. It does not keep chat history or publish answers. Local logs, scratch, swap and backups
remain sensitive. The future privacy mode will need an explicit lifecycle/egress/export contract.

Onboarding direction: users will be able to choose external/cloud-assisted or local onboarding and
change their preference for later runs. This does not overwrite existing notes, silently restart
jobs or undo prior cloud disclosure. The current `onboard` command still emits the existing
external-agent handoff; the local onboarding selector is not implemented by this pilot.

Run core/CLI verification with `cargo test --workspace`; the local transport tests use
`node --test scripts/local-librarian.test.mjs` and bind temporary loopback servers. A real-model
smoke test is separate from both and does not establish general answer quality or device support.
