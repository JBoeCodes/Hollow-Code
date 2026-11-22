# Vault and data kept on purpose

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point.

## Data kept on purpose

- The paused AI company was removed on 2026-11-10 (git tag `company-paused-20261110` keeps the code; record: company assignments). Its 46 tables stay, inert, in databases that have them: never drop them. A queued row it left (`restrictedResume`, an `s-company-` chat turn) is settled once as failed (`run_pump::removed_company_row`). The CFO (`cfo/`) holds the only copies of the company pieces still used; see [CFO](../CFO.md).
- A project's `local-*` workspace id hashes its absolute path, so Move Hollow home changes it. Append-only tables (`audit_log`) keep the id a row was written with; `workspace_aliases.rs` records old to new. Any reader that looks up audit, usage or history rows by workspace id resolves through it (`Aliases::ids_for` for an `IN (...)` filter, `current` to group): see [Move Hollow home](../DIRECTORIES.md#move-hollow-home-2026-11-12).

## Vault and directories

- Workspace catch-up, task review labels, Calendar refresh and recovery: [Catching up a stale workspace](../reference/vault.md#catching-up-a-stale-workspace).
- Projects + Vault setup, external project discovery, live Library reads and the refresh/ignore contract: [Projects and Vault directories](../DIRECTORIES.md). Setup lives in Settings → Hollow folder.
