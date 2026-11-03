# Bounded project context

`akasha context` validates the selected project and returns a Markdown orientation bundle,
bounded to 16,000 Unicode characters by default. Request a different per-call budget explicitly:

```sh
akasha context --max-chars 32000
akasha context --max-chars 4000
akasha --json context --max-chars 32000
```

`N` is a positive decimal integer up to 4,294,967,295. The budget includes identity, headings,
source paths, data fences and omission messages. It counts Unicode scalar values, so UTF-8 byte
length can be larger. JSON exposes `max_chars`, `rendered_chars`, selected `entries` and an exact
`omissions` ledger; JSON serialization overhead is outside the Markdown budget.

If the complete bundle fits, every candidate is returned. Otherwise selection keeps whole entries
in priority order: open tasks, open problems, roadmap, entity index, latest handoff and recent
events. An entry that does not fit is skipped, and later entries are still considered. No note
is cut. A budget too small for identity and omission framing fails on stderr with exit 4 and
empty stdout. Invalid arguments fail with exit 2.

Entry bodies are project data inside collision-proof fences. Context assembly changes no files,
does not recover transactions and does not invoke an agent or model. Existing project validation
and its errors remain authoritative.

Assembly reads and validates every candidate, retaining exact lengths and source fingerprints
instead of body strings. It selects entries before rereading only selected sources. Each selected
read checks the complete original source and its regular non-symlink canonical identity. If a
selected source changes during assembly, context fails with retry guidance instead of combining
old selection metadata with a new body; filesystem failures still report their existing error
class. No partial bundle is printed. Unselected sources are not reread; this is not an atomic
archive snapshot. Stable-source outputs and omission lengths remain exact.

Retained selected body bytes are at most four times the Unicode character budget, even counting
repeated entry occurrences. A single bounded source image is loaded at a time. Path/count/evidence
metadata, parser/allocator memory and Markdown/JSON rendering copies remain separate costs.
Breadcrumbs retain task/handoff metadata without bodies and keep their original reads. Each
ordinary canonical/projection source read obeys the separate 16 MiB allowance, independently of
the output budget. Oversized sources fail validation even when their body would be omitted. See
[resource limits](resource-limits.md).

In the terminal interface, use `/context` or `/context --max-chars 32000`. Context always belongs
to the selected project, including while browsing global knowledge. The literal reader supports
arrows, PageUp/PageDown, Home/End and Esc to return. Invalid or failed reads preserve the current
view; unsaved editors, unfinished forms and pending reviews block entry. Delayed replies retain
unfinished work, recovery views and changed project/root selection. Budgets and results are not
persisted. Terminal syntax requires separate `--max-chars N` words.
