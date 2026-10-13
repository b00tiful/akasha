# Search

Named commands search the selected project by default and return at most 30 hits:

```sh
akasha search 'memory' --limit 10
akasha search 'memory' --global
akasha search 'memory' --all --limit 100 --json
```

Search matches literal text in validated note titles, exact paths and source, using Unicode
lowercasing. Results sort by exact identity and report the total and truncation. Limits are
1..100; no matches is a successful result. Quotes in the shell keep a multiword query together.

In the terminal interface, the default remains 100 hits:

```text
/search memory --limit 10
/search-all memory --limit 100
/global
/search memory --limit 1
```

`/search` uses the currently selected project or global scope. `/search-all` includes all projects
and global notes while retaining the selected navigation scope. Enter or `/open NUMBER` opens
an exact result; Esc returns to the result list, then to its parent list.

The terminal accepts one trailing `--limit N`. Missing, duplicate, misplaced, signed and
out-of-range limits show an error and retain the current view. To search text containing the
standalone word `--limit`, put `--` before the whole query:

```text
/search -- memory --limit 10
```

That searches the literal text `memory --limit 10` with the default bound. The terminal prompt
does not parse shell quotes; type multiword text directly. Other option-looking text is literal.

Search leaves files unchanged and performs no transaction recovery. Unsaved edits, unfinished
forms, administration reviews and busy/recovery states guard entry. Successful delayed replies
retain editing, unfinished work, recovery views and changed request/scope selections. Core
validation errors retain the current reader; existing pending-journal failure guidance remains
available through [recovery inspection](recovery.md).
