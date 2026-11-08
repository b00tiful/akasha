# Local onboarding draft pilot

The development adapter can draft initial project knowledge with an explicit local GGUF model
and return a Rust-validated proposal and preview. It shares the librarian's owned llama.cpp
transport. Existing external-agent onboarding (`akasha onboard`) remains available.

Build the CLI and initialize a disposable or chosen configured project first. Then select the
repository files the model should see; paths are relative to that project's registered repository.
The adapter does not scan directories or choose additional files.

```sh
cargo build -p akasha-cli -p akasha-mcp
umask 077
node scripts/local-onboarding.mjs \
  --root /path/to/akasha-root --project example \
  --source README.md --source Cargo.toml --source src/lib.rs \
  --server /path/to/llama-b12110/llama-server \
  --model /path/to/Qwen3.5-9B-UD-Q4_K_XL.gguf \
  --model-sha256 YOUR_VERIFIED_64_CHARACTER_SHA256 \
  --gpu-layers 99 --threads 4 --json > draft.json
```

Without `--json`, the command displays the note bodies, projections, proposal ID and preview ID
as bounded plain text. Metrics go to stderr. Failure returns nonzero with no normal stdout.
The draft contains private repository material; keep saved artifacts private.

The model returns a typed content plan: a short name, configured string metadata, Markdown body,
claims and numeric handles for complete source files. Rust supplies exact source paths/hashes,
JSON-compatible YAML frontmatter, configured destination paths and canonical navigation. It
preserves the original projection bytes and appends links for every new note. The model does not
generate storage paths, projection text or line ranges. Absolute filesystem identities are kept
out of the model input.

This command performs no canonical writes. Review every generated metadata value and claim against its supplied
source, including labeled inferences and unknowns. Required string fields do not enforce every
custom field's meaning; the live acceptance needed one reviewed-date correction. Source fingerprints and valid citations do not
establish factual entailment. To publish, pass `draft.json`'s `proposal` field to the existing
onboarding MCP `akasha_onboarding_validate`, request `akasha_onboarding_preview`, review that
summary, and approve `akasha_onboarding_apply` through the connected agent's normal approval
flow. Validation, preview and apply must use one MCP server lifetime; the development command's
preview is not a retained MCP approval. Changed sources or project state require fresh review.

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

`plan.json` has `notes`, each with `note_type`, `name` (1–80 lowercase ASCII letters, digits or
hyphens), `fields` (string-valued metadata), `body` and `evidence`. Claims contain `kind`, `claim`,
`sources: [{"source": 1}]` (one-based packet source handles), and `rationale` for inferences or
unknowns. Unknowns have an empty source list. `schema_version` and `evidence` cannot be replaced
through metadata fields. Compiled Markdown is then checked through the same preview path.

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

```sh
node --test scripts/local-onboarding.test.mjs scripts/local-librarian.test.mjs \
  scripts/inference-session.test.mjs
cargo test -p akasha-core --test onboarding
```

The model-free adapter tests explicitly apply only inside disposable synthetic fixtures and
verify state, unchanged repository files, and librarian retrieval. They do not attest a human
approval interaction or production model quality.
