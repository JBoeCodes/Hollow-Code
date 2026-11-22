# Docs map

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point.

## Where records live

- [docs/BUILD_LOG.md](../BUILD_LOG.md) is the chronological record, newest on top.
- Manual docs describe the current state only.
- [docs/WHATS_NEW.md](../WHATS_NEW.md) is the user-facing log. Each `###` heading needs a `COVERAGE` row in `src/paletteCoverage.test.ts` (the palette action or Settings control that reaches it, or where it lives instead). `scripts/check-whats-new.mjs` refuses "installed yet" and "not in the installed app". What an install brings comes from git history in `src-tauri/src/install_brings.rs`.
- Every capability's visible click paths, accelerators, and click-only test live in `src/routeLedger.ts`, rendered to [docs/ROUTE_LEDGER.md](../ROUTE_LEDGER.md) by `node scripts/route-ledger.mjs`; `src/routeLedger.test.ts` fails on a palette action, `g` target, or shortcut that reaches no visible route.
- `npm run test:scripts` runs `scripts/check-claude-paths.mjs`: every repository path named in `AGENTS.md`, a `docs/agents/` doc, or a reference doc must exist, every link to a heading in them must resolve, `AGENTS.md` stays under 15,000 bytes and links each of those docs, and no doc outside the build log links `CLAUDE.md#...`.

## Reference docs

| Doc | Read it when | What it holds |
| --- | --- | --- |
| [areas](../reference/areas.md) | you need the doc for one area (projects, tasks, chat, context, phone, and more) | Short pointers by area |
| [product](../reference/product.md) | before changing a surface: what each part of the app does now | Current shape: rail, workspaces, Results, Waiting on you, Preview pane and project files, build workspaces, COO, and more |
| [running-and-ci](../reference/running-and-ci.md) | you need a command beyond the short list, the gate's exact steps, or CI's lanes | Command table, smoke timing under load, CI workflows, pre-release build lane, smoke notes, gate time |
| [build-and-ship](../reference/build-and-ship.md) | building, installing, or recording an install; versions, signing, config, disk guard, diagnostics | Build & ship, Updates, In-app installer, Privacy grants after an update, Self-install CI compatibility |
| [architecture](../reference/architecture.md) | adding a lock, command, window, or tray item; touching startup, IPC, or performance | Lock order, Performance profile, IPC contract, Command registration, Adding a new command, AppState, Windows, Tray and global shortcuts, Microsoft 365 |
| [chat-stream-grammar](../reference/chat-stream-grammar.md) | changing chat event order or adding an event type | MUST rules (G01 to G10) for one turn's event stream, `chat_grammar.rs`, and `tests/fixtures/chat-streams/` |
| [backend-module-map](../reference/backend-module-map.md) | finding the Rust module that owns something; adding or removing a module | Backend module map |
| [frontend-state](../reference/frontend-state.md) | store, chat, page context, async stale-response guards (`latestRequest`) | State, Chat, Preview/web, Workspace context, ActivityDock + trackTask, Bell/toast policy |
| [styling](../reference/styling.md) | touching CSS, tokens, themes, chrome, or motion | Styling, Tokens ([notices](../../THIRD_PARTY_NOTICES.md)), The cascade, Custom themes, Page chrome and shared primitives, App shell, Shared motion kit, Visual contracts |
| [surfaces](../reference/surfaces.md) | changing one of the feature surfaces it lists | Tasks and Automations density, Settings sections, Companion apps, Access and trust (phone), Waiting on you, Key hints, Push-to-talk, Ask, Canvas gallery, Library tabs, Mission Control, COO presence |
| [vault](../reference/vault.md) | changing vault schemas, sync, the watcher, learnings, evidence, recovery, or task files | Vault mirrors and source of truth, Per-tab folder schemas, Sync direction, Catching up a stale workspace, Stable URLs, File watching, Trusted learning, evidence and recovery, Tasks & Projects |
| [runs](../reference/runs.md) | changing usage, budgets, the run ledger, results, resume, or run diagnostics | Usage ledger, Run ledger v2, Friction to fix loop, Flight recorder, Step spine, Usage budgets, Mid-run enforcement, Result kinds, Resume, Per-run scratch space, Silent-run watchdog, Phase timings |
| [sidebar-interactions](../reference/sidebar-interactions.md) | sidebar drag, row details, file drops | Moving, Details |
| [keyboard-shortcuts](../reference/keyboard-shortcuts.md) | adding or changing a shortcut | Keyboard shortcuts |
| [agents](../reference/agents.md) | changing agent CLIs, context, models, CLI updates, skills, or Claude accounts | Chat integration, Automatic account rotation and quota recovery, Self-hosted models, Skill-import rename |
| [interrupts](../reference/interrupts.md) | touching question cards, approval cards, or their ids, or adding a phone answer route | Interrupt record and answer shape (`interrupt.rs`, fixtures in `src-tauri/fixtures/interactions/`), `apr-` ids, expiry, deny by default, subagent cards, Decline reasons |
| [agent-tools](../reference/agent-tools.md) | changing permission modes or any tool Hollow gives agents | Handoff record, Computer control ([what it does now](../COMPUTER_CONTROL.md)), Messaging is unavailable, Ask your vault, Owned MCP preapproval, Broker proofs, Permission modes, Task tools, Hollow app tools, Refusal codes, Listening ports, Question cards, COO parity |
| [peers](../reference/peers.md) | the peer link, reconnect rules, stream ([phone API](../PHONE_CHAT_API.md)), routed turns, offline copies | Commands, Hub, Stores, UI |
| [connectors](../reference/connectors.md) | changing connectors, the context prioritizer, or onboarding | Connectors library and context prioritizer, Onboarding and directory setup |
| [automation](../reference/automation.md) | changing automations, the run queue, routines, or the heartbeat | Background runner, routines and heartbeat, Event triggers, Launch at login, Scheduled routines, Workflow permission stamps, Automation queue, Failure streak, Skips and retry, Precheck command |
| [common-tasks](../reference/common-tasks.md) | adding a page, connector, slash command, or process spawn; debugging IPC | Add a new top-level page, Add a new connector, Spawn a new agent or CLI process, Add a new local slash command, Debug a "command not found" or "undefined field" |
| [quirks](../reference/quirks.md) | touching runs, processes, MCP, Jev, the CSP, artifacts, video playback, canvases, or PLAN.md ticks | Runs and processes, Frontend and build |
| [glossary](../reference/glossary.md) | a project term is unclear, or which home a concept lives in | Everyday product model, Glossary, Artifact-to-session continuity |

Other entry points: [docs/README.md](../README.md) indexes the per-feature references in `docs/`; frontend modules are mapped in [docs/FRONTEND_MODULES.md](../FRONTEND_MODULES.md); agents' code lookups (`hollow_code_context`, CLI `hollow code-context`) are in [docs/CODE_CONTEXT.md](../CODE_CONTEXT.md); what reaches an agent per entry point is in [docs/AGENT_CONTEXT.md](../AGENT_CONTEXT.md).
