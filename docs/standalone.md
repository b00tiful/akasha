# Akasha and external editors

Akasha operates through its shared Rust core, CLI, terminal interface and desktop client.
Its on-demand MCP server serves agent-assisted onboarding only. No Obsidian process,
plugin, REST API, credential or MCP bridge is required or connected.

Markdown with frontmatter and wikilinks remains authoritative. Users can independently
open the same data folder in Obsidian or another editor. That editor's settings, plugins,
cache and UI belong to that application. Akasha treats edits to shared Markdown as external
file changes and applies its existing validation, refresh and conflict safeguards.
The terminal retains active readers and source buffers when changes arrive, and refreshes idle
browser selections in place; see [document opening and refresh](terminal-refresh.md).
Creation, lifecycle and administration keep newer work when preparation or preview replies become
obsolete; see [workflow review controls](terminal-workflows.md).
Argument-free terminal commands reject extra text before changing drafts, reviews, recovery
views or files; see [command syntax](terminal-command-syntax.md).
Readers reuse one exact document layout for scrolling; reader/editor focus pauses prompt
decoration. See [terminal runtime and measurement limits](terminal-runtime.md).
The shared core rejects excessive YAML metadata using explicit [resource limits](resource-limits.md),
with bounded registry reads and exact Markdown source preservation.

Agents read files directly or use bounded `akasha context` and named CLI commands. Humans
use Akasha's own terminal or desktop interface. Development notes under `akasha/` remain a
temporary Markdown workbench accessed directly; they are not automatically migrated into
a configured product root.

Named and terminal context accept an explicit `--max-chars N` budget, preserving the 16,000
default, whole entries and exact omissions. See [context usage](context.md).

Akasha's core already parses and validates its supported exact vault-relative wikilinks
and exposes outgoing note identities. The bounded named CLI commands `links NOTE` and
`backlinks NOTE` inspect canonical notes and configured project indexes/roadmaps through
the shared core; see [note-link inspection](note-links.md). The desktop now provides
[guarded inline navigation](inline-navigation.md) using core-bound occurrences and exact-source
checks. Its [Backlinks panel](backlinks.md) now shows bounded incoming identities, exact
totals/coverage and checked canonical source opening while retaining dirty and stale views.
The terminal's `/links [NOTE] [--limit N]` and `/backlinks [NOTE] [--limit N]` now show the
same saved-file graph in bounded read-only reports; omit NOTE while a canonical note is open.
Numbered `/follow` activation rechecks canonical source/edge bindings; Esc returns to its report.
See [terminal controls](note-links.md#in-the-terminal-interface).

The former REST/MCP bridge, launcher, fallback logger, dedicated credentials and native
backlink prototype have been removed. Historical development evidence remains archived
in private project memory and is not an installation procedure.

Cross-project `status` and `digest` provide bounded shared-core dashboard metrics, with plain/JSON
named commands and matching guarded TUI readers. See [status/digest usage](status-digest.md).

Named `resolve` and terminal `/resolve` inspect the current root/project selection and its
provenance without changing files. See [resolution inspection](resolution.md).

Named and terminal search share core matching, scopes and exact identities. The terminal now
accepts explicit 1..100 limits while keeping its 100-hit default. See [search usage](search.md).

Validation, breadcrumb, template, onboarding and integration inspection use literal terminal
readers with request-bound replies and retained unfinished/recovery views. See
[read-only terminal inspection](terminal-inspection.md) for syntax and controls.
