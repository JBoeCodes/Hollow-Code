# Core workspace finish and native review

2026-09-27 · Merged in PR #193 (`c0c9fc5`), installed as 0.5.3.

## Current experience

The conversation remains primary and has nothing stacked above it. The
workspace panel's **Overview** recap (Welcome back, Made here) went with the
panel (COO rebuild L4, 2026-11-10): a project's notes open as files through
the top bar's Project files, and a Result opens in the Preview pane with its
review bar. Opening a result preserves the unsent composer draft.

File loading has an explicit state and retry. A failed read clears the previous
document from view and disables editing/saving until the requested file loads;
saved drafts remain retained. File actions → Reload file (keep draft) reads the
current disk file without discarding the draft. Compare-and-swap remains the save
guard. **⌘S / Ctrl+S** saves within the file editor; clean files have no Save button.
Workspace-list loading/failure is distinct from an empty collection. Home's
"Pick up where you left off" does the same for `workspace_digests` (review F03,
2026-10-13; `homeReturn.ts` `returnSectionState`): a first read that fails shows
"Couldn’t read your recent workspaces" with the error and **Retry**; a failed
refresh keeps the last successful cards under "Not refreshed"; only an empty
set of workspaces to return to hides the section.

## Read model and boundaries

`workspace_reentry.rs` builds `WorkspaceDetail.reentry` as part of the existing
`read_workspace` IPC, without an AI request or new command. It reads the brief
and plan through the existing 1 MB guarded reader; excerpts cap at 480 characters
for Next step and three 240-character plan items. The checklist count is from
the actual plan and is null if reading fails. Fenced examples do not become tasks.

The latest listed conversation's newest turn is selected with a SQL LIMIT 1
query (`session_store::latest_turn`), without loading its transcript. A receipt
must match the finalized history run, chat source, session key and terminal status.
Missing or mismatched receipts produce unavailable checks, not zero failures.
Zero recorded checks, failed checks and inconclusive checks have distinct wording.
Run success means the run finished; it does not certify the workspace outcome.
The existing 1,000-session listing limit and retained receipt coverage still apply.

Wire additions (camelCase):

- `WorkFile.modified`: nullable filesystem modification timestamp.
- `WorkspaceDetail.reentry`: `asOf`, nullable `nextStep`, `openItems`, nullable
  `openItemCount`, nullable `lastRun`, and `gaps`.
- `lastRun`: `sessionKey`, nullable `status` and `finishedAt`, nullable
  `failedChecks`, `checkCount`, `unknownChecks`, and `verification`.

`hollow.visited.<workspaceId>` stores the previous successful entry timestamp in
localStorage. Timestamp comparison counts **current** output files modified since
that visit; it does not claim to inventory deleted files. Unknown timestamps hide
the comparison. The snapshot refreshes on opening, file save, completed turns,
manual refresh and coalesced workspace filesystem events. No database migration.

## Isolated native review

> **Superseded (2026-10-02)** by the candidate identifier, which separates
> every Hollow store by itself (the [test bed](history/TEST_BED.md) that
> built and checked candidates was retired on 2026-10-09). The recipe below
> is kept for history.

Use a separate Tauri identifier/product name, an explicit `HOLLOW_CONFIG` pointing
to a synthetic vault, and an absolute `HOLLOW_APP_DATA_DIR` for preferences,
provider settings and the service registry. With that app-data override, native
startup does not register the production global shortcuts or repoint its login
item. Unit tests retain their existing per-process temporary app-data directory.

The override is **not an OS sandbox**: provider login state, custom themes and
other system resources can still be read. Vault-derived stores remain keyed by
the configured vault. Do not point the review build at the production vault or
copy production service/automation settings into its private app-data folder.

The review config used product `Hollow Studio Review`, identifier
`com.hollow.app.studio-review`, and ad-hoc signing. Build with capped jobs and
`tauri build --debug --bundles app --config <review-config.json>`. Launch the
result's executable with both overrides, rather than replacing `/Applications/Hollow.app`.
This is a debug review build, not a production release or performance benchmark.

## Verification

See the newest [build-log entry](BUILD_LOG.md) for test counts and the native
workflow exercised. The merged release was built with production signing and
installed through the existing atomic installer. Source provenance, model
self-test, native relaunch and task/workspace file preservation were verified.
The synthetic agent run remains a separate debug-review check; no new production
agent request was sent during release verification.
