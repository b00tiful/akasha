# Desktop backlinks

Open a canonical note in Local Akasha or Global library, then open **Backlinks** in the note
inspector. The panel is available in Live Preview, Source and Reading modes. Tab reaches its
summary and buttons; Enter/Space uses the normal controls. Opening the panel inspects saved files
through Akasha Core, without recovery or writes.

The panel initially shows at most 30 distinct incoming files in exact identity order. It reports
the full total, whether the result is truncated, and the numbers of canonical notes and configured
indexes/roadmaps scanned. **Show up to 100** explicitly expands a truncated result. The maximum
is 100; larger totals remain visible. **Refresh** repeats inspection at the current bound.

Coverage includes all registered projects and Global, even in Local Akasha. Templates, Inbox,
Meta, unregistered projects and arbitrary Markdown are excluded as graph sources. Multiple
references from the same file count once; same-file fragments can create a self backlink.
See [note-link inspection](note-links.md) for parser and coverage details.

Click a canonical source identity to open it in the same inspector. Rust rechecks the displayed
note's exact saved source and requires the source identity to remain a canonical incoming file
in the current bounded result. The checked result opens directly, without a recovery-running
reload. Index/roadmap identities remain visible with an explanation that they are unavailable
as library documents. Local Akasha opens only the selected project's books; cross-project and
Global sources direct you to Global library.

Unsaved changes may inspect the saved baseline; the panel explains that results describe saved
files. Save or Discard before opening a source. If you start editing while an open request is
pending, the exact draft stays visible. Refused or obsolete requests preserve the viewer.

A failed refresh preserves previous results, labels them stale and disables opening them until
a successful refresh. A changed displayed baseline requires a fresh note load. Invalid data or
any registered project's pending mutation journal refuses inspection and activation; reconcile
through the [recovery workflow](recovery.md) before retrying. The panel performs no recovery.

Results are freshly validated saved-file reads, without an atomic filesystem snapshot or stored
graph. Native visual/physical-key acceptance remains unverified under the existing capture issue;
automated core, adapter and DOM tests cover this slice.
