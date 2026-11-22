# Where each area is documented

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Where each area is
documented" section, moved here word for word on 2026-11-17 to keep CLAUDE.md
under its size cap (only relative links were re-based). Keep it current with
the code in the same change, as AGENTS.md asks.


Short pointers, by area. Per-surface contracts that used to sit here are in
[Surface notes](surfaces.md#surface-notes-moved-from-claudemd-2026-11-13),
[Visual contracts](styling.md#visual-contracts-moved-from-claudemd-2026-11-13)
and [Smoke notes](running-and-ci.md#smoke-notes-moved-from-claudemd-2026-11-13).

- **Projects and Home:** [Chat-first Home](../WORKSPACES.md#chat-first-home);
  modules and commands in [Workspaces and Library](../WORKSPACES.md#navigation-and-implementation); Archive older entries… (menu) and
  `workspaces.archive_shared_docs` (dry run, hash-bound apply) are `context_compact.rs` ([product](product.md)).
- **Tasks and the tracker (Linear):** [sync](../TRACKER_SYNC.md),
  [agent work](../TRACKER_SYNC.md#tracking-agent-work),
  [audit](../TRACKER_SYNC.md#the-audit-record),
  [screens](../TRACKER_SYNC.md#hollows-screens-for-the-tracker),
  [Friday status](../TRACKER_SYNC.md#status-and-canvas-copies),
  [phone](../phone-api/tracker.md), [Linear link](../TRACKER_SYNC.md#linear-link);
  task drafts in [Tasks](../TASKS_WORKSPACE.md); page contracts in
  [Tasks and Automations](surfaces.md#tasks-and-automations-page-density-p4).
- **Automations:** [drafts](automation.md#automation-drafts),
  [event triggers](automation.md#event-triggers-f2) (they start
  an automation or wake a lead), [the queue](automation.md#automation-queue-paperclip-pr-7a)
  (chats at once, subagents per chat), [run state and Last night](automation.md#automation-run-state-and-last-night).
- **COO** (the Main agent until 11-10, L6; ids stay `main`/`main-agent`): one
  surface, its page, and [one conversation](../COO.md#one-conversation-2026-11-07),
  opened with ⌘⇧M (`main-drawer`). It runs on any agent and delegates by
  default ([Provider and model](../COO.md#provider-and-model),
  [leads](../COO.md#project-leads-l8), [presence](../COO.md#coo-presence),
  [subagent view](../COO.md#subagent-work-view)).
- **CFO** (`cfo/`): [agent, model, effort and account per turn; budgets](../CFO.md).
  A chat's own effort lives in Rust (`session_store` `chat_efforts`).
  Quota recovery: [guards and notices](../QUOTA_FAILOVER.md).
- **Settings:** thirteen sections; see [Settings sections](surfaces.md#settings-sections-p1-22-d4-s2).
- **Canvases:** [Canvases](../CANVAS.md), including dither (chart fills are opacity-only cells from `canvas/kit/ditherFill.ts`, kit budget 99 KiB; hover repaints a cached data layer) and
  [pinned live canvases](../CANVAS.md#pinned-live-canvases-overnight-2026-10-1213-f1);
  [Canvas gallery](surfaces.md#canvas-gallery-overnight-10-1112-f3).
- **Chat motion** (working SVG, tool timeline, reserved height): [Chat motion](../CHAT_MOTION.md).
- **Design:** [Design system](../DESIGN_SYSTEM.md) (3.0, shared glyph
  artwork, [experimental visuals](../DESIGN_SYSTEM.md#experimental-visuals)),
  [styling](styling.md), [Empty views](../EMPTY_STATES.md).
- `docs/DESIGN_LANGUAGE.md`: design rules N36 to N49; read before UI work.
- **Custom icons** (Upload an icon, Generate a logo, validation, Dock
  restoration): [Custom icons](../CUSTOM_ICONS.md).
- **Meeting prep:** [PREP_PACKS.md](../PREP_PACKS.md).
- **Changes, quit and installs:**
  [Changes freshness](build-and-ship.md#changes-freshness),
  [Quit and installs](build-and-ship.md#quit-and-installs).
- **View recovery** (page and section error owners): [View recovery](frontend-state.md#view-recovery).
- **Smoke fixtures and timing:** [Running, testing and CI](running-and-ci.md).
- **Code mode** (CM1a sandbox, CM1c served): `codemode.rs` ([module map](backend-module-map.md)). The `run_script` tool (action `codemode.run`, experimental, read tier) is first-class in `app_mcp.rs` (so Claude and Codex over the shared HTTP MCP and Grok over the `hollow app-mcp` stdio twin all get it), and `app_broker::dispatch` runs it with the caller, asker and policy; every inner call re-authorizes on its own tier and card. `run_effects.rs` counts a script as a change (keyed on the script) when its report lists a non-read call.
- **Code context** (CC2): `code_context.rs`, `code_context_mcp.rs`, `code_index/` ([module map](backend-module-map.md)). The `hollow_code_context` server (Claude and Codex in a git checkout, Settings toggle `codeContextMcpEnabled`) and its CLI twin `hollow code-context` for Grok and Antigravity; [CODE_CONTEXT](../CODE_CONTEXT.md).
- **Frontend modules:** [module map](../FRONTEND_MODULES.md); `ChatBody` owns the lazy persistent summary and title-menu tools.
- **Scan rules:** `vendor/ctx-scan/` (re-vendor from `org-context`, never edit), `scan_rules.rs`: [rules](vault.md); [leak checks](../DIRECTORIES.md#leak-checks-cx10-2026-11-15) (CX10).

Moving the vault and Projects into one Hollow folder (dry run, run, rollback; iCloud moves by copy;
append-only `audit_log` keeps old workspace ids, so readers by workspace id resolve through `workspace_aliases.rs`):
[Move Hollow home](../DIRECTORIES.md#move-hollow-home-2026-11-12).

Context layers (home, personal, org, project): [CONTEXT_LAYERS.md](../CONTEXT_LAYERS.md). Setup brief: `src/contextSetupBrief.ts`.
Clone roots, the machine slug (`~/.config/hollow/machine.json`), no-symlink path rules
(`context_roots.rs`): [Context roots](../DIRECTORIES.md#context-roots-cx3-2026-11-13). A new
session's profile block carries `home/<slug>.md` (4 KB) and the org `INDEX.md` (2 KB); the org
clone fast-forwards every 30 min (`context_layers.rs`, read only). Retrieval kind `org`:
[Context layers](../DIRECTORIES.md#context-layers-read-side-cx4-cx5-2026-11-14).
Claude turns get `autoMemoryDirectory` (CX4b). Personal folders sync by `sync_engine.rs`
([guide](../DIRECTORIES.md#context-sync-engine-cx6a-2026-11-14)); vault root:
[local git, off](../DIRECTORIES.md#vault-root-cx6b-2026-11-15); clashes `sync_clash.rs`; binaries `sync_files.rs` (V4).
Org changes only via `context.propose_org_change` (External; a `gh` PR; an attended owner chat
merges once checks pass, others wait) and `context.merge_org_change` (`context_propose.rs`, CX8);
nightly [digest](../DIRECTORIES.md#org-changes-digest-cx8b-2026-11-15). CX9: the extractor sends a machine fact to `home/<slug>.md` (`memory_homes::save_home_fact`), a Acme fact to a pending org proposal (`queue_org_fact_at`, unwritten); About you reads the personal repo.
Settings → Sources → Context (`context_settings.rs`, CX17):
[Settings, Context](../DIRECTORIES.md#settings-context-cx17-2026-11-15).

The company code went 11-10 (L11); its tables stay, inert. Never drop them.
