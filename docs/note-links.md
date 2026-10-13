# Inspect note links

Use an exact full vault-relative Markdown identity in a configured data root:

```sh
akasha --root /path/to/data --project example links Projects/example/entities/core.md
akasha --root /path/to/data --project example backlinks Projects/example/entities/core.md
akasha --root /path/to/data --project example --json backlinks Projects/example/entities/core.md --limit 100
```

`links` lists distinct outgoing file targets. `backlinks` lists distinct source files that link
to the requested note. Both return at most 30 results by default; `--limit 1..100` expands or
reduces that bound. Results are sorted by exact identity and report the full count, truncation
and the number of canonical notes and project projections scanned. No source bodies are printed.

Inspection scans saved canonical notes in every registered project and global entity folder,
plus every project's configured index and roadmap. Templates, Inbox, Meta, arbitrary Markdown
and unregistered projects are excluded as sources. A configured index or roadmap is also a valid
inspection input. `--project` selects resolution context; it does not restrict incoming sources
to that project.

The supported wikilink subset uses exact vault-relative targets, optional target `.md` suffixes,
aliases, embeds and file fragments. Multiple references from one source to one target count once.
Same-note fragments count as self links. Frontmatter, code, comments and escaped links do not
contribute edges. Optional projection frontmatter is skipped without applying canonical-note
schema requirements. Heading and block existence remain unchecked.

Each result is classified as `canonical`, `projection` or `other`. `other` means a valid outgoing
link to root-contained Markdown outside source coverage. That classification grants no arbitrary
file access: inspection inputs must belong to the canonical/projection set, and existing desktop
and TUI document loading still accepts only projected book identities.

Unknown, shortened, unsafe or case-mismatched input identities fail with exit 4. Broken links,
malformed source and stale project-state evidence also fail with exit 4. Any registered project's
pending note journal fails with exit 5; reconcile and recover through the existing recovery
workflow before retrying. Operational filesystem errors use exit 6. Failures go to stderr with
empty stdout, including under `--json`.

These commands perform no recovery or writes and do not read editor drafts. Each invocation
validates current saved data, without a cache or atomic filesystem snapshot. Later document
navigation must still validate the identity and respect dirty-editor guards. The desktop's
[inline activation](inline-navigation.md) now supplies source-bound core checks and keeps
non-book targets visibly unavailable. The [desktop Backlinks panel](backlinks.md) adds bounded
incoming inspection and baseline/edge-checked canonical opening in the shared note inspector.

Plain output replaces control characters in displayed identities with spaces. Opt-in JSON
preserves exact identities as escaped data. Akasha needs no external editor, plugin or model
for inspection.

## In the terminal interface

The persistent TUI exposes the same saved-file graph through `/links` and `/backlinks`:

```text
/open Projects/example/entities/core.md
/backlinks
/follow 1
/back
/links Projects/example/entities/core.md --limit 100
/backlinks Projects/example/index.md --limit 1
```

Omit NOTE only while a canonical note is open. Otherwise supply the exact full vault-relative
identity; paths with spaces need no quotes. The only option is a trailing ` --limit N`, with
30 results by default and explicit limits from 1 to 100. Tab after either command suggests
loaded canonical note identities; configured indexes and roadmaps can be entered explicitly.

The scrollable report shows direction, exact totals/truncation, canonical/projection coverage
and numbered classifications across all registered projects and global knowledge. Arrow and Page
keys scroll. `/follow NUMBER` opens a canonical result in a report about a loaded canonical note,
rechecking the saved baseline and outgoing occurrence or current bounded incoming edge through
the core. `/follow` then Tab lists canonical result numbers and exact identities. Esc from the
followed clean note returns to the report at its previous scroll position; Esc from the report
returns to the library list. `/open PATH` remains available through the existing checked loader.
Projection reports and projection/`other` results remain inspection entries. Refresh the library
before binding newly added canonical notes. A successful report
replaces the clean reader, so subsequent inspection commands need an explicit NOTE until another
canonical note is opened.

Dirty editors, unfinished forms, pending administration reviews and running operations block
inspection. Failures retain the current reader/report, with a visible error. Pending journals
refuse without parsing, recovering or writing; activation failures also retain the report.
Dirty followed notes block report return until saved or discarded. Background changes retain
the report behind the external-change signal; refresh or inspect again to obtain fresh results.
Reports and their single return parent are kept only in memory and cleared by unrelated successful
navigation. Use `/recovery` and the existing recovery workflow
when appropriate. Reports use saved files and exclude drafts; they are point-in-time inspection,
without inline edge activation or automatic refresh of the report. Display replaces controls
with spaces; use the named command's explicit JSON for exact control-bearing identities.
