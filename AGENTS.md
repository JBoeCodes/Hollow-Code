# AGENTS.md

The agent manual for this repository. `CLAUDE.md` only includes this file (`@AGENTS.md`). Detail lives in the docs mapped under [Reference docs](#reference-docs) and [Area contracts](#area-contracts): read an area's doc before changing it. Keep this file current-state only and under 15 KB.

## What this app is

**Hollow Code** (Hollow) is a native desktop workspace for substantial work with agents: a Rust (Tauri 2) shell embedding a SolidJS + TypeScript WebView, macOS-first, also built for Linux. The sidebar's pages are Projects (formerly Work), Tasks, Library, and Automations, with Settings at the bottom; Needs you has no sidebar row and opens from the bell's See all or ⌘5 (`src/navigation.ts`). Each workspace owns a Markdown brief, plan, sources, outputs, and session records; local files stay canonical. Tasks and projects can sync with a configured Linear team (shared plus optional personal team); Notion is a connector or one-time importer. Agents: Claude Code, Codex, Antigravity, and Grok, through one app-wide agent/model preference. The COO (formerly Main agent) is one page and runs on any installed agent.

Current direction: outcome workspaces, a simple Library, and local plain-file context; standalone Email, Slack, Teams, and Calendar pages stay retired. The [source contract](docs/SOURCE_CONTEXT.md) and [product](docs/reference/product.md#what-this-app-is) describe the current shape.

## Settled owner decisions

Enforced in code. Full wording: [product](docs/reference/product.md#what-this-app-is).

- **Bypass asks nothing:** no Hollow approval or confirm card appears in `bypassPermissions` (question cards and task suggestions still show). Exception: `app_actions::ALWAYS_ASK` (standing-orders edits, the gardener's tidy-up, settling a decision record, `friction.draft_fix`, `build.install_built_app`, `build.publish_main`, `settings.cfo_approve_extra_usage`, `personas.accept`) asks on a card in every mode and never runs from an automation or a COO check-in. A destructive or access-granting `tracker.graphql` mutation asks in every mode.
- **Configured tracker writes ask no card** (`app_actions::TRACKER_WRITES`, Write tier, automations included); teams outside the configured allowlist are refused. Keep `mode_contract`'s bypass wording and its fixture in step.
- **Agent work tracking** (`tracker.trackAgentWork`: off, dryRun by default, on): waiting items and commitments can mirror to configured tracker issues.
- **Auto asks only** before leaving this Mac, deleting, touching secrets or accounts, or giving agents more power.
- **Services and delegation:** services are bounded in every mode except bypass. Delegates are bounded by their parent: allowed tools intersect, denied tools accumulate, a deny wins; never widened.
- **Human review:** only a person accepts a Result or resolves review items, commitment and learning proposals, and the profile builder; agents only propose or mark ready.
- **History bounds:** History keeps at most 200 runs, 90 days, and 512 MiB and captures files up to 2 MiB; a run with an uncapturable file never claims to be fully recoverable.
- **Quota failover:** a hard budget ceiling never stops work silently; work may continue on another account or agent with room, and the switch shows in the chat and on the receipt.
- **Messaging:** Hollow has no built-in email, Slack, or Teams; an agent uses a messaging service only through a connector the user shared. Microsoft 365 is read-only (calendar, files) when configured via `HOLLOW_M365_CLIENT_ID` and `HOLLOW_M365_TENANT`.

## Running the app

Rust 1.89+ (pinned to 1.96.0 in `rust-toolchain.toml`; bump it on purpose, in its own change) and Node.js 20+. Full text: [Running the app](docs/reference/running-and-ci.md#running-the-app).

- `npx tauri dev` is the only end-to-end entry point. In a plain browser (`npm run dev`) Tauri's `invoke()` is undefined; `/?mock=1` (only `npm run dev` and `npm run build:mock`) serves the `mockApi.ts` fixtures for Playwright. Import `mockMode.ts`, never `mockApi.ts`, to ask "am I mocked?".
- Run `cargo fmt` in `src-tauri/` before committing Rust; `cargo clippy --lib --tests -- -D warnings` must pass. Never add a line to `Cargo.toml`'s `[lints.clippy]` baseline to get a change through: fix the sites and drop the line. Dead code warns; a helper only tests use is `#[cfg(test)]`; an item kept on purpose has a targeted `#[allow(dead_code)]` with its reason. Source-text pins go through `crate::source_pins`.

### The commands you need on most tasks

| Command | Use |
|---|---|
| `npm run gate` | The full check: `gate:frontend` then `gate:rust` |
| `npm run typecheck` / `npm test` / `npm run test:scripts` | The frontend halves one at a time |
| `cargo fmt` / `cargo clippy --lib --tests -- -D warnings` / `cargo test --lib` | The Rust half, in `src-tauri/` |
| `npm run test:smoke` / `npm run smoke:critical` | Playwright over `?mock=1`, only with no other dev server up |
| `CARGO_TARGET_DIR="$HOME/Library/Caches/hollow-release-target" npm run tauri:rebuild` | Build, install, verify, and clean up the production app |

Every other command (agent-eval, workbook recalc, perf, kit): [running-and-ci](docs/reference/running-and-ci.md).

### Gate and install

- `npm run gate` runs `gate:frontend` (version check, typecheck, vitest, `test:scripts`, build, bundle budget) then `gate:rust` (`cargo fmt --check`, clippy, `cargo test --lib`), in the `America/New_York` time zone. CI runs only on pull requests. `CARGO_TARGET_DIR` must be absolute with no `~` segment (`scripts/check-cargo-target.mjs` refuses one).
- Release builds are numbered `0.6.<first-parent commit count>`; never bump versions by hand. Details: [Build & ship](docs/reference/build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app).

## Data safety

Full text with code pointers: [Quirks to know about](docs/reference/quirks.md#quirks-to-know-about).

- Never re-render an existing task, project, or workflow file: edit through `vault_tasks::patch_task`, `frontmatter::patch`, or `patch_workflow`; task steps through `vault_tasks::splice_subtasks`; idempotent creators use `vault_tasks::create_idempotent`.
- A read-modify-write of a vault text file goes through `vault::write_merging` (or `replace_if_unchanged`), never read then `atomic_write`. Never write a watched dot-state file with `atomic_write` directly: use `run_guard::write_host_state`.
- A write on a run's behalf goes through `automated_history::with_live_run`; never authorize with `lease_holder` and then write.
- Never write tasks or `.review-queue.json` from an MCP subprocess; proxy writes to the GUI broker.
- State stores fail closed: never read a damaged store as defaults or overwrite it.
- Workflow vault writers hold `workflow_permissions::write_guard()`; seed test workflows with `vault_workflows::seed_workflow(_with)`.
- Found context is accepted one item at a time. Never prune `refs/hollow/checkpoints/` or `refs/hollow/branches/` by hand. Hollow never writes `~/.claude`.
- Never drop inert legacy schema tables in existing databases. New readers of audit, usage, or history rows by workspace id resolve through `workspace_aliases.rs` ([data](docs/agents/vault-and-data.md)).
- Dev runs are not sandboxed (a temp `vaultPath` does not isolate one): use the mock harness or isolated tests. Native quit checks never terminate a running Hollow instance.

## Code rules

- Components reach the backend only through the typed `api` in `src/api.ts` (or a lazy typed API module), never `invoke()`. The backend owns all I/O.
- CSS rules live in cascade layers (`css:check`). New motion keeps functional content visible on its first frame and obeys both reduced-motion gates.
- A new accelerator gets a row in `src/routeLedger.ts`; regenerate the inventory after adding interactive views or recovery controls.
- A user-facing feature gets its [What's new](docs/WHATS_NEW.md) entry and a `COVERAGE` row in `src/paletteCoverage.test.ts` in the same change.
- A new or removed top-level `src-tauri/src` module changes the [backend module map](docs/reference/backend-module-map.md); a new top-level `src/` module changes [docs/FRONTEND_MODULES.md](docs/FRONTEND_MODULES.md) (`scripts/check-module-map.mjs`).
- Keep every chat provider-neutral and preserve the recorded identity of existing conversations.
- Brief-wording and visual-guide edits regenerate both prompt fixtures (`HOLLOW_REGENERATE_PROMPTS=1` with `cargo test --lib context_budget` and `cargo test --lib probe_prompts`, from `src-tauri`).
- Rust tests use unique temp folders, hold at any hour or time zone (`test_clock::clear_of_midnight`), share no `static`, hold a `flock` as `flock::HeldLock`, use `#[ignore = "why"]`, and add no sleep, clock read, or tight wall-clock bound (`scripts/check-test-sleeps.mjs`, `scripts/check-timing-asserts.mjs`).

Update `AGENTS.md`, the area's reference doc, and a newest-first [docs/BUILD_LOG.md](docs/BUILD_LOG.md) entry with each substantive change.

## Reference docs

When to read each and what it holds: [the full table](docs/agents/manual-and-docs.md#reference-docs).

- Product and running: [areas](docs/reference/areas.md) (pointers by area), [product](docs/reference/product.md), [running-and-ci](docs/reference/running-and-ci.md), [build-and-ship](docs/reference/build-and-ship.md), [common-tasks](docs/reference/common-tasks.md), [quirks](docs/reference/quirks.md), [glossary](docs/reference/glossary.md).
- Code structure: [architecture](docs/reference/architecture.md), [backend-module-map](docs/reference/backend-module-map.md), [docs/FRONTEND_MODULES.md](docs/FRONTEND_MODULES.md), [chat-stream-grammar](docs/reference/chat-stream-grammar.md), [frontend-state](docs/reference/frontend-state.md), [styling](docs/reference/styling.md).
- Surfaces and data: [surfaces](docs/reference/surfaces.md), [sidebar-interactions](docs/reference/sidebar-interactions.md), [keyboard-shortcuts](docs/reference/keyboard-shortcuts.md), [vault](docs/reference/vault.md), [runs](docs/reference/runs.md), [automation](docs/reference/automation.md).
- Agents: [agents](docs/reference/agents.md), [agent-tools](docs/reference/agent-tools.md) (permission modes, tools), [interrupts](docs/reference/interrupts.md), [peers](docs/reference/peers.md), [connectors](docs/reference/connectors.md).

Also: [docs/README.md](docs/README.md) (feature index), [docs/AGENT_CONTEXT.md](docs/AGENT_CONTEXT.md), [docs/CODE_CONTEXT.md](docs/CODE_CONTEXT.md), [repository layout](docs/agents/repository-layout.md).

## Area contracts

How each area works and the invariants it keeps; read yours first.

- [chat](docs/agents/chat.md): composer, transcript, conversation state, chat search, file links.
- [shell-and-navigation](docs/agents/shell-and-navigation.md): sidebar, windows, palette, notifications, Preview pane.
- [work-and-home](docs/agents/work-and-home.md): welcome, Work, Home, Running, project notes.
- [library-and-canvases](docs/agents/library-and-canvases.md): Library, canvases, the canvas kit and texture.
- [settings-tasks-automations](docs/agents/settings-tasks-automations.md): also connector setup and calendar.
- [coo](docs/agents/coo.md): the COO page.
- [visual-design](docs/agents/visual-design.md): design system, themes, motion, icons.
- [agents-and-run-queue](docs/agents/agents-and-run-queue.md): accounts, quota, shared hosts, admission, queue, run records.
- [learning-and-memory](docs/agents/learning-and-memory.md): learning capture, memory, shared skills.
- [tracker-and-phone](docs/agents/tracker-and-phone.md): Linear tracker sync and phone send settings.
- [vault-and-data](docs/agents/vault-and-data.md): data kept on purpose, workspace ids, directories.
- [manual-and-docs](docs/agents/manual-and-docs.md): the reference-doc table, where the build log and What's new live.
