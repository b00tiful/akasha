# First run

Build the CLI with `cargo build -p akasha-cli`. For optional agent onboarding also build
`cargo build -p akasha-mcp`; the executables are in `target/debug/`. Product binaries stay
separate from private memory.

Choose a **new** data directory whose parent exists. Setup copies a version-1 configuration,
an empty project registry, canonical agent instructions and seven editable templates. It creates
no inferred project knowledge and does not change your shell, user configuration or agent homes.

## Terminal walkthrough

From the repository you want to remember, open `akasha --root /absolute/path/to/new-memory tui`.
The initial missing-root diagnostic is expected. Enter:

1. `/setup /absolute/path/to/new-memory` — inspect the exact files and destination.
2. `/confirm PLAN_ID` — use the complete displayed `sha256:…` identifier. `/discard` cancels
   without creating any file.
3. `/init my-project` — inspect the project scaffold, repository pointer and registry update.
   You can append a repository path when it differs from the launch directory.
4. `/confirm PLAN_ID` — use this initialization review's identifier.
5. `/onboard` — view the optional external-agent handoff. F5 loads the empty project for offline use.

Setup is available before a project has loaded. To create another root, start a separate session
with `--root` pointing to the new destination. Paths in the TUI are literal remainders of the
command; spaces are supported without shell quotes. Later launches still need `--root`,
`AKASHA_ROOT`, or the existing user root configuration; setup does not persist that preference.

## Named commands

```sh
akasha setup-root /absolute/path/to/new-memory
akasha setup-root /absolute/path/to/new-memory --plan-id 'sha256:DISPLAYED_ID'
# Run from the repository:
akasha --root /absolute/path/to/new-memory init my-project
akasha --root /absolute/path/to/new-memory validate
akasha --root /absolute/path/to/new-memory onboard
```

Replace the example path and identifier. `setup-root` always uses its positional destination;
combining it with `--root` or `--project` is an error. Every named command supports explicit
`--json`; errors use stderr and a nonzero exit code. Preview and handoff are read-only.

## Optional connected-agent population

`onboard` validates the initialized project and prints a local stdio executable name, an exact
JSON argument array and a request to send to an external MCP-capable coding agent. If the tools
are unavailable, configure that descriptor temporarily in your client's MCP settings, using the
installed executable or its absolute build path. Akasha does not start an agent, detect a live
connection or install client wiring. No network listener or embedded model is involved.

The external agent calls `akasha_onboarding_prepare`, inspects repository sources with its own
tools, and submits supported facts, labeled inferences and unknowns. Facts/inferences need
repository-relative evidence with whole-file SHA-256 fingerprints; inferences/unknowns need
rationale. Unsupported historical decisions and invented tasks should be omitted.

The agent calls `akasha_onboarding_validate`, then `akasha_onboarding_preview`, shows the exact
summary and obtains **human approval through the MCP host** before `akasha_onboarding_apply`.
Keep preview and apply in the same server lifetime. A supplied identifier alone is not human
approval. After an uncertain result, inspect current state and obtain a fresh review; do not
blindly replay apply. Finish with `akasha --root PATH --project SLUG validate`.

The synthetic tests exercise populated memory and the existing approval binding without invoking
a model. This release slice does not add observed acceptance for another live agent client.

## Interrupted setup

Apply creates a private persistent sibling `.NAME.akasha-setup.lock` and a temporary
`.akasha-setup.json` inside the new root. All root files use exclusive creation. `akasha.toml`
publishes last; other product operations refuse while the setup journal exists.

After interruption, run `setup-root PATH` (or `/setup PATH`) again and review the new plan.
Only an exact journal-owned partial tree from the same bundled defaults can resume; it fills
missing files without replacing existing ones. Review cancellation does nothing. A changed file,
symlink, unexpected path, or different bundle causes refusal and preserves the partial tree.

If refusal persists, stop writers and back up the entire partial root before inspection. An
interruption immediately after directory reservation may leave a directory without a journal;
an interrupted file operation may leave staging residue. These are deliberately not deleted or
automatically adopted. Choose a new destination or reconcile the backed-up partial root manually.
Do not remove a journal to bypass the check. A completed root refuses repeat setup; continue with
`init`, or `validate` if a project is already initialized.

Returned filesystem errors need the same inspection as an interrupted process. Once the
filesystem problem is corrected, use the remaining state to choose the next action:

| Remaining state | Next action |
|---|---|
| Destination absent | Prepare and confirm a new setup review. A private sibling lock may remain. |
| Exact journal-owned partial tree | Prepare a fresh review and resume. Previously published files are retained; supporting directory syncs are retried before journal removal. |
| Unowned incomplete directory, changed files, missing supporting files after configuration, or staging residue inside the root | Back up and inspect the partial root. Setup refuses adoption; preserve it and choose a new destination or reconcile it manually. |
| Complete defaults without a journal | The final directory sync may have failed after journal removal. Repeated setup refuses this existing root; inspect its files, then continue with `init`, or validate an already initialized project. |

A persistent supporting-directory sync or journal-removal failure returns an I/O error and
retains the journal. Exact visible bytes alone do not mean setup finished. Fresh review and cancellation
remain read-only. Private staging residue beside the sibling lock is also preserved; it is
never adopted as a lock or setup journal. Keep it with the backup during inspection.

Ten regression tests cover 290 returned-error and preservation scenarios, including genuine
partial writes, every default-file publication and directory-sync boundary, compound staging
cleanup failures, repeated refusal, exact bytes/modes, released locks and validated init/handoff
after retry. Process-exit tests additionally cover publication boundaries. Physical power loss,
device flush behavior and interruption within kernel calls are not covered. Existing project
mutation recovery is described separately in [the recovery runbook](recovery.md).

## Interrupted project initialization

Use `/recover-init` in the TUI, or `akasha --root PATH recover-init` in the shell,
to review the root's pending initialization transaction without changing files.
Back up the root and journaled repository first. The review lists exact paths,
hashes, presence and the discard/rollback/finalize outcome. `/discard` cancels;
`/confirm PLAN_ID` or `recover-init --plan-id PLAN_ID` applies only a matching fresh
review under the registry lock. Changed or unexpected artifacts refuse safely.

Recovery never starts a new project. After rollback, prepare a separate `/init`
review; after finalization, refresh/select and validate the existing project.
This works before a project can load. F5 and `/recovery` concern note journals.
See [the initialization recovery procedure](recovery.md#interrupted-project-initialization).

An initialization I/O error can occur after a file became visible. The initializer
tracks a published pointer before syncing its directory so rollback retains ownership.
Recovery retries surviving directory syncs before removing its journal; persistent
filesystem failures keep returning an error. Inspect the remaining state, correct
the failure and obtain a fresh recovery review. An error after journal removal can
leave a completed result without a journal, so validate before starting another init.
