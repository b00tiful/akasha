# Inspect the selected project

Run `akasha resolve` from a linked repository, or select a root and project explicitly:

```sh
akasha --root /path/to/memory --project example resolve
akasha --root /path/to/memory --project example --json resolve
```

The result shows the root and its selection source, project and its selection source,
repository pointer (if used), registry, repository directory and project directory.
Explicit project selection reports `none (project selected explicitly)` for the pointer.
Failures return a nonzero exit status with an explanation on stderr.

In the terminal interface, enter `/resolve` to inspect the current session's selection.
It accepts no arguments; use `/project SLUG` to select another registered project.
The reader shows the same plain fields as the named command. Arrows and PageUp/PageDown
scroll; Esc or `/back` returns to the current list. Unsaved drafts, unfinished forms,
administration reviews and running operations must be resolved first. Failed or obsolete
replies retain the current view.

Resolution reads identity and configuration without changing files or recovering transactions.
It can succeed while a note transaction is pending or a note is invalid. Use `validate` for
canonical note validation and the [recovery workflow](recovery.md) for interrupted writes.
