# Terminal creation, lifecycle and administration reviews

Use `/event TYPE` or bare `/handoff` for a guided immutable event, `/create TYPE` for a
configured record/entity, and `/lifecycle` while a project record/entity is open. Enter an
explicit path where requested. Ctrl-N advances through multiline fields and the maintained
roadmap/index; Ctrl-P returns to revise. Review the complete exact note and, for mutable
notes, its projection before Ctrl-S applies. `/discard` cancels a form without writing.

Administration uses `/integration apply|remove instructions|hook CLIENT [HOME]`, `/setup PATH`,
`/init PROJECT [REPOSITORY]`, `/recover-init`, or `/link PROJECT [REPOSITORY]`. Inspect the
displayed exact plan, then use `/confirm PLAN_ID` with its complete ID. `/discard` cancels
without applying. Preparation from an unchanged clean source editor remains available.

If a preparation finishes for an obsolete session or while new work/recovery is present,
Akasha keeps your current view and reports that it was retained. Resolve the work or return
from recovery, then request a fresh form or review. Preview replies also require the exact
current form, path, field values, note/projection sources and review stage. A stale preview
keeps your drafts and undo history; advance again to request a fresh preview.

Input to source fields, form navigation, save and discard pause while a foreground operation
is running. Preparation/preview retention does not cancel a write already submitted through
Ctrl-S or `/confirm`. Checked core writes still revalidate current files and refuse drift;
administration apply attempts consume their review and require fresh preparation after failure.

Use `/recovery` for metadata-only journal inspection; Esc returns to retained work. Follow
[the recovery runbook](recovery.md) before retrying a refused transaction. F5 reloads the
library and retries note recovery; root initialization recovery uses `/recover-init` and its
own confirmation. See [document opening and refresh](terminal-refresh.md) for external changes.
