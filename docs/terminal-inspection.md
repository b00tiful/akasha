# Read-only terminal inspection

These commands use the shared core and show a scrollable, literal text reader:

| Command | Result |
|---|---|
| `/validate` | Validate the selected project's configuration, canonical metadata, links and state |
| `/breadcrumb` | Show the selected project's open-task count and latest handoff |
| `/template TYPE` | Show the exact configured template, its resolved path, class and scope |
| `/onboard` | Prepare instructions and a stdio descriptor for an external onboarding agent |
| `/integrations codex [HOME]` | Inspect instruction-pointer and SessionStart-hook plans independently |
| `/integrations claude [HOME]` | Inspect the same two plans for Claude |

`validate`, `breadcrumb` and `onboard` take no arguments. Templates require one configured type
and a selected project. An explicit integration home may contain spaces; omitted homes use the
existing client discovery rules. Invalid syntax reports usage without starting an operation.

Use the reader's arrow/Page keys to scroll and Esc or `/back` to return to the list. Frontmatter,
Markdown headings, fences, placeholders and wikilinks remain visible as source. Terminal controls
are sanitized for display; the underlying template and note bytes remain exact.

Unsaved edits, unfinished forms and pending administration reviews guard command entry. A reply
also checks the original root/project/resolution inputs and retains unfinished work, active editing
or recovery views. An obsolete reply leaves the current document, undo state, graph report and
scroll intact. Resolve the pending work, leave source editing and retry the inspection.

These commands do not write the data root or client home. Onboarding starts no agent; integration
inspection applies neither plan. A failure retains the current reader under existing recovery
guidance. A successful read does not treat journal presence alone as failed recovery. Use
[`/recovery` and the recovery runbook](recovery.md) when an operation reports a pending journal;
F5 or `/refresh` remains the explicit existing retry after reconciliation and resolving drafts.
