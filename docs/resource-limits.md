# Source and YAML resource limits

Ordinary canonical note reads (project and Global) and configured project index/roadmap reads
accept at most **16,777,216 bytes (16 MiB) per file**, including frontmatter and exact line endings.
The shared core reads at most that allowance plus one sentinel byte, even if the file grows.
Excess fails with exit **4**, concise stderr and empty stdout before UTF-8 or YAML decoding.
Successful sources remain exact; split or reduce an excessive document explicitly before retrying.

The bound applies to validation, library/search, graph/document loads, context and ordinary
mutation source reads. Creation, replacement and lifecycle previews/writes check prospective
notes and projections before journal publication. Existing onboarding proposal limits still
apply and are generally smaller. Checked replacement comparisons read no more than the exact
expected image length plus one byte; growth refuses as a conflict.

Historical recovery keeps unrestricted journal/image reads and recovery-only project validation
for all existing shared-journal versions. It retains exact schema/link/integrity and foreign-byte
checks. A recovered oversized project can finish recovery while subsequent ordinary loading
still refuses it. No journal migration or automatic rewriting occurs. Recovery therefore retains
its previous memory exposure; see [the recovery runbook](recovery.md).

For a preexisting oversized project, back up the complete root first. Use a previously trusted
compatible version's checked operations to reduce mutable notes and accept matching projections,
or restore a compatible complete backup. Immutable events retain their exact historical bytes;
continue using the compatible version until an explicit migration is available. Editing content
or state hashes to silence validation is not a migration. The new limit has no override switch.

This is a per-source input limit, not a total process-memory or cumulative-I/O bound. Allocation
capacity, parser queues, many fitting context bodies, paths/counts, editor drafts, templates,
TOML, state/journal/other JSON and total archive size retain separate limits or open work.

Run source-boundary, no-write and historical recovery regressions with:

```sh
cargo test --offline --workspace --all-features source_limit
```

The shared Rust core applies these limits to canonical note frontmatter and the project
registry in every interface. It rejects excessive metadata without truncating or rewriting it.

| Resource | Note frontmatter | Project registry |
|---|---:|---:|
| YAML input bytes | 65,536 (64 KiB) | 1,048,576 (1 MiB) |
| Nodes, including alias-expanded values | 8,192 | 65,536 |
| Parser events | 16,392 | 131,080 |
| Total scalar bytes / total comment bytes, each | 65,536 | 1,048,576 |
| Container depth | 32 | 32 |
| YAML documents | 1 | 1 |
| Anchors / alias references, each | 128 | 128 |
| Replayed alias events | 16,384 | 131,072 |
| Alias replay stack depth | 16 | 16 |
| Expansions of one anchor | 32 | 32 |

The parser's existing alias-to-anchor ratio heuristic also applies. Duplicate keys, merge
keys and explicit tags remain invalid. Ordinary aliases within all budgets remain supported.

Frontmatter byte counting excludes both delimiter lines and includes the YAML's exact LF/CRLF
bytes. Delimiter searches stop within the header allowance, including when the closing delimiter
is missing or a line is enormous. Optional projection headers use that same framing limit;
their YAML is still skipped without imposing canonical metadata schema.

Registry file reads stop after the allowance plus one sentinel byte, including when a file grows
during reading. An excessive registry fails validation before UTF-8 decoding of the sentinel.

Named CLI failures return exit code **4**, concise stderr diagnostics and empty stdout in both
plain and JSON modes. The terminal retains its existing failed-operation and draft safeguards.
Split excessive metadata into smaller notes or simplify its structure, then retry.

Markdown bodies stay byte-exact and are outside these metadata limits, while ordinary complete
sources obey the 16 MiB limit above. YAML limits do not bound total archive size, TOML, journals,
onboarding JSON or wall time. Search limits bound returned hits, while validation and matching still scan the archive.
The [inline-code lookahead](archive-runtime.md) avoids repeated suffix searches but may queue
offsets proportional to examined backtick runs; it does not add a body or archive memory cap.

Integrity checking retains original-read fingerprints instead of every canonical body; the
[archive guide](archive-runtime.md) explains that memory reduction and its verification. Whole-note
reads obey the separate source allowance; other retained images, including context candidates,
remain outside these YAML limits.

[Context assembly](context.md) retains exact length/path metadata without individually non-fitting
bodies and measures budget selection without constructing rejected Markdown. It keeps only the
latest handoff body, while breadcrumbs keep metadata without bodies. Many individually fitting
context bodies can still accumulate, especially with expanded output budgets; per-source reads,
parser images and path/count metadata remain separate from the output bound.

To measure actual projection, all-library search and the core work used by background checks on
100, 1,000 and 10,000 generated Unicode notes:

```sh
cargo test --offline -p akasha-core --test library synthetic_archive_runtime \
  -- --ignored --nocapture
```

Each JSON row reports microseconds, archive size, exact result counts and source preservation.
Separate raw-read, frontmatter-parse and SHA-256 phase timings run after the operation timers.
Use `--release` to measure optimized code; [archive reads](archive-runtime.md) explains exact-byte
line-boundary reuse and the measurement boundaries.
The fixture splits added notes equally between one project and Global and updates integrity
state before measuring. The background measurement includes pending-journal inspection and a
fresh projection, followed by equality comparison; it excludes terminal scheduling, watched
editor reads and drawing. Fixture creation and no-write snapshots are outside the timers.
The filesystem is already warm from setup; these are single synthetic observations, not cold
disk or universal latency guarantees. See also [terminal runtime](terminal-runtime.md).
