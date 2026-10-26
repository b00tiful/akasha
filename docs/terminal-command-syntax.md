# Terminal command arguments

Commands without arguments reject extra text with `Usage: /COMMAND` before taking action.
For example, `/save other.md` does not save, `/discard keep` does not discard, and `/quit now`
does not exit. Remove the extra text and submit the intended command again.

These commands take no arguments:

| Purpose | Commands |
|---|---|
| Source editing | `/edit`, `/read`, `/save`, `/discard` |
| Form steps | `/previous`, `/next` |
| Navigation | `/home`, `/projects`, `/global`, `/ls`, `/back`, `/help` |
| Inspection and recovery | `/resolve`, `/validate`, `/breadcrumb`, `/onboard`, `/recovery`, `/recover-init`, `/refresh` |
| Lifecycle | `/lifecycle` |
| Animation and exit | `/motion`, `/quit`, `/exit`, `/q` |

Leading and trailing whitespace is allowed. Malformed commands keep the current source and
undo history, form drafts and reviewed previews, administration plans, graph navigation and
recovery view. They do not submit work or cancel an operation already running. The submitted
command still appears in command history.

Commands that accept arguments keep their usual syntax. Paths with spaces remain valid in
`/setup PATH`, `/init PROJECT [REPOSITORY]`, `/link PROJECT [REPOSITORY]` and client-home
arguments. `/confirm PLAN_ID` requires exactly the complete displayed plan ID. See
[workflow controls](terminal-workflows.md), [search](search.md) and [context](context.md).
