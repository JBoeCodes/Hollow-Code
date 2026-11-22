# Hollow Code

Hollow Code is a native macOS desktop workspace for getting substantial work done with AI coding agents. Start with an outcome, bring the relevant files and repositories, and keep the conversation, current context, diffs, previews, and editable results together. Projects, Tasks, Library, Automations, and Settings form the main navigation.

Each workspace uses plain Markdown: a brief, plan, source manifest, local snapshots, outputs, and bounded session records. Library is a focused file list and reader. Existing folders and repositories stay in place and are linked in rather than moved. See [Workspaces and Library](docs/WORKSPACES.md) for the file contract, [What's new](docs/WHATS_NEW.md) for features, and [the build log](docs/BUILD_LOG.md) for release history.

The UI is SolidJS and TypeScript inside a Tauri 2 WebView. The backend is a single Rust process; Tauri IPC is the only application transport. There is no hosted Hollow server or hosted database.

## Product model

Local files stay canonical and operational state lives in bounded sidecar stores. External integrations are opt-in and explicit:

- **Agent CLIs:** Claude Code, Codex, Antigravity, and Grok run as installed local CLIs. Availability, sign-ins, and model defaults live in Settings → Models and Settings → Accounts.
- **Task tracking:** Optional Linear sync mirrors tracked projects and issues into local Markdown ([Tracker sync](docs/TRACKER_SYNC.md)).
- **Connectors:** Shared MCP connectors and built-in tools are managed in Settings → Connectors.
- **Microsoft 365 (optional):** Read-only Calendar and Files via OAuth 2.0 + PKCE when configured with your own Azure AD app (`HOLLOW_M365_CLIENT_ID` and `HOLLOW_M365_TENANT`).
- **Recordings (optional):** Plaud recordings can be pulled one-way through the Plaud CLI.

Model output is not trusted merely because it is well formed. Activation, learning, commitment detection, and extraction create bounded proposals that a person reviews before anything is written.

## Main workspace

- **Projects:** create an outcome workspace or attach a git repository, work alongside agents in isolated worktrees, review per-turn diffs and pull requests, run project scripts, and edit Markdown results.
- **Tasks:** focused task list and detail pane (⌘2) with optional Linear sync. [Interaction and storage details](docs/TASKS_WORKSPACE.md).
- **Library:** search and read notes, manage skills and learnings, browse conversation history, and access file recovery.
- **Automations:** repeat checked procedures inside a workspace on a schedule or trigger.
- **Settings:** Sources, Connectors, Personalization, Chat, Models, Accounts, Background work, Devices, Usage, and Advanced.

## Safety and evidence

- File writes use atomic replacement with compare-and-swap protection against external edits.
- Agent-backed Chat, Workflow, and Automation runs capture before/after filesystem history and execution evidence.
- Destructive or external actions require explicit approval according to the chat's permission mode.

## Run locally

Prerequisites:

- macOS with Xcode Command Line Tools
- Node.js 20 or newer
- Rust 1.89 or newer (toolchain pinned in `rust-toolchain.toml`)
- At least one signed-in agent CLI (Claude Code, Codex, Antigravity, or Grok)

```bash
npm install
npm run tauri:dev
```

`tauri:dev` runs Vite and the Rust Tauri shell together. `npm run dev` serves the browser mock fixture at [http://127.0.0.1:1420/?mock=1](http://127.0.0.1:1420/?mock=1).

On first launch, the setup flow initializes your Hollow folder (default `~/Hollow`) and writes configuration. Config lookup order:

1. `$HOLLOW_CONFIG`
2. `~/.config/Hollow/config.json`
3. `~/Library/Application Support/Hollow/config.json`
4. `./config.json`

Minimal configuration:

```json
{
  "vaultPath": "/absolute/path/to/your/workspace",
  "claudeProjectsPath": "/Users/you/.claude/projects"
}
```

## Verify a change

`npm run gate` runs the full local verification suite (frontend, then Rust):

```bash
npm run typecheck
npm test
npm run test:scripts
npm run build
npm run bundle:budget
(cd src-tauri && cargo fmt --check && cargo clippy --lib --tests -- -D warnings && cargo test --lib)
npm run test:smoke
git diff --check
```

## Build and install

```bash
npm run tauri:rebuild
```

Builds `Hollow.app`, verifies bundle integrity and the embedded semantic search model (`bge-small-en-v1.5`, downloaded and SHA-256 verified on first release build), and installs to `/Applications/Hollow.app` via a rollback-safe swap.

## Repository map

```text
src/
  pages/                 workspace pages
  components/            shared UI and operating-loop components
  settings/              focused Settings sections and panels
  store.ts               compatibility state facade
  store/                 runtime state and domain entry points
  styles.css             ordered stylesheet manifest
  styles/                cascade-preserving style domains
  api.ts / types.ts      typed Tauri IPC contract
  mockApi.ts             browser-smoke fixture

src-tauri/src/
  lib.rs                 process setup and explicit command registration
  commands.rs            AppState and IPC command adapters
  commands/              focused IPC adapter domains
  vault_*.rs             workspace schemas, readers, writers, and indexes
  activation.rs          proposal-only adoption scan
  review_queue.rs        shared durable review lifecycle
  connectors.rs          shared MCP connectors library
  context_rank.rs        per-request context prioritizer
  project_icons.rs       workspace app artwork and owner avatars
  backup_health.rs       conservative backup diagnostics
  m365_auth.rs / m365_mcp.rs  optional Microsoft Graph OAuth (read-only)

docs/
  README.md              feature references and manual index
  BUILD_LOG.md           chronological change log
```

Start with [AGENTS.md](AGENTS.md) for the contributor and agent manual (`CLAUDE.md` includes it directly).
