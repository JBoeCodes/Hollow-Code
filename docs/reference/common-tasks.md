# Common tasks

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Common tasks", moved
there word for word on 2026-10-13 (only relative links were re-based). Read it
when adding a page, a connector, a slash command or anything that spawns an
agent or CLI process, and when debugging "command not found" or an undefined
IPC field. Keep it current with the code in the same change, as AGENTS.md
asks.

## Common tasks

### Add a new top-level page

1. Create `src/pages/MyPage.tsx`. Use `<PageHeader>` + a `.page-body` wrapper.
2. Add the id to `PageId` in `store.ts` and one `NAV_PAGES` entry in
   `navigation.ts` (section, label, icon, breadcrumb extension, keywords, and
   optional shortcut key). Rail rows, TopBar crumbs, page shortcuts, and
   CommandPalette navigation rows derive from this registry.
3. Lazy-import in `App.tsx` and add a `<Show when={activePage() === 'mypage'}>`
   branch under the `<Suspense>`.
4. If the page needs a Rail badge or urgent state, add that small signal mapping
   in `Rail.tsx`.
5. Workspace conversations carry their own context, and `pageContext()`
   serves only Conversation history (Library → Conversations), so a new page needs no
   chat context builder.

### Add a new connector

1. New Rust module (e.g. `src-tauri/src/foo.rs`) that owns the I/O.
2. Public `#[tauri::command]` in the nearest `commands/` adapter that delegates
   to it.
3. Register in `lib.rs::generate_handler!`.
4. Typed wrapper in `api.ts`.

### Spawn a new agent or CLI process

From the GUI process, give a `RunSpec` `process_scope: ProcessScope::GuiOwned`
(or call `process_group::isolate` on a plain `Command`), adopt a
`process_group::GroupGuard` right after spawn, and wait only through
`process_group::drive` (streaming) or `process_group::run_to_completion`
(short CLIs). Pass time bounds as `deadline` / `limit` and cancellation as
`stop`, never as an outer `tokio::time::timeout` or a `select!` that drops the
run, and release any lease only after those return; one-shots do the same through
`claude_runner::run_oneshot_supervised`. A dropped run gets only `GroupGuard`'s
Drop cleanup (SIGKILL of its group plus an unverified async sweep), after the
caller may already have released its lease. Adopt with
`GroupGuard::adopt_run(&cmd, &child, Some(run_id))` when the run has a history
run id, so the run ledger can note its queue row if a later launch has to stop
its leftovers (R2). A synchronous `std::process` probe
uses `process_group::isolate_probe` and `end_probe_group`. Never
isolate or signal groups from MCP-subprocess code (`team_mcp.rs`,
`computer_mcp.rs`, `vault_mcp.rs`, `m365_mcp.rs`); spawns there are
`ProcessScope::Inherited`. `agent::tests::gui_run_specs_isolate_and_team_mcp_inherits`
enforces this per file, so comments in those files must not name the variants.
A run that attaches MCP servers must be built by `agent::build_command` and
adopted by a `GroupGuard`: that is what writes its private config file and
registers its process tree for the brokers' peer check (merged plan slice 6).
Never put an MCP config, a proof or a secret on argv yourself.

A new automation that needs the full-vault lease becomes a run-queue row, never
a direct `try_begin_run` call: add a `RunSource`, a claim rule in
`run_pump::decide_claim` and an admission arm in `run_pump::step` (Paperclip PR
7a). An agent CLI that runs WITHOUT the lease takes a run permit
(`run_permit::acquire`, or `claude_runner::run_oneshot_supervised`, which does
it for you); one that runs inside its own lease-holding run uses
`run_oneshot_under_lease` and must not take one.

Every agent invocation also owns exactly one usage row (Paperclip PR 6): go
through `run_collect` / `run_oneshot_supervised` with a `Meter`
(`Meter::for_vault` for a leased run, `Meter::oneshot(purpose)` for a detached
one-shot, `Meter::delegate_from_env` inside team-mcp), or, for a new streaming
launch point, `UsageRun::begin` right after admission, `set_billing(usage::classify
(kind, &cmd, true))` after the build, `launched(&cmd)` after the spawn,
`observe(&evt)` for every parsed event, and `finish(outcome)` on every exit,
BEFORE `automated_history::finish_run`. Nothing on the ledger returns an error to
propagate, so no `?` can leak the lease.

### Add a new local slash command

Add to `LOCAL_COMMANDS` in `src/components/chat/slashCommands.ts` (moved out
of `ChatBody.tsx`) and handle the case in `handleLocalCommand()` there. Return `true` to mark it as handled (it won't be
sent to claude). Don't forget the `desc` — the popup and the composer's
**Commands** menu use it. Then give it a visible route in `COMMAND_ONLY`
(`src/inventoryMap.ts`): `inventoryMap.test.ts` reads `LOCAL_COMMANDS` and fails
until it is placed, and `node scripts/inventory-reconcile.mjs` refreshes
`docs/INVENTORY_RECONCILIATION.md`.

<!-- recipe: local-slash-command -->
```sh
grep -n "const LOCAL_COMMANDS" src/components/chat/slashCommands.ts
grep -n "function handleLocalCommand" src/components/chat/slashCommands.ts
grep -n "export const COMMAND_ONLY" src/inventoryMap.ts
```

### Debug a "command not found" runtime error

You added a `#[tauri::command]` to `commands.rs` but forgot to list it in
`generate_handler!` in `lib.rs`. Add it there. (This is the most common
papercut in this codebase.)

<!-- recipe: generate-handler -->
```sh
grep -n "generate_handler!" src-tauri/src/lib.rs
```

### Debug an "undefined field" on the WebView side

The Rust type and the TS type drifted. For a generated type
(`src/generated/ipc.ts`) the drift check fails `cargo test --lib`; regenerate
it. For a hand-written one, check `types.rs` vs `types.ts` —
`serde(rename_all = "camelCase")` on the Rust side means the TS shape uses
camelCase keys (`fetched_at` Rust ↔ `fetchedAt` TS).

### Keep a runbook step checkable (recipes)

A step that names where something lives can rot when the code moves (the
slash command step above named `ChatBody.tsx` for weeks after the move). Put
an HTML comment `<!-- recipe: <id> -->` on the line right before a fenced
`sh` block of read-only checks, as above (BI7, 2026-11-18).
`scripts/runbook-recipes.test.mjs` (part of `npm run test:scripts`) finds
every recipe in AGENTS.md and `docs/` (not archived docs) and runs each
line in the checkout; any non-zero exit fails it. Only `grep`, `ls`, `test`,
`head` and `wc` run, without a shell: a pipe, redirect, glob, substitution,
absolute path or `..`, a flag carrying a path, a flag that reads a named file
or walks a tree (`grep -f`/`-r`/`-R`, `wc --files0-from`, `ls -R`), or an
operand whose symlinks lead out of the checkout refuses the whole recipe and
fails the test, so a recipe can never write or read outside the checkout. A step that changes things stays prose.

---
