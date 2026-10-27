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

Assembly releases bodies whose complete entry cannot fit the requested budget and measures
selection without rendering rejected bundles. It still reads and validates every candidate,
preserves exact omission lengths, and permits explicit expansion to include those whole bodies.
Only the newest handoff body is retained; breadcrumbs retain task/handoff metadata without bodies.
This is not an archive memory cap: transient whole-note reads, paths and many individually fitting
bodies can still consume substantial memory. See [resource limits](resource-limits.md).

In the terminal interface, use `/context` or `/context --max-chars 32000`. Context always belongs
to the selected project, including while browsing global knowledge. The literal reader supports
arrows, PageUp/PageDown, Home/End and Esc to return. Invalid or failed reads preserve the current
view; unsaved editors, unfinished forms and pending reviews block entry. Delayed replies retain
unfinished work, recovery views and changed project/root selection. Budgets and results are not
persisted. Terminal syntax requires separate `--max-chars N` words.
