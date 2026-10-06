# Recovering a refused project mutation

Use this procedure when the desktop or TUI reports a pending `.akasha-edit-journal.json`
and refuses to load the selected project because a journaled file has unexpected
bytes. It covers shared journal versions 1, 2 and 3 on the tested Linux local
filesystem. Root initialization and client instruction/hook journals have separate
contracts.

The desktop's load/retry and the TUI's F5/`refresh` actions can roll back or finalize
a transaction. The TUI's `/recovery` inspects journal presence without writing or
reading its contents and preserves unsaved buffers underneath the diagnostic view.
Esc returns to retained drafts; `/discard` explicitly cancels them without writing.
F5 refuses while a draft or unfinished form remains. Drafts are only in memory:
preserve wanted text separately before closing the process or discarding.
Inspection reports a point-in-time location/presence; it does not establish that
the project is consistent or clear a refused load. The named CLI has no shared
note-journal recovery command; `recover-init` below handles root initialization only.

## Interrupted project initialization

Root initialization uses a separate version-1 journal beside the configured project
registry, such as `Meta/.projects.yaml.akasha-init-journal.json`. The new project may
not yet be registered or loadable. Recovery does not require selecting that project,
running from its repository, or having the current root template tree available.

Stop other writers and make a private backup of the complete data root and journaled
repository, including its pointer. Keep the original journal unchanged. Review with:

```sh
akasha --root "$AKASHA_ROOT" recover-init
```

The read-only review lists the exact project/repository identities, registry and
journal hashes, each planned artifact's expected hash and presence, and the proposed
outcome. No journal produces an explicit no-change result (`null` with `--json`).
Preview/cancellation create no lock or files. `--project` is rejected: this journal
is root-wide. In the TUI use `/recover-init`, inspect the scrollable review, then
`/discard` to cancel or `/confirm PLAN_ID` with its complete displayed identifier.
Drafts and unfinished reviews must be resolved before entering this operation.

To apply from the shell, copy the complete freshly reviewed ID into
`INIT_RECOVERY_PLAN_ID` and run:

```sh
akasha --root "$AKASHA_ROOT" recover-init --plan-id "$INIT_RECOVERY_PLAN_ID"
```

The core takes the existing registry lock, revalidates the complete review, and
performs only recovery. A busy lock is a wait/retry condition; retain the idle lock
file. Changed configuration/journal/registry bytes or artifact presence require
fresh review. Changed scaffold/pointer bytes, unexpected uncommitted paths, symlinks,
malformed journals and unknown versions refuse without removing artifacts.

- `discarded`: no initialization artifacts were published; remove the unused journal.
- `rolled-back`: remove only exact recognized uncommitted files/pointer and empty
  planned directories, then the journal. Registry bytes are retained. Create the
  project only through a **separate fresh init review** afterward.
- `finalized`: retain the exact committed scaffold/pointer/registry and remove the
  journal. Refresh/select the project and run normal project validation.

No outcome starts another initialization. A TUI apply attempt consumes its review,
including refusal; prepare again before retrying. F5 and `/recovery` keep their
selected-project note-journal scope. Root setup instead resumes through a fresh
`setup-root PATH` or `/setup PATH` review as described in [first run](first-run.md).

Init journals contain hashes, **not restorable source images**. Preserve unexpected
edits and investigate; use only independently verified original bytes/backups for
manual reconciliation. Do not infer old files from changed templates, edit journal
hashes, or delete a journal to make the root load. After an I/O failure, inspect the
remaining state and obtain a fresh review rather than replaying an old ID. A failure
after journal unlink can leave completed recovery visible without its journal.

Recovery retries the registry and projects parent directories and the repository
directory when it still exists. Finalization also syncs every committed scaffold
directory, deepest first. These checks run before journal removal even if a prior
attempt already removed the pointer or left exact committed bytes. A failed sync
retains the journal and reports an I/O error; correct the filesystem problem and
obtain a fresh review before trying again.

Five actual process exits verify journal/directory/file/pointer/registry publication,
exact snapshots and modes, lock release, recovery-only completion and stale replay
refusal. CLI and TUI fixtures also verify human-byte retention and fresh retry; the
keyboard walkthrough passes at 110×32 and 40×12. Physical/device durability,
in-kernel interruption and root-setup returned-I/O acceptance remain separate limits.
Initialization additionally has returned-error coverage for publication, rollback,
persistent completion-sync refusal and journal cleanup, with exact bytes/modes,
foreign-artifact preservation and fresh reviewed retry.

## When an operation reports an I/O failure

A storage, permission, write or sync error does not guarantee that no files changed.
Stop retrying while the filesystem problem persists; preserve the root and journal as
below. Once the cause is corrected, use the existing recovery/reload path and validate
the project before submitting a fresh reviewed creation or edit.

Recovery retries every journaled artifact's parent-directory sync before deleting its
journal, even if all bytes already look restored or committed. A failing completion
sync retains the journal. A failure syncing the project directory **after** journal
unlink is still reported as an error, but the complete transaction may already be
visible without a journal. Inspect the intended note/projections and validate rather
than assuming creation failed or deleting the existing note to make retry succeed.

Returned-error fixtures verify these paths with synthetic I/O errors. They do not
establish physical device or power-loss durability.

## 1. Stop writers and preserve the evidence

1. Record the selected root, project, journal path and complete failure message.
   A busy writer calls for waiting until that writer finishes, rather than file
   reconciliation. Keep the persistent `.akasha-write.lock` file in place.
2. Close Akasha windows/TUIs, stop agent writes, and pause external editors and
   sync tools for this root. Keep any unsaved editor buffer separately. Advisory
   locking does not stop an external editor.
3. Make an owner-only backup of the **complete data root**, including hidden
   journal/state files, root configuration and registry, outside the live root.
   Preserve the linked repository's `.akasha.toml` separately if it exists. Use a
   new destination and check that the copy completed; never overwrite a prior
   backup. Compare the copied files' exact bytes or hashes with the stopped root.
   Retain both unexpected external edits and the unmodified journal.
4. Inspect the backup, with all writers still stopped. Confirm the journal's
   project matches the selected project, its version is supported, and its note
   identities resolve inside that project. Projection identities must name its
   configured index or roadmap. Check paths and file types before copying any
   image back. A malformed journal, unknown schema, wrong project, path escape,
   symlink/non-regular file, or unreadable/missing required state/projection is a
   stop condition: preserve everything and investigate the cause. Do not invent
   images, edit journal metadata, delete the journal, or recreate the lock.

Backups and decoded images contain private note text. Keep them local with a
private parent directory (0700) and files (0600); avoid terminal dumps, shared
temporary directories, or uploading journals as diagnostics.

## 2. Compare exact images and choose rollback

Decode JSON string values as UTF-8 bytes before comparing them with files.
Escaped `\r\n`, quotes and Unicode in JSON are not their on-disk spelling. Preserve
line endings, trailing spaces and final-newline presence; do not use a Markdown
editor that normalizes them. Compare complete files, including state/projections.

| Journal | Recorded artifacts | Accepted current images |
| --- | --- | --- |
| Version 1 | `id`, `note_before`, `note_after`, `state_before`, `state_after` | Mutable note: exact before or after. Created note (`note_before: null`): absent or exact after. State: exact before or after. |
| Version 2 | Version 1 plus `projection.id`, `projection.before`, `projection.after` | The same note/state rules, plus exact before or after for the configured index/roadmap. |
| Version 3 | Every `notes[].id`/`after`, both `projections[].id`/`before`/`after`, state images | Each created note: absent or exact after. Each projection and state: exact before or after. |

Any third image causes refusal. A trusted journal represents the transaction
Akasha began; it is not proof against tampering. If its provenance is uncertain,
use a separately verified backup and investigate before restoring anything.

For the conservative rollback procedure, explicitly choose to abandon the pending
transaction **after preserving external changes in the verified backup**:

1. For each conflicting mutable note, maintained projection or state file, restore
   its exact recorded **before** image. Leave already matching files untouched.
2. A conflicting newly created note has no before image. After verifying its
   backup, remove that one live file to restore absence. The saved external text
   stays in the backup for review. Never interpret JSON `null` as file contents.
3. Compare the restored bytes again. Keep the live journal byte-for-byte unchanged.

In the development workspace, Node can extract a single decoded string into a
new private staging file without displaying it or changing the live root. Node
is a development tool here, not a product runtime requirement. Set the three
variables below to the verified backup and a new owner-only staging directory;
this example extracts the state preimage:

```bash
node --input-type=module - \
  "$RECOVERY_COPY/Projects/example/.akasha-edit-journal.json" \
  state_before "$RECOVERY_STAGE/state-before.toml" <<'JS'
import { readFileSync, writeFileSync } from 'node:fs';
const [journalPath, selector, outputPath] = process.argv.slice(2);
const allowed = ['note_before', 'state_before', 'projection.before',
  'projections.0.before', 'projections.1.before'];
if (!allowed.includes(selector)) throw new Error('Unsupported image selector');
let value = JSON.parse(readFileSync(journalPath, 'utf8'));
for (const key of selector.split('.')) value = value?.[key];
if (typeof value !== 'string') throw new Error('Image is absent or not a string');
writeFileSync(outputPath, value, { encoding: 'utf8', flag: 'wx', mode: 0o600 });
JS
```

For version 3, first check the selected projection's `id`; array order alone does
not establish whether it is the index or roadmap. Review the extracted image
and destination before restoring it. The extraction refuses an existing output
file and does not apply any restoration.

## 3. Retry recovery, validate, and reapply reviewed work

1. With external writers still stopped, reopen Akasha, select the same root and
   project, and submit the library load (F5/`refresh` in a clean TUI session).
   The existing core recovery runs under
   the project lock. An entirely untouched transaction is discarded; a partial
   transaction rolls back in reverse publication order; a complete, valid
   after-image is finalized. Restoring a conflicting artifact to its before
   image normally leads to discard or rollback.
2. If retry refuses again, retain the journal and all backups, close Akasha and
   investigate the new failure. Do not repeatedly guess images. Validation can
   also fail because an unrelated canonical file changed; this procedure does
   not repair arbitrary out-of-band project changes.
3. Confirm the journal is gone and the project loads, then run:

   ```bash
   akasha --root "$AKASHA_ROOT" --project example validate
   ```

   Require exit status 0. Check the affected notes, maintained projections and
   state against the expected rollback/commit images. A second load should
   require no recovery. Keep the backup until the human verifies the result.
4. Review retained external edits. Reload a fresh baseline and reapply wanted
   changes through the existing checked editor, entity/record lifecycle update,
   or reviewed create-only onboarding/event workflow, including its maintained
   projections and evidence. Never paste a raw state file or an immutable event
   back into the recovered root to bypass validation. Resume external writers
   only after validation and review succeed.

## Abandoned staging files

A process can exit while writing a hidden same-directory file named
`.<destination>.akasha-<pid>-<sequence>.tmp`. Akasha reserves that exact pattern
(nonempty destination, positive 32-bit PID and unsigned 64-bit sequence in normal
decimal spelling). Matching regular files are preserved but excluded from note
validation, fingerprints, context, library and search. They may contain partial
private text or be another hard link to a created note. They are not recovery
journals or canonical notes and do not supply replacement source.

Include these files in the complete backup. A matching filename proves neither
ownership nor that its writer is dead; do not promote it to Markdown, edit it in
place, or delete files by a wildcard. Recovery does not automatically clean up
this residue. Symlinks, directories using the reserved pattern, malformed names
and unrelated non-Markdown files still cause validation errors. Preserve and
investigate those errors instead of broadening the ignore rule.

## Verified boundary

Disposable Rust fixtures cover repeated refusal with exact byte preservation,
backup retention, preimage restoration, successful retry and validation for all
three shared journal versions, including created-note absence and conflicts in
projections/state. Onboarding also has child-process exit checks after synced
journal creation, each of two notes, the note batch, index, roadmap and state.
Recovery in the parent proves the crashed child's lock is released without
destructors; a second recovery is a byte-preserving no-op.

TUI acceptance additionally verifies metadata-only inspection over retained source/form drafts,
repeated refusal with every fixture file preserved, explicit discard without writes, and core
rollback/revalidation after operator reconciliation. Nine Linux PTY scenarios pass, including
keyboard-only use and initial refused loads at 40x12, with exact terminal restoration. Background
observations of active writers' transient journals retain the view and signal external change.

Another 147 actual child exits cover staging creation, partial file content,
before/after file sync, immediate hard-link/rename publication and before/after
caller directory sync across versions 1/2/3 publication and replacement rollback.
Exact snapshots include all retained scratch bytes; validation, repeated recovery,
lock release and fresh publication with residue left in place pass.

These tests exit at instrumented boundaries, including between two parts of a
staging write. They do not simulate physical power loss, interruption inside a
kernel write/sync syscall, device flush failures, or concurrent external writers
during manual reconciliation. They do not establish support for other operating
systems or network filesystems.
