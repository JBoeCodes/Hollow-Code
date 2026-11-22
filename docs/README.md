# docs

Start with [AGENTS.md](../AGENTS.md), the agent manual's entry point, and [BUILD_LOG.md](BUILD_LOG.md), the chronological record (newest on top). The rest of the manual is in [reference/](../AGENTS.md#reference-docs), indexed in `AGENTS.md` with when to read each. Area contracts live in [agents/](../AGENTS.md#area-contracts), with the full reference-doc table in [agents/manual-and-docs.md](agents/manual-and-docs.md); `CLAUDE.md` only includes `AGENTS.md`.

Mapped visible routes, accelerators, and click-only tests: [ROUTE_LEDGER.md](ROUTE_LEDGER.md). Current advisory reachability (npm and Rust): [DEPENDENCY_TRIAGE.md](DEPENDENCY_TRIAGE.md).

## Read by code or checks (don't move or rename)

| File | Read by |
| --- | --- |
| [BUILD_LOG.md](BUILD_LOG.md) | `install_brings.rs` |
| [HOLLOW_APP_COMMAND_MAP.tsv](HOLLOW_APP_COMMAND_MAP.tsv) | `app_actions.rs` (`include_str!`): every command's area and tier |
| [FRONTEND_MODULES.md](FRONTEND_MODULES.md) | `scripts/check-module-map.mjs`: one row per top-level `src/` module |
| [reference/backend-module-map.md](reference/backend-module-map.md) | `scripts/check-module-map.mjs`: its "Backend module map" section names every top-level `src-tauri/src` module |
| [reference/](../AGENTS.md#reference-docs) and [agents/](../AGENTS.md#area-contracts) (every `*.md`, with `AGENTS.md`) | `scripts/check-claude-paths.mjs` (paths and heading links must resolve) |
| [AGENT_CONTEXT.md](AGENT_CONTEXT.md) | Rust capability-contract table test |
| [WHATS_NEW.md](WHATS_NEW.md) | bundled into the app (`?raw`) and parsed by tests |
| [INVENTORY_RECONCILIATION.md](INVENTORY_RECONCILIATION.md) | `src/inventoryMap.test.ts` (reconciled against `docs/inventory-baseline.json`) |

## Feature references

| File | What it covers |
| --- | --- |
| [ACTIVE_SESSION_MOTION.md](ACTIVE_SESSION_MOTION.md) | Session-state indicators in the rail |
| [ASK_HOLLOW.md](ASK_HOLLOW.md) | Where asking lives (Work's Ask field, command bar Ask row) |
| [ASK_YOUR_VAULT.md](ASK_YOUR_VAULT.md) | Save as note on a chat reply, and file cites in chat replies |
| [CODE_CONTEXT.md](CODE_CONTEXT.md) | `hollow_code_context`: read-only map, symbols, callers, callees, impact, read, slice, changes, and affected_tests |
| [AS_OF.md](AS_OF.md) | Nightly as-of snapshots of brief, plan, tasks, and decisions |
| [BRAND_DOORWAY.md](BRAND_DOORWAY.md), [DOORWAY_LANDSCAPE.md](DOORWAY_LANDSCAPE.md) | The doorway mark and Home panorama assets and size rules |
| [BUILD_WORKSPACES_MULTI.md](BUILD_WORKSPACES_MULTI.md) | Multiple worktrees per build workspace |
| [CANVAS.md](CANVAS.md) | Canvases: fence, sandbox and CSP, bridge, kit, store, ask-canvases, panel sections |
| [CHAT_MOTION.md](CHAT_MOTION.md) | Chat working indicator, tool timeline, reserved canvas height |
| [COMMITMENTS_LEDGER.md](COMMITMENTS_LEDGER.md) | Commitments tracking |
| [CONTEXT_LAYERS.md](CONTEXT_LAYERS.md) | The four context layers (home, personal, organization, project) |
| [CORE_WORKFLOW.md](CORE_WORKFLOW.md) | Workspace finish, output arrivals |
| [DAY_RING.md](DAY_RING.md) | The local-day dial on Today and Agenda |
| [DIRECTORIES.md](DIRECTORIES.md) | The Hollow folder: Projects and Vault setup, discovery, Library reads |
| [EMPTY_STATES.md](EMPTY_STATES.md) | Empty-view artwork |
| [EXTRACTION.md](EXTRACTION.md) | Extraction proposals from recordings and chats |
| [JEV.md](JEV.md) | Jev, the optional decision model |
| [PER_AGENT_PERMISSIONS.md](PER_AGENT_PERMISSIONS.md) | Per-agent permission menus, defaults, and chat mode retention |
| [COO.md](COO.md) | The COO page: role, Ask entry, standing orders, started chats, cap |
| [PREP_PACKS.md](PREP_PACKS.md) | Meeting prep packs |
| [PRESSURE_TESTS.md](PRESSURE_TESTS.md) | Stakeholder-simulator pressure tests |
| [RECORDS.md](RECORDS.md) | Decision and person-fact records |
| [RUNBOOKS.md](RUNBOOKS.md) | Automations and runbooks |
| [SCHEDULER.md](SCHEDULER.md) | Scheduler: windows, catch-up, night queue, gardener |
| [SOURCE_CONTEXT.md](SOURCE_CONTEXT.md) | Source and workspace-context contract |
| [TASKS_WORKSPACE.md](TASKS_WORKSPACE.md) | The Tasks page |
| [TODAY.md](TODAY.md) | The Today morning brief |
| [TRACKER_SYNC.md](TRACKER_SYNC.md) | Task tracker (Linear): types, adapter trait, scope guard, config, API key, IPC, sync engine |
| [UPDATES.md](UPDATES.md) | Update channels, Worker routes, tokens, manifest format, publish order |
| [USAGE_INSTRUMENTS.md](USAGE_INSTRUMENTS.md) | Settings → Usage graphics and attribution |
| [WEEKLY_REVIEW.md](WEEKLY_REVIEW.md) | The weekly review |
| [WORKSPACES.md](WORKSPACES.md) | Workspaces and Library |
