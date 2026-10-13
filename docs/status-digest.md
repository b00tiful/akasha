# Cross-project status and digest

Use an existing linked repository, or specify a configured root and a registered project:

```sh
akasha --root /path/to/memory --project example status
akasha --root /path/to/memory --project example digest --limit 100
akasha --root /path/to/memory --project example status --json
```

`status` lists project metrics by project name. `digest` lists the same metrics by descending
latest canonical note date, with name ordering for ties and missing dates last. Both include
every registered project, including paused or archived projects, and validate global knowledge.
The selected project anchors resolution and does not filter these reports.

Each report shows total projects, notes, global notes, configured categories, open tasks and
problems, canonical outgoing links and latest canonical note date. Project rows include registry
status, notes, populated categories, open tasks/problems, outgoing links and latest date.
Open counts follow the configured semantic roles and open statuses. Link counts use canonical
note outgoing targets; they are distinct from the broader `links`/`backlinks` source coverage.
Latest dates come from canonical `date` metadata, not file modification times or review dates.
The digest summarizes metadata without generating prose or claiming changes since a prior run.

The default is 30 project rows. `--limit N` accepts 1 through 100 and explicitly expands output.
Whole-library totals stay exact when rows truncate. Plain output neutralizes controls and keeps
up to 120 Unicode characters per display field, adding `…` when truncated; JSON retains existing core
display metadata and exact identities. Bounds count project rows, not JSON bytes.

JSON is opt-in. Its additive report object contains `kind`, `root`, `selected_project`, `limit`,
`truncated` and `dashboard`. `dashboard.projects` and all aggregates cover the full library;
`dashboard.project_metrics` contains the ordered bounded rows.

The TUI exposes `/status [--limit N]` and `/digest [--limit N]` through the same core and plain
renderer. Arrow/PageUp/PageDown keys scroll; Esc or `/back` returns to the existing list.
Drafts, unfinished forms and administration reviews prevent replacement. Failed or obsolete
responses preserve the current view. Reports do not persist across launches.

Both commands read saved sources afresh and refuse unknown projects, invalid configuration,
malformed or unsafe sources, stale state and pending note recovery journals in any project.
Failures print to stderr with a nonzero status; pending recovery returns 5 and validation
returns 4. Reporting never recovers or writes. See [recovery guidance](recovery.md) for explicit
operator reconciliation. Reads retain the existing non-atomic concurrent snapshot boundary;
archive-scale latency has not been measured.
