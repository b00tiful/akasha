# Synthetic memory-system evidence

This corpus is intentionally system-neutral. Each adapter must ingest the same facts without
adding facts from the source repository.

## Historical events

- 2026-06-01 — `TASK-LANTERN-17` was the next task: prototype a SQLite-backed project index.
- 2026-06-02 — `ARCH-SQLITE-BLUE` was accepted because faster search was expected.
- 2026-06-03 — `FAIL-BRAMBLE-LOCK` failed: a persistent sentinel file survived process death and
  blocked the next writer. Do not repeat that locking design.
- 2026-06-04 — The SQLite decision was superseded after exact-source correction and portable
  review proved more important than search speed.

## Current truth

- `TASK-ORCHID-42` is the one immediate next task: add a checked note-replacement acceptance test.
- `ARCH-MARKDOWN-GREEN` is the accepted authority model: Markdown is canonical and indexes are
  rebuildable projections.
- `WHY-AUDITABLE-REVERSAL` is the rationale: a human must be able to inspect, correct, and reverse
  current truth without depending on an opaque database.
- `POLICY-SUPERSEDE-KEEP-HISTORY` governs corrections: replace maintained current truth, mark the
  older decision superseded, and retain the historical event.

## Noise

- The synthetic project uses a green status icon.
- The example repository contains a `src/` directory.
- No production credentials or personal data are present.
