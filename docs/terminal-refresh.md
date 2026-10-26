# Terminal document opening and refresh

Akasha checks for external changes every five seconds while focused and once on focus return.
An unchanged check stays silent. Automatic checks validate the library and compare loaded source
baselines through the serialized worker; they perform no recovery or writes.

When a change affects an active note, source editor, diagnostic, graph report, search result,
form or administration review, Akasha retains that view and shows `external change`. The reader's
scroll, source buffers, undo history and graph-return context remain available. Checks pause
until a successful explicit reload. Foreground work queued during a check is retained too.

In an idle browser, a valid change refreshes the library while preserving the current project or
global scope, category and selected identity. Missing targets fall back to a validated parent.
The command draft and focus remain in place.

Use **F5** or **`/refresh`** to reload explicitly. `/refresh` takes no arguments. Resolve unsaved
edits, unfinished forms and administration reviews first. A clean source editor can be left by
an intentional open or reload command. A successful reload returns to the project categories,
clears the external-change signal and uses the existing core recovery path. Reopen a note to
read its current saved source. A refused reload retains the current view and signal.

Open/reload replies bind their original root, project, working directory and resolution environment.
They retain unfinished work, newly entered editing and changed recovery views before replacing
reader or navigation state. An intentional command from an unchanged clean editor remains valid.
Obsolete background checks, including failures, are discarded. A successful read is a point-in-time
view; these guards do not create an atomic snapshot across independent filesystem changes.

When recovery is required, use [`/recovery` and the recovery runbook](recovery.md), reconcile the
source and resolve drafts before retrying F5. An explicit retry from the same recovery state may
complete core recovery and reopen the library. Opening a document retains its existing core
recovery behavior. Named CLI output, navigation-state storage and MCP behavior are unchanged.
