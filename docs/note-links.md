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
