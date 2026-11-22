# Runbooks (removed)

Executable runbooks (master plan 2026-10, #11) are gone: their screen left the window on 2026-11-08 and the Rust behind
it (`runbooks.rs`, the four `runbooks.*` actions and their Tauri commands, `runbooks.confirm_step`) on 2026-11-10 (S6).

What replaced them:

- **Repeatable work on a schedule or an event** is an automation (Rail → Automations; [SCHEDULER.md](SCHEDULER.md)).
- **A one-off procedure** is a request to the COO or a project's lead, which hands the steps to subagents. A note
  written as a checklist still works as instructions for them; nothing reads `runbook: true` any more.

The `runbook_runs`, `runbook_steps` and `runbook_approvals` tables are still created (the schema is a const in
`session_store.rs`) and stay inert; `usage_ledger` still joins `runbook_steps` for old night costs. The old format and
behaviour are in git history (this file before 2026-11-10) and in [docs/BUILD_LOG.md](BUILD_LOG.md).
