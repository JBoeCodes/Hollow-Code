# Agent integration and Claude accounts

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Chat: how the headless
agent integration works" (its opening part) and "Claude accounts", moved there
word for word on 2026-10-13 (only relative links were re-based). Read it
before changing how an agent CLI is launched, what context a turn gets, models
and preferences, one-shots, skills, or Claude accounts and failover. The tool
and permission subsections of the same section are in [Agent tools and
permissions](agent-tools.md). Keep it current with the code in the same
change, as AGENTS.md asks.

## Chat: how the headless agent integration works

**Agent PATH (`agent::cli_path`, 2026-10-16).** Every agent CLI starts with Hollow's tool folders
(`~/.cargo/bin`, `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`), then the login shell's PATH, then the
inherited PATH, then the system folders, each folder once. The login shell's PATH (`shell_path.rs`) is read once at
launch on its own thread: `$SHELL` if `/etc/shells` lists it, else `/bin/zsh`, run as `-ilc` in its own session
with stdin/stderr closed, `TERM=dumb`, cwd home and a 5 s limit; it prints the PATH after a `__HOLLOW_PATH__=`
marker and only absolute folders are kept. Until it answers, and when it hangs, fails or prints no marker, agents get
`self_install::tool_path(inherited)` exactly as before; nothing waits for it. The log line `[shell-path] read N
folders from <shell> in X ms` (or the reason) never prints the folders. `is_available`/`find_binary` scan the same
PATH. A chat turn that can take a build slot (every mode but plan, Antigravity only in bypass,
`build_slots::can_take_slot`; a build workspace's in every mode) puts the `hollow-slot`
folder in front of this PATH (`chat::build_workspace_env`; only build workspaces before OF07, 10-17; before
10-16 it used the bare inherited PATH, so a Dock-launched Hollow's build agents lost Homebrew and cargo).

**Seeing that PATH (A12, 2026-10-16).** `shell_path::merge` builds it folder by folder, each tagged `hollow`,
`shell`, `inherited` or `system` (the first source to list a folder keeps it), and `cli_path` joins that list, so
what Settings shows and what agents get can't drift. The read-only command `agent_tool_path`
(`commands/system.rs`; app action `system.agent_tool_path`) returns `{ entries: [{ dir, from, exists }], shell:
{ state: read|pending|failed, detail } }` from one look at the shell's read. Settings → Advanced → **Folders agents
can run tools from** (`settings/AgentToolPath.tsx`, words in `agentToolPath.ts`) lists it with each folder's
source, missing folders muted and marked, a Copy button for the joined PATH, and a plain note when the shell's
PATH is still being read or couldn't be (with the reason). It's the first place to look when a tool is "command
not found" for an agent but works in Terminal. The browser mock shows the failed note with
`localStorage.mockAgentToolPath = 'failed'` (or `'pending'`).

`agent.rs` adapts the installed **Claude Code** (`claude`), **Codex** (`codex`),
**Antigravity** (`agy`) and **Grok** (`grok`) CLIs to one event vocabulary. `agent_settings.rs`
resolves every run: explicit agent/model → saved app preference → first installed
runtime. An explicitly chosen unavailable agent or invalid model pairing fails;
it never falls through to another provider. Empty explicit model means runtime
default, while an absent model inherits that provider's saved model. Model IDs
stay free text underneath, but every model field is now a `ModelPicker` menu
(2026-09-26, `agent_models.rs`, IPC `agent_models(agent, refresh?)`). Every list
comes from the agent itself, so a new model shows up without a Hollow change
(2026-10-13):
- **Claude**: Claude Code's own `initialize` answer over stream-json (the
  handshake the Agent SDK's `supportedModels()` reads): `claude -p
  --input-format stream-json …` with only a `control_request` `initialize` on
  stdin, hooks off (`--settings {"disableAllHooks":true}`), no MCP servers, no
  session saved. No prompt is sent, so it costs nothing (about 1 s on 2.1.296).
  Each alias comes with `resolvedModel`, so `sonnet` is labelled "Sonnet 5.5"
  because that is what it resolves to today; Hollow adds the documented
  `opus`/`fable`/`sonnet` `[1m]` forms and `opusplan`, and pinned IDs read
  "· pinned". `--bare` was rejected: it skips the login and returns a reduced
  list. When Claude Code doesn't answer, the last list it gave
  (`agent-model-cache.json` in app data) then `builtin_claude_models()` stand
  in, and the picker's line says which.
- **Codex** reads its own `models_cache.json` (hidden entries skipped); when
  that is missing or over a day old (its `fetched_at`), `codex debug models`
  has the CLI refresh it first (30 s bound), falling back to the old file with
  a warning. What it printed is reused for a day, in case the CLI refreshes
  without rewriting `fetched_at`.
- **Antigravity** runs `agy models`, **Grok** `grok models`.
  Antigravity lists **one row per base model** (`gemini-4-neon`), not one per
  `…-low`/`…-high` variant: `agent_models::group_agy` groups them, and `ModelOption.levels`
  carries what the base takes (agy 1.5.13: Neon low to xhigh, Argon, Carbon and Hydrogen low
  to high, 3.1 Pro low and high). Hollow's one Effort control picks the variant at run
  time (`effort::resolve`, nearest level the model has); the composer's Effort list is
  narrowed to the chat model's levels (`modelMenu.levelsForModel`). agy 1.5.13 also
  takes `--model <base> --effort <level>` and refuses a level the model lacks, and a
  suffixed `--model` beside a different `--effort` is an error, so Hollow sends the variant
  id and no `--effort`. A saved variant id (default model, CFO allow-list, a chat) reads as
  its base: `agent_settings::migrate_antigravity_model` (also lends the level as the agent's
  effort when none is saved), `cfo::canon`, `modelMenu.catalogModel`.

  **Antigravity usage (agy 1.5.13, probed 2026-11-20).** Each finished stream-json step carries
  `usage` (`input_tokens` uncached, `cache_read_tokens`, `output_tokens` with thinking). Each
  becomes one `MessageUsage` (context gauge, `reportsContext` is true for every agent now), and a
  turn's figure is the sum of its own steps (`usage::antigravity_turn_usage`): the `result` block is
  session-cumulative on `--conversation` and is never read. No cost comes from agy, so the ledger
  prices it `estimated` at Google's published Gemini rates (`usage_prices::gemini_rows`, keyed by
  base model; Argon and Carbon rise 2026-12-01, 3.x Flash 2027-01-01, 3.7 Flash unlisted so
  unpriced). Budgets: tokens only, like Codex (`budgets::can_report`). The prepaid AI Studio credit
  balance is not exposed by agy: the usage menu links `ANTIGRAVITY_CREDITS_URL`.

Lists are cached in the backend for 10 minutes (1 minute when they carry a
warning), one fetch per agent at a time; the picker's Refresh passes
`refresh: true`. The picker shows the source and when it was checked
("Claude Code 2.1.296's model list · checked 12:40"), or the warning.
"Custom model ID…" accepts anything, and a saved ID the list doesn't contain
shows as custom, unchanged; nothing validates or rewrites the ID. Hollow
retains each CLI's own sign-in.

**Defaults that name a model follow the newest of their family.**
`agent_models::newest_claude(family)` returns what Claude Code last resolved
that alias to (this session, else the saved list, else the built-in table).
The extraction default on Claude (`extract::default_claude_model()`) and the
swarm default (`main_agent::default_swarm_model()`; it applies until the person saves any
COO setting, which stores the ID it showed) use the newest Haiku, pinned
to its exact ID so run history and prices name the model; `DEFAULT_CLAUDE_MODEL`
/ `SWARM_MODEL` remain only as the fallback. A model the person saved stays as
saved: `ModelCatalog.newestByFamily` lets Settings flag a pinned Claude ID whose
family has a newer model (`NewerModelHint`: "A newer Sonnet is out:
claude-sonnet-5-5 · Use it") under every model picker, the extraction model and
the swarm model. Left alone on purpose: `claude_accounts::PROBE_MODEL` (a
1-token quota probe where the model doesn't matter), and model names in prose
(the COO's prompt and app-action summaries).

**Agent updates** (`agent_cli.rs`, Settings → Accounts → Agent updates,
2026-10-13). Per agent CLI: installed version (`--version`, remembered per
resolved binary and modified time), latest, how it was installed, last checked,
Update, Restore and an Updates choice (`agent-cli.json` in app data;
settings keys `agents.updates.<agent>`, Confirm). Install methods come from the
binary's resolved path, and each updates only with its own tool:

| Install | Latest from | Update | Restore |
|---|---|---|---|
| Claude Code native (`~/.local/share/claude/versions/`) | `downloads.claude.ai/claude-code-releases/<autoUpdatesChannel>` | `claude update` | `claude install <version>` |
| npm global (`<prefix>/lib/node_modules/<package>`) | `npm view <package> version` | `<prefix>/bin/npm install -g <package>@latest` | same, `@<version>` |
| Homebrew (`Cellar/`, `Caskroom/`) | `brew info --json=v2` | `brew upgrade [--cask] <name>` | not offered |
| Grok's installer | `grok update --check --json` | `grok update` | `grok update --version <version>` |
| Antigravity's installer | can't check without updating | `agy update` | not offered |
| anything else | — | not offered ("update it the way you installed it") | — |

**Versions known to break Hollow** (repo ideas 2026-10-27 #9): a short list
shipped inside Hollow, `src-tauri/src/agent_cli_known_bad.json` (`agent`,
exact `version`, one-sentence `reason`; nothing is fetched; its schema is a
test, `agent_cli::tests::the_known_bad_list_ships_valid`). Warn, never block:
an installed listed version gets a warning line with its reason
(`AgentCliStatus.knownBad`); a listed latest version (`latestKnownBad`) gets one
too, Update and Update now ask "Update anyway" first, the `agents.update_cli`
card adds the warning (and the installed version's, from the version cache),
and Automatic doesn't install it by itself (Update still does). A waiting
Update remembers the version it was asked for (`pendingFor`): if a listed
version came out meanwhile it stops and asks for Update again. The list ships empty: no version is known to break Hollow today
(openbot's own list is empty too). Add an entry when a release breaks Hollow
and remove it once Hollow copes.

Updates are Automatic by default (the owner asked for "as needed"), Ask first,
or Off. `start_worker` (GUI, launch + 2 min, then every 5 min) checks each
installed agent once per 20 h; Automatic updates only while `runs_using(kind)`
is 0, Ask first posts one notice per new version, and a waiting Update (clicked
while runs use the agent, or while a check was running) runs once they finish.
Automatic leaves a version alone that failed to install (until the next daily
check) or that the person restored away from (until a newer one; Update clears
it); `AgentState.skip`/`skip_until`. Antigravity, which can't
check, runs its updater once a day under Automatic. In use = agent processes
whose run tag is still live in the process ledger (`agent::build_scoped` calls
`agent_cli::admit`; `orphans::Ledger::live_tags`), counted for 2 min before
adoption, chat turns in `agent_sessions`, or any process running from that
install (`ps -axww -o args=`: the program, by its path or by the bare name it was typed
as in Terminal (`grok --resume`: a first word with no `/` that is one of the install's
`package.json` `bin` commands or the located binary's file name; first word only, and a
different program of that name counts too, which only makes an update wait), and, for a
`node` wrapper, its script past any options (e.g. `node --max-old-space-size=…` then the script), each resolved,
a path with spaces read across words; for `bunx`/`bun x` the install's package name
(also as `-p`/`--package <pkg>`) or one of its `package.json` `bin` commands, and for
`bun run` (or its shorthand `bun <command>`) one of those commands (`agent_cli::count_in`); a listing `ps`
couldn't give whole counts as in use; catches teammates inside
`hollow team-mcp` and Terminal sessions). Claude Code's native install counts
Hollow runs only (a new version installs beside running ones, so a Terminal or
bot session doesn't hold it back). It is checked again after the update lock is
taken, so a run that started in between makes the update wait; a Restore that
meets a busy agent asks to try again rather than turning into an update. While one agent updates,
`agent::build_scoped` refuses its new runs with a plain sentence
(`agent_cli::admit`, which also registers the run, both under one lock); an
`agent-cli-updating-<agent>` marker in app data (expires after 11 min) tells
other Hollow processes, such as `hollow team-mcp` starting a teammate, the same.
Settings gets one `ps` listing per refresh for all four agents. Claude Code's native install alone may update beside
running chats ("Update now": each keeps the version file it started). After an
update: `--version` again, the model cache forgotten and re-fetched; failures
are one ANSI-stripped line with secrets hidden (`hide_secrets`: values after
token/key/secret/password/auth words, bearer values, URL passwords, `sk-`
keys, 24+ character letter-and-digit runs), and the version from
before is offered as Restore where the method can reinstall it. A Codex
version outside `codex_probe::VERIFIED` keeps its tool-free helpers off, and
the result says so. Commands run under process-group supervision (15 s
version, 45 s check, 10 min update) in the temp folder with Hollow's tool PATH.
IPCs `agent_cli_status(refresh?)` (read; app action `agents.agent_cli_status`),
`update_agent_cli(agent, now?)` (app action `agents.update_cli`, Confirm, card
names the version and command), and user-only `restore_agent_cli`,
`cancel_agent_cli_update`, `set_agent_cli_policy`. Worker results post a
notification (source `agent-updates`, action `settings:defaults`).

Preferences are atomic JSON at
`~/Library/Application Support/com.hollow.app/agent-settings.json`
(IPCs `get_agent_settings` / `set_agent_settings`; payload
`{defaultAgent, models, efforts?, codexNoOutputTimeoutMin?, noOutputTimeoutMin?, deadlineMin?, canvases?, compactWindow?}`;
`canvases` is Settings → Chat → Canvases, `off` or `most`, absent = When it
helps, never stored as `helps`; `compactWindow` (OB22) is 300000, 400000 or
600000, absent = off, with no control yet: see
[AGENT_CONTEXT.md](../AGENT_CONTEXT.md), "Compaction window").
Settings builds every save with `agentSettingsPayload()`
(`src/agentSettingsPayload.ts`) so no save resets a field it does not show.
Settings apply to new runs; a current conversation keeps its selection.

| Capability | Claude Code | Codex | Antigravity | Grok |
| --- | --- | --- | --- | --- |
| Chat, custom model, exact native resume | Yes | Yes | Yes | Yes |
| Unrestricted ordinary workflows | Yes | Yes | Yes | Yes |
| Hollow-owned vault/computer/team/questions/tasks MCP | Yes | Yes | Yes, over ACP (2026-11-20): `session/new` `mcpServers`, per chat; one-shots and teammates run `agy -p` and get none; `--settings` takes no `mcpServers` and nothing is written to `~/.gemini/config/mcp_config.json` | No per-run MCP flag (1.0.46); its own `~/.grok` servers still load; the chat and a teammate result say so |
| Explicit per-tool workflow restrictions | Yes | No; fails explicitly | No; fails explicitly | No; fails explicitly (only the computer-control `osascript`/`open` deny, as `--deny`) |
| Tool-free text assistants | Yes | Verified CLIs 0.146.0, 0.153.4 and 0.162.1 (`codex_probe`); another version doesn't run and says why | No verified tool-free mode | No verified tool-free mode |
| Ingest | Existing bounded file/web tools | Pasted/local text via host-written proposals | Restricted ingestion unavailable | Restricted ingestion unavailable |
| Steer a running turn (mid-turn steering, 2026-10-21) | Yes: chat turns read stdin (`--input-format stream-json`) | Yes: chat turns run `codex app-server` and steer with `turn/steer` (steering PR 3) | No (its stream input only queues a next turn); Queue or Stop and send | Not on `grok -p` (only ACP has `_x.ai/interject`); Queue or Stop and send |

The complete per-provider contract (owned MCP, connectors, app tools, the
reader, the CLI's own servers/apps/plugins, cards, skills, tool-free mode,
heartbeat, modes, delegates, budget metering) is one table in code,
`capability_contract.rs`. Tests render it verbatim into
[docs/AGENT_CONTEXT.md](../AGENT_CONTEXT.md#tools-per-provider-the-contract)
(regenerate with `HOLLOW_REGENERATE_CONTRACT=1 cargo test --lib
capability_contract`), check the cells that restate a runtime decision
against the code, and derive each chat's capability-index line from it.
Change a capability there; its guidance text reaches resumed sessions
(fingerprint v3 hashes no prompt text, OB04), so a change that must restart
native sessions bumps `chat_resume::PROMPT_CONTRACT`. Owned MCP for Grok is not built (what it
would take is in that doc; a separate decision); Antigravity chats get it over ACP
(`acp_client::mcp_servers_from_config`, `agent::owned_mcp_for`).

- **Conversations are Hollow's.** One Hollow session key spans provider
  stints; an agent or permission change clears only the native id, and the
  next turn carries a bounded `<conversation-context>` bundle and the
  `<task-state>` record. The store is
  `session_store.rs` (SQLite outside the vault: transcripts, receipts,
  reversible archive, usage ledger, run queue, question cards, feedback, and
  `derived_state`: each conversation's task-state record and profile
  baseline, rebuilt from the rest when missing).
- **A send is a run-queue row.** One live turn per conversation
  (`CHAT_ALREADY_QUEUED`); a send while an automation holds the vault queues
  behind it. `execute_turn` commits the user's message, emits `runStarted`,
  retrieves inside the lease, then spawns and drives the agent; every exit
  after the commit goes through `finish_observed_run` exactly once.
- **Each stage reads the request, not the prompt** (agent effectiveness F2,
  `turn_request.rs`). `accept_turn` splits the composed prompt once into a
  `TurnRequest` carried on the queued payload (`request`; a row from before
  reads as its `asked`): the user's own text (= `asked`), explicit entities
  (paths it names, `vault:` allowed, absolute ones in the vault made
  relative; attached image and file paths; notes under `projects/`, `people/`,
  `goals/` it names by file name as whole words, judged on the original
  text, where a combining mark continues a word; a capitalized name only
  when written capitalized; at most 8), resolved antecedents, the injected
  blocks kept apart by tag, size and SHA-256 (their text stays in the
  prompt only), and `taskState` (the conversation's task-state record as admitted:
  revision, last folded turn, items; a reference, the compiler sends the
  record). Retrieval reads `asked`;
  recall, connector selection and learning read `turn_inputs` (request,
  antecedent names, subject paths), but Jev's connector query is the request
  alone (`context_rank::ConnectorQuery`: antecedent names inform only the
  local rules). Note names match by character, never by byte offset into
  the lowercased text (`turn_request::Lowered`). The composed prompt is only what the
  agent is sent. **Antecedent resolver:** only when the request says "that
  project/person/goal/file/note/doc", "same …" or "previous …", or a kindless
  "that one", "same one" or "as before" together with a kind word (project,
  person, goal, or a file word; alone they resolve nothing), and names
  nothing of that kind itself; it reads at most the 8
  latest earlier user requests of the conversation (`session_store::
  latest_asks`, 4,000 characters each, never replies or injected blocks),
  takes the newest that named an entity of that kind, keeps at most 3. Local
  and deterministic, never a model call. "this project" is not a reference
  (in a workspace it means the workspace).
- **One host context compiler** (`context_compiler.rs`, agent effectiveness
  F4) decides what a chat turn's body carries besides the caller's text, for
  every entry point: the composer (vault, quick and workspace chats, answer
  cards, Continue) and conversations agents start or message through
  `hollow_app` (`app_sessions::start`/`send`, entry `appSession`, from the
  row's `app_depth`). On every fresh native session (new, reset, a failed
  resume retried) it puts the conversation bundle from the session store
  (`session_store::conversation_entries`: the last 24 user/assistant
  entries, the last 6,000 characters of each, the JSON body cut from the
  front to 32,000 characters, `<` escaped) and then the enabled operator
  profile (`operator::compile`, gated on the About you switch,
  `operator::Switch`: persisted to `operator-context.json` in the app-data
  folder, **the source of truth**: every turn reads it, the window adopts it
  at launch through `operator_context_switch` (`src/operatorSwitch.ts`) and
  sends only an explicit change; its localStorage copy is a display cache,
  never pushed, and carries an earlier "off" over only once, when no file
  exists yet. Never set means on, unreadable or no app-data folder means
  off; saves hold a lock and use a unique temp file) in
  front of the caller's text; a resumed turn gets neither. So the
  profile goes in once per native session, a new child included. Between
  the bundle and the profile a fresh session also gets the **task-state
  record** (`task_state.rs`, K2): derived state, never a source, built
  deterministically from the transcript (requests, replies, tool calls), the
  turns' chat receipts and History's after-hashes, stored per conversation in
  the session store's `derived_state` table and folded forward one finished
  turn at a time (stopping at a turn still `running`; another schema is
  rebuilt). It holds the goal (first request), settled constraints and
  corrections (sentences of the requests matched by fixed cue words, never
  a question, never the reply; a turn another agent sent through
  `hollow_app` is recorded as `turns.asked_by = agent`, its items carry
  `"by":"agent"` and the header then says who asked instead of calling them
  the user's), the steps that acted (tool calls that
  aren't reads, searches or question cards: `done`, `failed`, `unconfirmed`,
  so a fresh session doesn't repeat them; targets redacted like
  `read_session`'s tool inputs), the latest turn's open questions (a
  card's only while it is still pending or stale in the interactions
  store: one answered, dismissed or superseded is left out, so the answer
  turn doesn't list its own question)
  and the changed files with their revision, each citing its turn and run;
  bounded (12 constraints with the first 4 always kept, 6 corrections, 16
  steps, 4 open, 12 files, 280 characters an item, the block 8,000
  characters, a hard ceiling after `<` is escaped: past the usual drops,
  item texts are cut shorter, then items go one by one). Record schema 2;
  older rows are rebuilt. It ends with the transcript reference:
  `hollow_app` `read_session` with the key when the run has app tools, else
  a plain "can't be read from here". Its `lastTurns` is the stored turn
  count, this turn included (its request is stored before compiling and
  `read_session` counts it), so the call reaches turn 1; capped at 100
  (`task_state::READ_SESSION_MAX_TURNS`, `read_session`'s own maximum,
  which rejects rather than clamps), past which the text says the older
  turns are covered only by the record. A continued conversation's record merges
  the earlier ones' first, tagged with their key. On a **resumed** session
  the compiler compares the digest's sections (`operator::compile`'s
  `sections`: each `me/` file as it was sent, frontmatter stripped, so a
  review date alone is no change, and cut at 6,000 bytes) with what that
  session was given (the `profileBaseline` row, written on every fresh
  session; schema 2, so a raw-file baseline from before 2026-10-07 is
  adopted afresh; a line the digest cut was never received, so it arrives
  as added once it fits) and, when a line changed, puts an `<operator-update>` block in
  front once: per file the removed (`-`) and added (`+`) lines, at most 2,000
  characters, then the files to read. A line counts as removed only when it
  is gone from the file itself (`operator::compile`'s `sources`), not when
  the 6,000-byte cut or the 16,000-byte total merely pushed it out of the
  digest; such lines stay in the session's baseline, so their later removal
  is still reported. The truncation mark and a cut fragment of a line are
  never lines. A session with no baseline (started
  before K2) adopts the current profile and gets nothing; with About you off
  nothing is compared. **The fingerprint stays as it was** (decided in K2):
  a profile edit goes as a delta because a fingerprint change would restart
  the native session; skills stay `&[]` because Hollow attaches none per run
  (the relevant skill index is per turn, in its message, so a different
  index never restarts a session) and each turn is a new CLI process that
  loads its own skills (pinned by
  `chat::tests::the_fingerprint_leaves_the_profile_and_skills_out`). A
  conversation started from another one (Sessions -> Continue from summary,
  or `recoverChatContext` after the working folder vanished: the window keeps
  showing the earlier turns under a new key) names it in `chat_send`'s
  `continuedFrom` (`chatContinuedFrom` in the store, in memory, cleared by a
  new chat, kept in a backgrounded conversation's snapshot); `accept_turn`
  keeps well-formed keys other than its own, each once, the newest 8, and
  the bundle on a fresh session reads those conversations from this vault's
  store first, then its own. Agents' sessions name none. A block the
  text already starts with (a row queued by an older build) is not added
  twice. The composer sends only its request with its own blocks
  (`<page-context>`, `<hollow-chat-intent>`, `<attached-images>`,
  `<attached-files>`); there is
  no frontend builder of either block any more. **Counting
  rule:** every character bound counts Unicode scalar values (Rust `chars()`,
  JavaScript `[...text]`), never UTF-16 units or bytes; the shared fixture is
  `src-tauri/fixtures/continuity/char-counting.json`.
- **The profile digest keeps the settled files** (G2, 2026-10-07):
  `operator::compile` still reads Identity, Communication, Anti-patterns,
  Guardrails, Decisions in that order (6,000 bytes a file, 16,000 in all),
  but on overflow `Decisions.md` and `Guardrails.md` are kept whole and the
  prose gives way, Identity first, then Communication, then Anti-patterns
  (the file that doesn't fit cut to whole lines with the truncation mark).
  Until G2 it filled in reading order and stopped, so Decisions went first.
  Automations read the same digest.
- **One total context budget per turn, and deltas on resumed turns**
  (`context_budget.rs`, `stable_layers.rs`, G2, 2026-10-07). `chat::TurnComposer`
  builds every chat prompt, the first and a failed resume's retry alike.
  *Budget:* 128,000 bytes of Hollow's text per turn for every provider;
  Claude is charged its appended system prompt every turn, Codex,
  Antigravity and Grok what their prompt carries. Never cut: the configured
  half, the connectors note, the request, the task state, recall, the
  profile's Decisions and Guardrails, a profile delta (a turn whose never-cut
  layers exceed the total still goes; the record says `over`). Then the
  retrieved evidence whole, the workspace envelope and the conversation
  bundle sharing what is left (smaller whole, larger cut to an equal share;
  the envelope rebuilt by `workspaces::context_for_within`, giving way in its
  usual order), then profile prose (`operator::compile_within`), then the
  visual guide block, whole or left in its scratch file (`onDemand`; a guide
  with no file can't be cut). A fresh turn whose budget cut the prose stores
  the prose room with its profile baseline, and resumed turns compare within
  it, so withheld prose never comes back as an `<operator-update>`.
  *Deltas:* once the agent took the prompt (a finished reply, under the id
  its stream announced; a failed, stopped or silent turn keeps the previous
  baseline, `chat::stable_layers_holder`), the conversation's
  `derived_state` row `stableLayers` records what its native
  session holds (the announced native id, the appended prompt's SHA-256 when
  the prompt carried it, the envelope's documents, source entries and
  excerpts). A turn that resumes exactly that native id, within 8 resumed
  turns of the last full send (`REFRESH_TURNS`), gets the envelope as an
  update (`DELTA_PREAMBLE`: unchanged documents as a marker, a changed one as
  `+`/`-` lines or whole when that isn't smaller, excerpts it has by range in
  `givenEarlier`, quiet sources counted in `unchangedSourceFiles`; an
  unchanged quick-chat envelope is left out), and Codex, Antigravity and Grok
  get `agent::CONFIGURED_HELD` instead of the appended prompt
  (`RunExtras::configured_held`, honoured only while the spec resumes); the
  workspace policy and permission paragraph go every turn, and Claude's
  appended prompt is its system prompt, so it is unchanged. New, reset and
  retried sessions, another native id, a changed envelope preamble, a budget
  cut and the refresh get everything in full. Nothing here is fingerprinted.
- **Stop and finish:** `chat_cancel(sessionKey)` is keyed. A stop or a normal
  finish ends the turn's whole process tree (SIGTERM, SIGKILL after 3 s,
  verify), then finishes History and the receipt. A stop also stops the
  child task turns the turn started (F10 D2, see Agent teamwork); a finish
  leaves them running.
- **Pause and resume (P1a, 2026-11-15; backend PR 1, UI PR 2):**
  `chat_pause(sessionKey)` (`sessions.pause`) asks a running turn to pause
  (`chat_pause.rs`; phase `pausing`). The attempt loop ends it at its next
  safe point (`chat_resume::SafePoints`: the next main model request with no
  tool call open, never after a question card) like the other turn guards
  (`Yield::Pause`, `Ended::LimitReached`), keeps the native session and
  records the chat in the session store's `chat_pauses` table with a one-line
  note. The turn closes `cancelled` everywhere statuses are read (History,
  receipt, queue row, usage outcome), so nothing fails over, retries or
  follows up, and its child task turns keep running; its transcript closes
  with a `paused` event and the window gets `{type: "paused", sessionKey,
  note, at}` after the receipt, then `done`. A turn that ends before any
  safe point is not paused: its request just ends with it and the window's
  Pausing state ends with `done` as for any turn. Claude and Grok (Claude's
  stream-json, steering pipe or not) reach a safe point at the next main
  request after a tool result, Codex (exec and app-server) when its next
  agent message completes after a tool result; Antigravity's stream has no
  tool calls, so it never pauses and runs to its end. A Stop wins: one before
  the safe point ends the turn stopped, and `settle_pause` decides while the
  turn's stop slot is still held, so a Stop after the cut either closes the
  turn as stopped (no record) or reaches the slot and `cancel` clears the
  record (`stop_clears_pause`; any Stop of a paused chat unpauses it).
  `chat_resume_paused` (`sessions.resume`) claims the record once
  (`claim_resume`: `claimed_at` and an owner token set in one write
  transaction, so of two resumes at once one is refused; a claim lapses
  after 120 s), then queues an ordinary turn with `RESUME_PROMPT` on the
  stored native session, the token as its `clientTurnId`. Only that token
  releases the claim: a failed send, every exit of the resume's turn and
  every unstarted end of its row (`run_pump::end_row`, a cancel from the
  queue). The record clears when a turn of the chat spawns its agent
  (`on_turn_start`: the resume or the person's own message), so a resume
  stopped or failed before its spawn leaves the chat paused, and a resume
  still queued after another turn cleared the pause is refused at admission
  (`Superseded`: a cancelled queued row, no turn).
  `agent_session_live` and `sessions.overview` list a paused chat as phase
  `paused` (`paused: true`), never running. In the window (PR 2): **Pause**
  sits beside the composer's Stop once the turn has started (Pausing… and
  disabled while asked; Stop still works and wins), the `paused` event
  leaves "Paused: <note>. Resume picks it up from here." (not a stop notice,
  so no Continue), and **Resume** takes Pause's place once paused; it calls
  `chat_resume_paused` and adopts the queued turn like a send made while
  busy. Paused chats show the `paused` indicator (two bars, no ring) in the
  sidebar and their newest cancelled row reads Paused with Resume in
  Mission Control. `/pause` and `/resume` are faster routes to the buttons.
  After a reload the window learns a chat is paused from its record when it
  is opened or its newest queue row ended cancelled.
- **Pause all (P1b, 2026-11-15):** Running's header has **Pause all** while
  a chat is running and **Resume all** while one is paused (`chat_pause_all`,
  `chat_resume_all`; `sessions.pause_all`, `sessions.resume_all`, never the
  calling chat). Pause all asks every registered turn that isn't stopping
  and is a person's own chat (the stop slot's `own_chat`: depth 0,
  attended, not bare, not the COO's or a lead's) to pause at its next safe
  point as reason `all` (`chat::request_pause_all`);
  a chat already asked keeps its own reason, and a queued row has no turn to
  ask, so it is left alone. Each asked chat settles once: paused with its
  note (`settle_pause`) or ended first (its stop registration's drop, or a
  newer turn of the chat replacing it). The last to settle posts one
  notification ("Paused 2 chats; 1 finished first", a line per chat with its
  title and note) whose action `mission-control:` opens Running. The batch
  lives in memory; a quit before it settles posts nothing, and each pause
  record still holds its note (the batch is process-global). Resume all
  resumes every paused chat through `chat_resume_paused`'s claim, whatever
  paused it, except one whose resume time is still ahead (listed as
  `skipped`).
- **Resume at a chosen time (P1c):** `chat_set_resume_at(sessionKey, at)`
  (`sessions.resume_at`, RFC 3339; omit `at` to clear) sets or clears the
  record's `resume_at`, within a year; a time already past resumes at once.
  The scheduler's one-minute window tick (`schedule_windows::start_timer`,
  queue owner only) calls `commands::resume_due_paused`: each chat whose
  `resume_at <= now` and whose claim is free (`chat_pause::due`) is resumed
  through the Resume button's claim under an automatic token naming its
  attempt (`pause-resume-auto-<n>-<hex>`, counted in the record's
  `auto_attempts`), so a chat is never resumed twice; a time missed while
  Hollow was closed comes due at the first tick after launch. A definitive
  refusal (not paused, gone) drops the time; a transient send failure
  retries next tick up to 3 attempts, then drops it; an automatic resume
  released without a spawn (withdrawn, refused, failed or stopped before
  start) drops it too, so a past time never re-fires. Each drop posts a
  notification that the chat stays paused, and a resume that starts posts
  "Resumed <title>". Stop, a Resume or a new message
  ends the time with the record (any turn's spawn clears it). In the window:
  **Resume at…** beside the composer's Resume and on Running's paused rows
  (`components/chat/ResumeAtControl.tsx`: in 1 hour, tonight 9 PM, tomorrow
  8 AM, a custom date and time, Clear time) and the time beside it
  ("Resumes tomorrow at 08:00"); `chat_paused_list` gives the window each
  record's note, reason and time, and the `paused` event now carries
  `reason` and `resumeAt`.
- **Pause at a usage limit (P1d):** a turn that ran out (R1 stopped with
  every Claude account out, or the provider's `ProviderQuota`, never a
  self-hosted endpoint) and failed now pauses instead (`chat_pause::
  limit_pause`, decided in `execute_turn` before the error is emitted):
  reason `usage_limit`, native session kept, the note names the limit and
  its reset ("Every Claude account hit its usage limit. It resets at 16:40;
  the chat resumes then."), and `resume_at` is the reset plus 60 s
  (`RESET_MARGIN_SECS`; the reset from `chat::limit_reset_at`: the soonest
  account, the stream's rate-limit event, else a time in the error or the
  result's own text), so P1c resumes it; no reset time means no time. It
  closes as a P1a pause, recorded under the stop-slot lock while the turn's
  registration still holds it (`settle_limit_pause`), so a Stop that came
  first closes it as stopped. It still fails as before on credit or billing
  exhaustion (`result_kind::CREDIT_RE`: no reset ends it), when a Stop or a
  night or budget guard ended it, "when one runs out" moved it to another
  agent, it named no native session, or it is a child task, unattended,
  bare, COO or lead turn (a resume runs at depth 0, attended). A limit hit
  right after an automatic resume, before any reply or tool, counts that
  attempt; at 3 it pauses with no time and a notification. Stop and Resume
  work on it as on any pause.
- **Every adapter gets the same context** (vault map, no-messaging policy,
  operator and recall context, attached capabilities, scratch guidance):
  appended system instructions for Claude, prompt text for the others.
  Evidence comes from filesystem changes and completed tools, never prose.
- **One-shots** (`claude_runner.rs`) use the app preference; pure transforms
  are tool-free. Codex's `codex_transform.rs` runs only on a CLI version in
  `codex_probe::VERIFIED` (0.146.0, 0.153.4, 0.162.1), each proved by the ignored
  `installed_codex_advertises_zero_tools_to_local_responses_fixture` test (a
  loopback Responses fixture, no provider call; rerun it against a new CLI
  before adding its line) and, since night ST12, by the ignored
  `installed_codex_json_output_parses_from_a_local_fixture_answer` (the
  fixture answers; the CLI's real `--json` output must parse through the
  helper) with the captured output pinned as a normal test. The one-line
  check: `cd src-tauri && cargo test --lib installed_codex_ -- --ignored`. `codex_probe` reads the version once per binary
  (path, size, mtime, ctime, inode and the npm `package.json`; failures
  never kept). Any other version, a missing CLI or an unreadable version
  stops before a model turn with one sentence naming the version, the
  verified ones and the fallback (nothing ran; Hollow never switches
  agents); the usage row keeps it as its note and Settings → Models
  shows it (IPC and app action `codex_helper_status`).
- **Skills: a relevant index per turn, read on demand** (F9, wave 4 · V2,
  `skill_index.rs`). Hollow attaches no skill to a run's configuration.
  Each chat turn's message gets a `<hollow-skills>` block (after the
  connectors note) when a skill fits: up to 5 skills, one line each (name,
  source, why, description cut to 160 characters), from only two roots, the
  vault's `.agents/skills` then the shared `~/.agents/skills` (a vault skill
  wins a name clash). A skill selected with Skills → Use in workspace
  (`skillCatalog.ts` `useSkillPrompt`, whose text the host parses) always
  comes first, and the selected skills' `SKILL.md` text goes inline while it
  fits 8 KB in total, so Antigravity and Grok get it too; a selected
  provider or plugin skill is read through `skills::read_skill_in` (the
  registered set) and only inlined. The rest are a local, deterministic
  match of the person's words against each name (3 a word, 10 when named
  outright) and description (1 a word), at least 3: no model, no Jev. No
  block when nothing fits. The agent reads one whole with `hollow_vault`
  `read_skill`, else `hollow_app` `skills.read_skill` (which now also reads
  a vault skill by its path), else its own file tools at the listed path.
  The manifest's `skills` record says what was indexed, why, what went
  inline and what the agent read. A provider's own runtime may also load
  the skills installed for it (unverified per run); Codex plugin skills
  don't load (plugins are off). Workflows and the heartbeat get no index.
- **Skills → New skill and Automations → Edit → Save as skill:** both request
  `write_user_skill` with `create_only: true`. Exclusive creation refuses an
  existing directory, including an empty one. On collision, Replace names the
  skill and warns that History cannot undo it; Save under a new name tries
  create-only again, and Cancel writes nothing. The automation form disables
  Save as skill for every unsaved field. Native callers default to create-only;
  the existing confirm-tier agent action explicitly retains its create-or-replace
  contract. Provider-owned/symlink skill locations remain protected.
  The Skills editor retains multiline descriptions. Shared writes serialize
  descriptions as JSON-escaped, YAML-compatible double-quoted strings; catalog
  reads decode them once. Quotes, colons, backslashes and line breaks survive
  create/replace/reload. Empty descriptions use the skill name. Edit still
  warns before dropping header fields other than name and description.
  Every shared SKILL.md write (New skill, Edit, and a Saved skill lesson)
  goes through one protected writer, `skills::write_skill_file`: a safe
  name, no symlinked `.agents`, `skills`, skill folder or SKILL.md, an
  exclusive folder for a new skill (`SKILL_EXISTS` otherwise) and an atomic
  write, under `SKILL_WRITE`.
- **Skill lessons** (2026-11-08, `skill_learning.rs`). A capture can propose
  a lesson for a shared skill the source chat used (pinned to a fresh read's
  hash) or a new shared skill; Library Learnings and the source reply show
  Save and Reject. Save appends `## Learned task guidance` to the pinned
  bytes (frontmatter untouched) or writes the new skill, after scanning the
  whole resulting file with the memory safety scan, not only the lesson. A
  skill that changed since the pin, or a new skill whose name is now taken,
  closes the lesson as stale (`Superseded:`, shown as "Stale skill lesson");
  a later chat proposes again against the fresh read. One bad skill target in
  a capture is skipped with a logged reason; the batch's other proposals
  still queue. A saved lesson has **Undo skill lesson** (`undo_skill_lesson`,
  user-only): while the skill is exactly what Save wrote, it puts back the
  pinned bytes (or removes the new skill's folder) and the lesson is pending
  again; after any later edit it refuses and changes nothing. The memory
  change log records Save and Undo with the before and after text, and Undo
  checks it first (`memory_guard::ready`, like every memory change): a
  damaged last entry or a log that can't be appended refuses Undo before the
  lesson or the skill changes. A refused Save, Reject or Undo shows its error
  on that lesson's card, in Library and the source reply alike
  (`skillErrorKey`, kept while the lesson's status is the one it was tried in).
- **Skills → Import skill** (2026-10-09, `skill_import.rs`, `src/skillImport.ts`). Next to
  New skill, plus a file drop anywhere on the Skills page. Takes Claude's `.skill` export (a
  zip of one skill folder), any `.zip` (recognised by its bytes, not its extension), a skill
  folder (Choose folder… only: a *dropped* folder is refused, because WebKit reads through
  links inside it and Rust can't see them), or a bare `SKILL.md`. Two steps:
  `preview_skill_import` reads and checks the source and stages it in
  `~/.agents/.skill-imports/<token>/skill/` (beside `skills/`, so nothing lists it yet and
  the install is one `rename`); the preview (name, description, files with sizes, a warning
  naming the files that run as code: under any `scripts/` or `bin/` folder, a script
  extension, or executable) is built from those staged files. `commit_skill_import` (the
  person's Install, `user-only`) first claims the token (`<token>` → `<token>.claim`, so a
  racing Cancel/discard finds nothing; a failed commit hands it back), then moves it to
  `~/.agents/skills/<name>`. On a name clash `replace` swaps the folder (never through a
  link; the old one waits in `<token>.previous` until the swap succeeds, and an error names
  it if the rollback fails; `.previous` is never swept) and `keep-both` installs
  `<name>-2`… and rewrites every top-level frontmatter `name`; Cancel discards. Refused:
  absolute or `..` paths, backslashes, symlinks and special files (folder files opened
  `O_NOFOLLOW`), more than 1,000 files, 50 MB in all or 25 MB in one file (counted on what
  decompresses, not the zip header), SKILL.md over 1 MB or without frontmatter `name` (the
  New skill rule: letters, digits, `-`, `_`) and `description`, two paths differing only by
  case, a path that is both a file and a folder, SKILL.md neither at the top nor in the one
  top-level folder, and (2026-11-16, NV2) hidden text: a zero-width, bidi-control or Unicode
  tag character in any text file (UTF-8, no NUL; a leading byte-order mark is allowed), found
  by `memory_guard::invisible` (one scan for memory writes and imports) and refused at stage
  naming the class, file, line and column but never echoing the text. **Illegal names (SD2,
  2026-11-17):** a SKILL.md `name` outside letters, digits, `-` and `_` (`stitch::generate-design`)
  is still refused, but the message now names a suggested legal name
  (`skills::suggest_skill_name`: lowercase kebab, each illegal run one `-`, trimmed, at most 64
  characters; the first of `<name>`, `<name>-2`, … not installed; "nothing legal remains" when no
  letter or digit is left). `preview_skill_import` takes an optional `renameTo` and
  `skills.import_skill` an optional `renameTo`: only when passed, the import is staged under it
  (a legal name, not installed, else refused; refused too when the name was already legal),
  SKILL.md's frontmatter `name` is rewritten and every whole-name occurrence of the old name in
  the skill's own `.md` files (not scripts or other files, never outside the skill folder; `/`
  is a boundary, so `skills/<old>/x.md` paths are rewritten but `<old>-2`, `x<old>` and
  `<old>.md` are not) becomes the new one. The preview reports `renamedFrom` and `rewritten`
  (path and count), and the Skills page offers the suggestion as an "Import as" button. NV2's
  hidden-text scan, the size caps and the SKILL.md check run on the renamed result. Check for
  update on a renamed skill still refuses the source's illegal name (follow-up). **Skill lint (NV1,
  2026-11-16):** `skills::list_all_in` attaches `skill_lint::lint` to each skill of the shared
  library (`Skill.lint`, absent for provider and plugin copies): a score of 100 minus fixed
  deductions in four capped areas (schema 30, description 25, size 20, safety 40), each
  finding with a rule, a message and a suggested fix, and the grade `good` (90 or more),
  `fair` (70 or more) or `weak`. Safety is `memory_guard::scan`'s (hidden characters, a
  stored secret or secret-sending command, an instruction override), reported by line and
  never by text. The Skills page shows a "Lint 85" badge on the row and a Skill check list in
  the detail; it runs no agent and sends nothing. `__MACOSX/`, `.DS_Store` and `.git/` are skipped and listed. Staged and
  claimed imports older than a day are swept. The UI ignores leaving the panel while Install
  runs and discards a check that finishes after the panel closed. Agents have `hollow_app`
  `skills.import_skill {path, onClash, renameTo}` (confirm, in the auto "what agents may do" class): its
  card stages the import and lists the files and scripts, and allowing commits that staged
  token (`card.bind`), so what installs is what the card showed; a clash with `cancel`, or
  `replace` on a link, is refused on the card.
- **Skill provenance** (repo ideas 2026-10-27 #2, `skill_provenance.rs`). Every commit,
  the person's or `skills.import_skill`'s, records the installed name in Hollow's app data
  (`skill-provenance.json`, never in the skill folder): what was picked (`path`) or dropped
  (`dropped`, its file name), the origin when the picked path sits in a git checkout whose
  `remote "origin"` is plain `https://` (URL, HEAD commit and the path's place in the repo,
  read from `.git/config`, `HEAD`, `refs/` and `packed-refs`: git never runs; each read is
  a plain file inside `.git` with no link on the way, opened without blocking, 64 KB at
  most, 4 MB for `packed-refs`, walked with `openat` and no link followed; a URL with a
  user name or password, a query or fragment, SSH or `file:` is not recorded), every file's SHA-256 as installed (the import's own walk and caps; SKILL.md
  hashed without Hollow's note block, so a note is never a change) and the time. Staging writes this beside the
  staged copy (`<token>/source.json`); a failure to record never undoes an install.
  Skills → a shared skill → **Source** (`skill_provenance`, also the read action
  `skills.skill_provenance`) shows "Imported <date> from <path or repo (folder) at
  commit>" or "Not imported by Hollow" (no record: made in Hollow, installed by something
  else, or imported before 10-27; plain text, not a warning), and **Changed outside
  Hollow** with the files added, removed or changed since. Edit skill refreshes only
  SKILL.md's hash, so an outside change to a script still shows; Delete skill drops the
  record. The block appears only for a copy Hollow manages (its folder is its name and no
  link is on the way, as for Edit and Delete), and every command checks the same.
  **Check for update…** (`stage_skill_update`, `user-only`) stages the recorded source
  again through Import's staging: an `https://` origin is cloned shallowly into staging
  (`fetch-<token>`, removed after; only on that click), isolated from this Mac's git setup
  (no system or global config, no `GIT_*` environment, an empty HOME so `~/.netrc` isn't
  read, `GIT_ALLOW_PROTOCOL=https`, no credential helper, prompt or hooks, links checked out
  as plain files) in its own process group, ended with everything it started at 120 s; the
  recorded folder must then be a plain path inside the clone, component by component.
  Else the recorded path is read again; a dropped file or a missing path says how to import
  the new version instead, and a source that now holds another name is refused. Import's
  own card then shows the files changed, added and removed against the installed copy, the
  scripts among them, and each changed or added file's text (the lines around the change,
  or an added file's opening lines; scripts first and open; 16 KB a file, 64 KB in all;
  binary files and files over 256 KB listed only), with Replace chosen; nothing is
  replaced before its Install. **Local note** (`set_skill_note`, `user-only`, at most 4,000
  characters, no HTML comments): kept in the same app-data file and written, under the same
  guards as Edit skill (no linked SKILL.md), into SKILL.md as one block right after the
  frontmatter (`<!-- hollow:local-note -->` …
  `<!-- /hollow:local-note -->`), so every agent reads it; any import that installs that
  name (an update) writes it again first. Nothing updates by itself.

### Repo trust (SK1, 2026-11-17)

A chat whose working folder is a git checkout (a build workspace's worktree) has its agent
load the repo's `CLAUDE.md` / `AGENTS.md` and project skills (the .claude/skills folder,
`.agents/skills`) by itself. `repo_trust.rs` decides whether that checkout is the person's:
the `origin` remote's GitHub owner of the main working copy (`project_icons::owner_of` on
`run_manifest::claude_memory_root`, so every worktree answers alike), compared without case
with `gh`'s signed-in logins (`hosts.yml` under `GH_CONFIG_DIR` or `~/.config/gh`; tokens are
never kept), the organizations `gh api --paginate user/orgs` lists (cached a day in
`repo-trust-orgs.json`, refreshed in the background, a failure retried after 10 minutes;
until the first answer an org repo stays labelled) and the `owners` in `repo-trust.json` (app data), which only
`skills.set_repo_owner`'s always-ask card changes. Fail-safe: no origin, a non-GitHub remote or
no known login counts as not the person's. The vault, a bare scratch turn and a checkout that
holds the home folder are never labelled. Such a checkout gets:

- a `<hollow-repo-trust>` block ("Untrusted, from this repo: ...") last in the brief, after
  any lead's project context: every instruction file in the repo, at any depth, and every
  skill is someone else's text. Byte-stable (no time, sorted names); only plain skill names
  (letters, digits, `-_.:`) are printed, paths have control characters replaced;
- each skill the person hasn't allowed denied by name (`Skill(<folder name>)`, plus the
  frontmatter name when it differs; a name that can't go in a rule denies `Skill` as a
  whole) on agents that hold deny rules (Claude), and listed in the block as waiting. A
  `Skill()` deny stops the Skill tool only: Read or Bash can still open `SKILL.md`;
- Edit, Write and MultiEdit denied on `repo-trust.json` and the orgs cache (`//**/<file>`:
  the app data path has a space, which the CLI's rule list splits), and `Bash(*repo-trust*)`.
  Only in labelled repos: on every turn the denies would stop a Claude run from handing work
  to a Codex teammate, which can't hold them. A shell write by another spelling still gets
  through, and an agent in a labelled repo can rewrite `.git/config`'s origin to the
  person's login: both are known residuals;
- `skills.approve_repo_skill` (`path`: the skill folder): an always-ask card (Bypass too)
  showing the repo, owner, files and non-Markdown files, bound to the content hash (SHA-256
  with a domain tag and a `u64` length before every path, link target and whole content, so
  no re-split of files keeps a hash; cached per process by each file's path, size, mtime, ctime, inode and
  device; files open with `O_NOFOLLOW | O_NONBLOCK`). The allow is kept by hash, so any edit asks again. A link leaving the folder or to
  anything but a regular file, a FIFO or other non-regular file (never opened), an
  unreadable file, more than 1,000 files, folders deeper than 8 or more than 64 MB make the
  skill unapprovable (the block and the card say why).

Codex and the other agents hold no deny rules: for them the block is the whole gate. Out of
scope: a repo's .claude/agents, .claude/commands and settings hooks, skills in folders
below the cwd, and automation workflow runs (`workflows::execute` spawns its own agent and
never reaches `chat::execute_turn`, so a workflow whose folder is a foreign repo is not
labelled).

### Setup hooks approved by content hash (OC15, 2026-11-18)

A repo can ship one script for Hollow to run when a build workspace is made: `.hollow/setup.sh`
at the worktree root (`setup_hook.rs`). It runs arbitrary code, so it is gated for every repo,
the person's own included:

- **Approval:** the plain SHA-256 of the file's exact bytes (`shasum -a 256` shows the same value),
  scoped to the repo's main working copy (`run_manifest::claude_memory_root`, a real path, so
  every worktree of a repo answers alike), kept in `repo-trust.json` as
  `setupHooks: {"<repo>": {"<sha256>": {"approvedAt"}}}` (absent until the first one; older files
  load unchanged). A newer approval never drops an older one, so switching back to a branch with
  an earlier approved script runs it without asking. The same bytes in another repo are not approved.
- **Never approvable:** a symbolic link (the file or the `.hollow` folder), anything that isn't a
  regular file, a file over 256 KB (read with `O_NOFOLLOW`).
- **On create** (`build_workspaces::create_with`, after the binding and the first-connect draft):
  an approved version starts in the background and the create returns at once, its `warnings`
  saying where the result goes; an unapproved, edited or unusable one doesn't run and the warning
  names the file, the short hash and `build.approve_setup_hook`. A failure or timeout (10 minutes)
  never fails the create.
- **In the background, with a result file:** an agent's action call waits at most
  `app_mcp::BROKER_CHANGE_TIMEOUT_SECS` (450 s, card time included), shorter than a hook may take,
  so a create and both actions only start the run (`setup_hook::start`, one run per worktree at a
  time). It writes `outputs/SETUP_HOOK_RESULT.md` in the workspace's vault folder (never the
  worktree): `Status: running` at the start, then `ok`, `failed (exit N)`, `stopped after 10
  minutes`, `stopped: the workspace's worktree was removed` or `not run: <why>`, the sha256 and the
  output's end. Removing the worktree (`remove_for_unless`, after its refusals) stops a running hook
  and waits for it (30 s at most); trashing the workspace asks it to stop, and a result is never
  written once `workspaces/<id>` is gone. The result's path is resolved with `vault::safe_join` again
  before every write.
- **Running** re-reads the file, hashes those bytes, refuses unless that hash is approved for that
  repo, and runs exactly those bytes: copied to a 0600 file in a fresh 0700 temp folder outside the
  worktree, run as `/bin/bash <file>` whatever its shebang says (not `bash -s`: a command reading
  stdin would eat the rest of the script), cwd the worktree (`$0` is the copy, so use `$PWD`),
  `GIT_*` removed, PATH set to `agent::cli_path()` like every agent CLI (a Finder or Dock launch
  otherwise lacks npm, cargo and brew), stdin empty, stdout joined to stderr, through
  `process_group::run_to_completion` (own group, settled).
- **Actions:** `build.approve_setup_hook` (`id`, Confirm, on `ALWAYS_ASK` so it asks in Bypass
  too): the card shows repo, owner, path, short hash, size, that the shebang is ignored and that
  the hash covers `setup.sh` alone (whatever it calls in the worktree is not covered), and the
  whole script (lines wrap at 160 characters onto `+ ` lines; control, line and paragraph
  separator, bidirectional and invisible format characters shown as `?`). A script that isn't
  UTF-8 or takes more than 400 card lines can't be approved. The card is bound to the hash and
  repo; at apply a change in either is refused, else the approval is kept and the hook starts
  once. `build.run_setup_hook` (`id`, Confirm like `start_app_build`, which also runs commands in
  a worktree): starts an approved version again, refusing an unapproved one with the approve
  action's name. Residual: like SK1's record, a shell write to `repo-trust.json` could add an
  approval; in a repo that isn't labelled, Edit and Write on it aren't denied either.

### Persona acceptance for pressure tests (C7, 2026-11-18)

`personas.accept {role}` (Confirm, on `ALWAYS_ASK`, so it asks in Bypass too and never runs unattended)
is the only way a persona note becomes usable by `simulate.pressure_test`. Its card shows the whole
`personas/<role>.md` the way a setup hook's script is shown (`setup_hook::card_script`) and binds the role
and the note's SHA-256; at apply `stakeholder_sim::accept_persona` refuses a note changed since the card.
It refuses before asking (and again at apply) a note with characters the card shows only as `?` (control,
direction-changing, invisible or tag characters), and before asking a missing, linked or non-UTF-8 note, one
over 16 KB, one with no evidence that still holds, or one already accepted. Agents call it only when the person asks. Details: [PRESSURE_TESTS](../PRESSURE_TESTS.md).

**What reaches an agent, per entry point:** [docs/AGENT_CONTEXT.md](../AGENT_CONTEXT.md)
is the one current map (after agent effectiveness wave 1). It covers UI chats, agent-started
sessions, automations, the heartbeat, teammates, and resume and reset: each layer in reading
order, with its bound and the file that composes it. It also covers how retrieval, connectors
and Jev choose, how to read a run's context manifest, the permission wording contract, the
per-provider tool table and the open gaps by wave. Change it in the same commit as any layer,
bound or capability it describes.

### Agent health recovery (P1-32)

Model catalogs share their latest request; rejection evicts only that exact
promise, never a newer Refresh. Local /help uses the same provider filter as
the Commands popup. Updating copy says new runs are unavailable and must be
tried again after the update, matching the send refusal.

Hollow-authored Codex failure messages name Continue and Agent updates without
exposing the transport name. Provider error detail and classification remain
intact. An active-turn approval request is still declined with its original
response shape; the first adds one warning step through the normal item parser.
Repeated, foreign-turn and ended-turn requests add no warning, and their raw
payloads never appear in the warning text.

Codex transport qualification requires a complete stable numeric version at or
above 0.163.0, based on the recorded 2026-10-21 native qualification. Unknown,
malformed, partial and suffixed versions use exec without native steering.
`resolve_support` shares Agent updates' canonical-binary/mtime version cache.
Turn setup selects that read against Stop before taking the steering lock;
the immutable result controls admission, every attempt's transport and the
`runStarted.steerSupport` row. Foreground and background snapshots retain
it. Agent-wide support refreshes ignore stale request results
and cannot replace a running turn's row. Unit tests never probe installed CLIs.
The model menu puts warning/Refresh and update-state controls beneath each
agent's model listbox, outside option semantics. Button Enter/Space activation
does not select the highlighted model. Model arrows use the focused row,
skip disabled providers, and do not take
Effort or Canvases keys. Catalog request tickets and menu health
read tickets ignore superseded and unmounted results. Opening the menu reads
local status, then polls every 30 seconds while idle or five while active,
never overlapping requests. Errors label retained status as last-known and
offer Retry. Settings update reads and menu reads invalidate agent-wide
support when installed version/state changes. Agent updates links in the menu,
failed-turn row and both unavailable-send notices respect navigation guards.

### Adapter capability flags (AC1, 2026-11-17)

`AgentCapabilities` (the agent picker's `AgentDescriptor.capabilities`) carries six flags next to
`sharedServer` and the rest: `steer`, `interrupt`, `fork`, `rollback`, `queue` and
`pendingSurvivesRestart`, plus `unsupportedReasons` (flag name to a plain sentence naming the feature and
the agent, only for a false flag). They come from one table, `agent::flags`, one literal row per agent,
and nothing else names an agent: a feature asks `agent::supports(kind, Feature)` or
`agent::require(kind, Feature) -> Result<(), String>` (pure, so it refuses before any stored row, spawn or
held send), and a new agent fits by filling one row. A flag is true only when Hollow's adapter does it
today; protocol facts that aren't wired are doc comments on `Flags`.

| Agent | steer | interrupt | fork | rollback | queue | pendingSurvivesRestart |
| --- | --- | --- | --- | --- | --- | --- |
| Claude | yes | yes | no | no | yes | yes |
| Codex | yes | yes | no | no | yes | yes |
| Grok | no | yes | no | no | yes | yes |
| Antigravity | no | yes | no | no | yes | yes |

- **steer** is the table above (Claude stream-json stdin, Codex `turn/steer`). `steering::support_for` asks
  the flag first (false is `SteerSupport::None` at any version) and then holds a Codex to its 0.163.0 floor, a
  version gate on top of the capability. `steering::steer_gate_reason` is the one source of both `support_for` and the note: each gate
  (the capability, the Codex version floor) carries its own reason.
- **interrupt** is Stop: the turn's process group ends for every agent and the session id stays resumable.
  Codex's `turn/interrupt` is written by `AppSession::interrupt` but Stop doesn't use it.
- **fork**, **rollback** are false everywhere: Claude's `--resume --fork-session`, Codex's `thread/fork` and
  `thread/rollback` exist in the protocols but no adapter wires them (CP1 and CH1 flip the flag when they do).
  Hollow's own chat forks (offline copies, subagents) are Hollow-side copies, not session forks.
- **queue** and **pendingSurvivesRestart** are Hollow-side held sends (`steering::hold`, saved to
  `held-sends.json`, sent on the next launch), so they are the same for every agent. `commands::send_or_hold` and
  `hold_agent_send` ask `steering::queue_refusal` (default agent resolved) first, before Stop and send's cancel,
  and `steering::hold` refuses again before saving anything. A run-queue row of a chat that
  never started is interrupted at startup with its text kept (`run_queue::recover_at_startup`), which is not
  this flag.

### Skill portability per agent (NV3, 2026-11-17)

The Skills page's detail has an **Agents** table: for the viewed skill, whether each agent loads it,
loads it degraded or doesn't, with the rule ids and the CLI version each rule was checked on. It is
`skill_portability.rs`, one const table `PROFILES` keyed by `AgentKind` (the only place the file names an agent),
each profile a list of rules (`id`, a pure `Check` on the skill's cheap `SkillFacts`, the `Effect` when it hits
(`Loads`, `Degraded`, `NoLoad`, `Unknown`), its `Evidence` and two plain sentences). `assess(profile, facts,
installed) -> Vec<Finding>` and `table(installed, facts)` are pure; `commands::skill_portability_table(path?)`
(plumbing in the command map, no `hollow_app` action) reads the skill, then each agent's installed version once
through `agent_cli::steering_version`, outside every lock. A rule is on one of two **channels**: Hollow's own
delivery (the index, inline text, `read_skill`) or the CLI's own loader. An agent's summary is the worst effect on
the Hollow channel only; native findings are listed beside it and never move it.

Evidence is `Hollow` (Hollow's own code, tested here: no CLI version), `Cli(version)` or `None(reason)`.
`version_status` compares a recorded version with the installed one and says "not verified on this version"
when they differ (a row is also stale when the installed version can't be read). Rules recorded today:

| Rule | Applies to | Effect | Evidence |
| --- | --- | --- | --- |
| `desc-cap-160` description over 160 characters (`skill_index::DESCRIPTION_CHARS`, whitespace flattened) | all four | Degraded | Hollow |
| `name-unfit` name over 64 characters or with a quote, slash, angle bracket or control character (`fit_name`) | all four | Degraded | Hollow |
| `frontmatter-16k` frontmatter past the 16 KB the index reads (`META_READ_BYTES`) | all four | Degraded | Hollow |
| `not-indexed-copy` a provider or plugin copy (the index lists the library and vault only) | all four | Degraded | Hollow |
| `inline-8k-read-skill` file over the 8 KB inline budget, read whole with `read_skill` | Claude, Codex | Loads | Hollow |
| `inline-8k-own-tools` same, but opened by path with the agent's own file tools (no per-run MCP) | Grok, Antigravity | Degraded | CLI 1.0.46 (Grok), 1.2.15 (agy), from the capability table above |
| `read-cap-64k` file over `read_skill`'s 64 KB | Claude, Codex | Degraded | Hollow |
| `native-codex-plugin-off` a Codex plugin skill (Hollow starts Codex with `--disable plugins`; the effect on plugin skills is not verified on a Codex version) | Codex | Unknown (native) | None |
| `native-claude`, `native-codex` the CLI may load its own skill folders | Claude, Codex | Unknown (native) | None: "not verified per run" in `capability_contract.rs` |
| `native-grok`, `native-antigravity` | Grok, Antigravity | Unknown (native) | None: no loader recorded or verified in this repo |

No rule guesses a cap a CLI was never seen to apply. A test pins the read-route rules to
`agent::capabilities(kind).owned_mcp`. To record a new fact, add a `Rule` with its `Evidence` and list it in the
profile; to re-verify after a CLI update, bump its `Cli` version in the same change.

### Mid-turn steering (2026-10-21)

While a chat turn runs, its composer offers **Steer now**, **Queue for after** and **Stop and send**
(design and per-agent probes: the vault's `outputs/steering-2026-10-21/`). The backend is
`steering.rs`; what each agent can do is `steering::resolve_support` (the table above), which the composer
reads through `chat_steer_support`.

- **Claude's transport.** A chat turn's spec sets `RunExtras::live_input` when its agent steers
  natively. `build_claude` then passes `-p --input-format stream-json` and no prompt, and
  `build_with_codex_servers` pipes stdin (every other run keeps `/dev/null`). In
  `drive_turn_attempts`, each attempt writes the prompt as its first stdin line
  (`steering::input_line`, uuid `hollow-turn-<run>-<attempt>`), then any steer the turn took that no
  attempt delivered (an account switch ended it), and registers the pipe (`steering::attach`).
  Stdin closes at the answer's `result` (`steering::on_result`): at once when nothing is pending,
  otherwise once the late steer lands (its own second `result`) or after `LATE_STEER_GRACE` (15 s).
  If it never closed, Claude Code would wait for more input after its `result`. The output stream
  and its parser are unchanged.
- **Shared process (SE9).** Local GUI-owned HTTP-only interactive turns use
  `shared_codex` behind `sharedServer`: one initialized Codex per compatible
  account and a small inherited-fd relay per attempt. Config and grants stay
  on the thread; the relay retains ordinary steering and process cleanup.
  Unsupported and remote runs retain the private path. [Contract](../SHARED_CODEX.md).
- **Codex's transport (steering PR 3).** A Codex chat turn with `live_input` (`agent::live_codex_input`:
  interactive only) builds with `build_codex_app_server` (exec's top-level flags and the run's
  `mcp_servers` override, then `app-server`), stdin piped; every other Codex run (tool-free helpers,
  workflows, teammates, one-shots) still runs `codex exec`. Each attempt makes one
  `codex_app_server::AppSession` from `agent::codex_app_server_start` (the same `contextual_prompt` exec
  would get, `spec.resume` → `thread/resume`) and writes its `initialize`; `drive_turn_attempts` feeds
  every stdout line to `AppSession::on_line`, writes what it answers (the rest of the handshake, a
  decline for any server request: exec can't ask either, so no card), and passes each translated
  `codex exec --json` line to the same per-line handling as an exec line, so the thread id (Session,
  resume), the rollout watch, usage, tool rows and file-change attribution, errors, the outage backoff
  and the failed-resume fresh retry all work as before. A steer goes down the pipe as
  `PipeMsg::Steer` (`steering::input_msg`), because only the attempt's session knows the running
  turn's id: the writer task asks the session (shared as `Arc<Mutex<AppSession>>`, never locked across
  an `.await`) for its `turn/steer {threadId, expectedTurnId, clientUserMessageId, input}`, held until
  `turn/start` is answered and then written by that same writer (`PipeMsg::HeldSteers`, or before the
  next steer), so steers keep their order. An `error` the server will retry (`willRetry`) is not
  passed on; a turn Codex ends `interrupted` on its own reads as failed, not a success (once Hollow is stopping,
  `AppSession::note_stopping` makes a shutdown-time `interrupted` that stop); a server that exits
  before its turn ended reads as failed too (`ended_early`, with the last retried error), unless the
  turn was stopped or ran out of time, or an error the server won't retry was already reported (that
  error stays the result, so an outage still backs off); a turn with no usage notification reports no usage. Its steered `userMessage` item marks it delivered (`steering::mark`); a
  refusal by the server (`no active turn to steer`) marks it not delivered with that reason
  (`steering::refuse`: `SteerRecord.reason`, cut at 200 characters, also in the stored `steerState`; the chat
  shows it, P1-30). On `turn/completed` the
  attempt closes stdin (`steering::close_input`; the server exits on EOF) and a later steer gets
  `STEER_ENDING`. A steer still `sent` then (the turn ended first) goes again to a backoff's next
  attempt, or `settle` marks it not delivered. Stop is the process-group stop, as for exec.
- **A steer** (`chat_steer` → `chat::accept_steer`, the UI's only path; no app action or agent tool
  steers) is stored first (`steering::steer`, then the `steer` event, then `steering::publish`; a turn that
  closed meanwhile refuses it as ending and stores it as not delivered) and only then written: for Claude as a user line with the composer's id as `uuid` (Codex: above). Claude Code holds it until the
  open tool returns (also while an approval card blocks that tool) and folds it into the same turn.
  Its `command_lifecycle` `started` marks it delivered, and `cancelled`, or a turn that ends first
  (`steering::settle`), marks it not delivered. Refusals: `STEER_NOT_RUNNING`, `STEER_UNSUPPORTED`,
  `STEER_ENDING` (input already closed; the composer re-sends it with `after: "queue"`),
  `STEER_INVALID` (empty, over 3,000 characters or 7,000 as JSON, a repeated id, or past 20 per turn),
  `STEER_NOT_STORED`. The UI never shows a code: `chatSteer.ts` `steerRefusalNotice` words each one and
  re-sends the first three as the next message. A late-steer timer closes the input only for its own wait of its own attempt, and only if no steer started a new step since that `result` (`close_if_stalled`); a cancelled steer the input waited for closes it at once.
- **Records.** `steer` / `steerState` events under the running turn, so `StoredTranscript.steers` →
  the assistant row's `steers` in `read_conversation` and `sessions.conversation`. Each steer has
  its `afterTools` (tool calls before it), `at` and `state`. The receipt gets `steers`
  (`ReceiptSteer`, text cut at 280). Task-state folds `TaskTurn.steers` like the person's request
  (always theirs, even on an agent-asked turn). Chat events: `{type:"steer", sessionKey, steer}`,
  once when sent and again as its state moves.
- **Held sends.** `chat_send(…, after: "queue" | "stop")` sends at once when the conversation has no turn queued
  or running and holds nothing; otherwise it holds the send behind any it already holds (`steering::hold`, at most
  5 per conversation) and answers with `heldId`. `stop` first stops the turn (`chat::cancel`). Held sends are saved
  to `held-sends.json` in app support after every change, so a quit doesn't lose them; the next launch sends them
  once their conversation is idle. `commands::drain_held` sends the oldest when the conversation is idle, one per
  turn, under one process-wide order lock (`HELD_ORDER`). It runs right after a chat row's `done` (the pump) and
  from a sweeper every 3 s while anything is held, which covers turns that ended without the pump's hook
  (withdrawn, never started) and lost races. Each outcome is a `chat-event`: `heldSent {heldId, after, queueId,
  clientTurnId}` or `heldFailed {heldId, error, prompt}`. `chat_cancel_held` drops one. A save that fails refuses the hold (or the cancel); an unreadable file is moved aside, never overwritten. A send put back after a lost race is kept in memory even when its save fails, and the sweeper saves it again. Since 2026-11-13 the drain's `steering::take_held` saves nothing: the taken send stays in the file, first in its queue, until `settle_held` removes it after the run queue committed its turn (or it failed for good); a refused one goes back with `put_back`, replacing the taken copy. Its chat row carries the run-queue key `held:<heldId>` (`steering::queue_key`), and a drain that finds a row with that key (`run_queue::row_with_key`, any status) settles the send instead of sending it again, so a quit between the commit and the removal's save doesn't send it twice (the restart recovers that row like any queued turn). A lookup that fails puts the send back for the sweeper and never sends it. The outcome event (`heldSent`, or `heldFailed`, which hands the words back to the composer) goes before the removal, so a quit before the report leaves the send held and the next launch drains it again (at least once). Known limits: a quit while a put-back send's saves keep failing loses it; a quit after `heldFailed` is emitted but before the window keeps the words loses them (the event is not acknowledged); the duplicate check holds only while retention keeps the row (30 days, newest 500).

### Grok over ACP (OC1, 2026-11-18)

A per-agent switch, **off by default**: `agents.acp.grok` (`AgentSettings::acp`, `{"grok": true}`; only
Grok is kept, no Settings control yet, `change_setting` sets it and a Settings save keeps it). On, a Grok
**chat turn** may run `grok --sandbox <profile> agent [--always-approve] --no-leader [--reasoning-effort
<e>] stdio` and speak the Agent Client Protocol (`acp_client.rs`) instead of `grok -p`. Probe and evidence:
the vault's `outputs/hollow-build-run-2026-11-17/oc1-grok-acp-probe.md`.

- **Same events.** ACP updates become the stream-json lines headless prints (`acp_client` module docs
  have the table), so the chat UI, records, receipts and usage see no difference. Usage comes from the
  prompt answer's `_meta.usage` (`inputTokens` is the full prompt there, so headless `input_tokens` is it
  less cached reads and writes); cost from `costUsdTicks` (1e10 to the dollar) when complete.
- **Resume** loads the session (`session/load`, its history replay dropped). A refused load says "Grok
  couldn't load session ... over ACP", which `result_kind` reads as a failed resume, so the chat retries
  fresh as for `--resume`.
- **Model and effort:** `grok agent -m` does not reach a stdio session (probed), so the model goes as
  `session/set_config_option` (a plain string value); effort goes both as the flag and as that option.
- **Stop** ends the process group, as headless. `--no-leader` keeps the agent in that group even when
  Grok's config turns its shared leader on.

Transport per Hollow mode. A mode goes over ACP only when it maps exactly onto headless
(`agent::build_grok`), never looser; anything else stays headless with the switch on
(`acp_client::grok_mode`, `agent::grok_acp_mode`):

| Mode | Transport | Why |
|---|---|---|
| bypassPermissions | ACP | headless `--permission-mode bypassPermissions --sandbox off` = ACP `--sandbox off` (probed to reach the stdio session) plus always-approve (`--always-approve` and `_meta.yoloMode`), Grok's own name for that mode |
| auto | headless | `_meta.autoMode` exists, but a configured always-approve supersedes it on `session/new` (Grok's docs), so it could run looser |
| plan, default, acceptEdits | headless | a stdio session takes no permission mode: `grok --permission-mode` is ignored there (debug log: `permission_mode: None`, the config's mode seeded); ACP can only turn auto or always-approve on |

Also headless with the switch on: a one-shot (team delegates, workflows), a runner or self-hosted turn,
and any run with a `--deny` rule (a top-level `--deny` left no trace in a stdio session).

**Not mapped:** no steer over ACP (`steering` still gives Grok none; a second `session/prompt` probably
queues, unprobed). No Allow/Deny card: a `session/request_permission` is answered with its reject option
(never an allow) and the tool reads as a failed result, as a headless denial does; unproven live, since
the probe's Grok (`permission_mode = "auto"`) never asked. `fs/*` and `terminal/*` get "method not found"
and are never advertised. **Antigravity** (probed live on agy 1.5.13, 2026-11-20) runs its chat turns over `agy acp`
with no switch (`agent::antigravity_acp`; one-shots keep `agy -p`; `HOLLOW_AGY_HEADLESS=1` turns it off). The same
`AcpSession` serves it with its own dialect: config options `model` (a base id), `thought_level` (a refused level is
not a failure) and `mode`; `session/new`'s `mcpServers` per session (probed: a stdio server's tools appear as
`mcp_<server>_<tool>`, and `--settings` rejects `mcpServers`); `session/load` reopens a headless conversation id
with its history; `usage_update {used, size}` becomes `AgentEvent::ContextFigure` (the window is 1,048,576 on Gemini
4); `session/request_permission` becomes `Step::asks`, answered by the person's approval card (`PermissionAsk::reply`,
written to stdin by `chat::spawn_acp_card`) instead of declined. Its prompt `usage` has Grok's shape
(`inputTokens` less `cachedReadTokens`). No steer over ACP yet.

## Claude accounts

`claude_accounts.rs`, Settings → Assistant → **Accounts** → Claude (the
section also holds the other providers' sign-ins, `provider_accounts.rs`). Claude plan
limits (5-hour block, weekly allowance) are per account, so several plans can
be spread to stay out of extra usage.

Accounts are tokens minted with `claude setup-token` (at most 8, stored in
the shared Keychain item). `claude_accounts::apply` sets
`CLAUDE_CODE_OAUTH_TOKEN` and `HOLLOW_CLAUDE_ACCOUNT` on every GUI-owned
Claude command; `CLAUDE_CONFIG_DIR` is never set, so `--resume` works across
accounts. Claude Code's own login is used when no account is added or a connector
explicitly requires it. Configured but unavailable accounts never fall back to
login. Rotation is automatic: refresh stale readings, then choose the most
headroom across both windows, with no affinity preference. Quota rejection
records override misleading meters until reset. See the migration and receipt
schema in [Quota failover](../QUOTA_FAILOVER.md). **Quota
failover (R1):** a Claude chat turn whose account runs out resumes
on each untried eligible account, visibly (`accountSwitched`). An explicit
quota rejection is evidence even when the last usage meter or probe is stale;
a generic 429 still needs an exhausted stream or fresh probe. Auth failures
and provider outages never walk accounts. No eligible fallback names Accounts
and reset recovery instead of claiming every account is exhausted.
See [Quota failover](../QUOTA_FAILOVER.md). **Ceiling
switch (turn guards, 2026-10-09):** a running
Claude chat turn whose own `rate_limit_event` puts its account at or over the
ceiling (past 100% on overage counts; such a turn never errors, so R1 never
fired) moves at the next safe point (the line starting its next main-thread
model request, no tool call open, a tool result since the last request) to an
eligible account under the ceiling: same native session, `CEILING_PROMPT`,
`accountSwitched {reason: "ceiling"}`, receipt "X passed its ceiling". Never
after a question card or `run_in_background` work in that attempt. A quiet
chat notice explains an over-ceiling background-work hold once per attempt;
it does not stop jobs or change provider preferences. ONE usage
row across the move (named for the account the turn finished on): the resumed
terminal is session-cumulative and verifies only against both attempts'
messages. No proactive move after any switch (`MAX_FAILOVERS`); terminal quota
recovery can still use untried accounts. With nowhere
to go means it stays. `chat_resume::{SafePoints, decide_ceiling}`. Added-
account runs only: a chat carrying a claude.ai connector runs on the login
(`apply_login`), which neither fails over nor moves at the ceiling (see the
login override under claude.ai connectors; while the login is out of room a
claude.ai connector set to Always comes only when a message needs it, so other
chats spread, 2026-10-21). Never refresh
Claude Code's OAuth token from Hollow (see the quirks). **In the timeline
(overnight D4):** a finished reply shows its receipt's `switches` as quiet rows
after the reply, above its footer. These cover account moves, agent moves and
the login stay, and each row names the account or agent and why
(`src/accountSwitchRows.ts`). They come back on reload, unlike the live
`accountSwitch` notice. A reply skips one account row for each live notice it
still renders anywhere in the reply. The rows hide while the full receipt is
open, because the receipt lists the same moves.

**Never lose a turn (#16, 2026-10-09; `account_pool.rs`).** Automations fail
over too: a Claude automation whose account runs out (a direct quota rejection, its own rate-limit
event or one probe; `chat_resume::decide_failover`, so outages never move)
restarts once, in the same run, on an eligible account. Workflows keep no
native session (`--no-session-persistence`), so this is a fresh session with
the original prompt plus `account_pool::restart_note`, not a resume; the
exhausted attempt keeps its own ledger row, and the receipt says "X hit its
limit after N tool call(s)". Not after a stop or budget stop, never under an
enforced spending backstop, never from or to the login. Real checkpoint
resume waits for F6. A night-queue item is a chat turn: it fails over inside
the turn through R1 (native resume), not `account_pool::judge` (a restart),
and is long work (`ChatSendRequest::unattended`). **Long work** (`RunExtras::long_work`,
set by automations and night-queue turns) starts away from an account at or above
`long_work_ceiling()` (the CFO's `100 - reserve`, 80% by default; see
`docs/CFO.md`) when a same-organisation account has room, only
with rotation always on. It refuses admission when no account has room; the
CFO also holds unattended starts in the run pump. **Organisations:** each account may
carry an `organization` (`update_claude_account`); failover (chat and
automation) and long-work routing stay inside the exhausted or routed
account's organisation (two unset ones count as one) unless
`accounts.claudeFailoverAcrossOrganizations` (confirm) is on.
`accounts.headroom` (read) lists each account's room, hashed ledger label and
organisation, and where the next chat and next long run would go; the
Settings page doesn't show it yet.

**When an agent runs out (repo ideas 2026-10-27 #8; `agent_out.rs`).** A
chat turn that ended because its agent ran out of usage with nowhere to move
(R1's stop with no eligible account, or a usage-limit result on a turn that
can't fail over: the login, failover off, another agent) says when it can go
on: its closing error starts "Waiting for Claude until 16:40"
(`chat_resume::reset_label`; R1's soonest added-account reset, else the
attempt's last `rate_limit_event`, else the `limit reached|<epoch>` the CLI
printed), or "until its usage resets" with the provider's own words.
Settings → Models → **When an agent runs out** (`whenOut` in
`AgentSettings`, `agents.whenOut`, confirm tier) is Wait by default (absent).
Set to another installed agent, a turn that ran out continues there only when
it changed nothing: no tool call in any attempt (`EvidenceCollector::tool_calls_seen`;
an agent that reports none never continues), the person's own turn (depth 0,
attended, not a COO wake: check-ins, wakes, night lanes and child tasks
wait, so the COO's daily stop and the night guard stay whole), and not
itself a continuation (one move per request). The turn's end leaves the plan
(`agent_out::plan`); after `done` the run pump sends it
(`commands::continue_on`) as the conversation's next message on that agent,
unless the person already queued one, and emits `agentContinued` so the window
shows it as sent. The fresh session there gets the conversation and its
task-state record from the context compiler (the re-grounding); the person's
next message goes to whichever agent they pick.

### 1Password secrets (parity 7.5, 2026-11-20)

An optional second secret path beside the Keychain, **off by default**
(`onepassword.rs`, `commands/onepassword_api.rs`, Settings → Accounts →
1Password). With the switch on, a secret the person points at an
`op://Vault/Item/field` reference is read at the moment it is needed with
`op read --no-newline <reference>` (the 1Password CLI, borrowing the desktop
app's sign-in when Settings, Developer, Integrate with 1Password CLI is on);
the Keychain stays the default and the fallback.

- **State.** `<app data>/onepassword.json` (0600, never the vault):
  `{enabled, refs: {<secret_store key>: "op://…"}}`. A reference names an item
  and is not a secret. Slots a reference can stand in for: the GitHub runner
  token (`runner.github.token`), the Linear API key (`tracker:linear`) and
  each Claude account's token (`claude:<id>`). A damaged file counts as off.
- **Read path.** `secret_store::get` asks `onepassword::lookup` first: `None`
  (off, or no reference for the key) goes straight to the Keychain and never
  runs `op`. A 1Password failure (`op` missing, locked, not signed in, no such
  item, no answer in 30 s) uses the Keychain value for the same key when there
  is one; with none, the caller gets the fixed message, never `Ok(None)`. A
  failure of one reference is remembered 2 minutes; one about 1Password
  itself (locked, not signed in, no answer) is remembered for every key until
  Settings' Check, a setting change or a restart, so no caller, agent action
  included, can re-raise an unlock prompt in a loop. Presence checks never run `op`:
  `secret_store::present` (the GitHub token and tracker key status lines) and
  the Claude accounts' `has_token` (the accounts view and routing). A Claude
  token read through 1Password is not cached in `claude_accounts`; it is read
  again at each use. `op` is found only in `/opt/homebrew/bin` and
  `/usr/local/bin` (owner root or this user, not group or world writable),
  never on `PATH` or under the home.
- **What `op` sees.** A cleared environment: `PATH` plus the Homebrew folders,
  `HOME`, `USER`, `LOGNAME`, `TMPDIR`, locale, XDG folders and the person's own
  `OP_*` variables; empty stdin; the value returns on a pipe and the buffers
  are overwritten after the copy. Never Hollow's, an agent's or a connector's
  tokens. The reference (an item name, not its content) is on `op`'s argv.
  Error text from `op` is only classified, never kept, logged or shown.
- **Redaction.** `claude_runner::redact` also masks, by value, anything read
  from 1Password this run (in memory only, 32 values, 6 characters or more),
  and by shape `op://` references, `ops_` service-account tokens and Account
  Secret Keys, so a value is masked in logs, health lines, transcript
  snapshots and error excerpts.
- **Agents.** `settings.accounts.onepassword_status` (read, never runs `op`)
  and three confirm actions that always ask (`…_set_enabled`,
  `…_set_reference`, `…_clear_reference`, in the secrets class). The Check
  button (`onepassword_check`, runs `op --version` and `op whoami`) is the
  person's only; it reads no secret.
- **Not done.** Writing a secret into 1Password, and caching a resolved value
  across reads. A `hollow serve` with the switch on and `op` installed reads the
  same way.
- **Tests.** `onepassword_tests.rs` fakes `op` with a stub script in a temp
  folder; no test runs the real one or touches a vault.

## Self-hosted models

A self-hosted model is one Alex runs on his own GPU (2026-11-03, brief
`hollow-qwen-agent-brief.md` in the boelab workspace): Qwen3.8 27B on Cloud
Run first, served by vLLM, which speaks Anthropic's Messages API and OpenAI's
Responses API. An `anthropic` entry rides the Claude harness and an `openai`
one (Q9, below) the Codex harness, so every `AgentKind` match stays as it is.
Module: `self_hosted.rs` (tested in `self_hosted_tests.rs`).

**Credential isolation (Q1).** A run whose `RunExtras::self_hosted` holds a
`self_hosted::RunTarget` never gets an Anthropic credential:
`build_with_codex_servers` skips `claude_accounts::apply*` for it, and
`self_hosted::apply_env` (the last env step) removes every inherited
`ANTHROPIC_*` variable, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`, the
Bedrock, Vertex and Foundry switches and `HOLLOW_CLAUDE_ACCOUNT`, then sets
`ANTHROPIC_BASE_URL` (Hollow's loopback forwarder), `ANTHROPIC_AUTH_TOKEN`
(the per-run key the forwarder checks), `ANTHROPIC_MODEL` and the three
`ANTHROPIC_DEFAULT_*_MODEL` aliases (the served id, so subagents and Claude
Code's own small calls go there too), `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`,
`API_TIMEOUT_MS=900000` (a cold start is 4 to 6 minutes) and
`ENABLE_TOOL_SEARCH=false`, and appends the loopback hosts to `NO_PROXY`
and `no_proxy` (an inherited proxy never carries the key). Every inherited
or set variable starting `ANTHROPIC_` or `CLAUDE_CODE_USE_` is removed by
prefix. `--model` is the served id whatever the spec says. A settings file's
`env` (`~/.claude/settings.json`, a project's `.claude/settings*.json`) is
applied over the process env, so the run also gets
`--settings` with `self_hosted::settings_json`: the base URL, the model and
aliases pinned, every credential and provider switch blanked, and no secret
(it rides argv; command-line settings outrank user, project and local ones;
probed with Claude Code 2.1.317: an env base URL on a closed port and an env
API key were both overridden; `NO_PROXY` is pinned there too). Managed
settings (`/Library/Application Support/ClaudeCode/managed-settings.json`)
outrank `--settings`; Hollow can't override an organisation's policy. The
account decision is the pure
`agent::account_route` (`AccountRoute::None` for a self-hosted run). The run
key is a `self_hosted::RunKey` (redacted `Debug`, zeroed on drop) that must
be `hsr_` plus 32 or more lowercase hex digits (`self_hosted::is_run_key`), and
`claude_runner::redact` masks that shape wherever it appears. The build fails closed (the `/bin/sh` stand-in with the reason) for an
agent the entry's API style doesn't run on (`self_hosted::wrong_harness`), a
remote (Mac mini) target, a base URL that isn't
`http://127.0.0.1` or `http://[::1]` (not `localhost`, which the hosts file
resolves), or a target with no run key or model id. The agent never
sees the endpoint's own token. Limit: macOS `ps -E` shows a non-Apple
process's environment to the same user, so the run key is readable by the
person's own processes while the run lives; the forwarder expires it when the
run ends. `Debug` of a std or tokio `Command` prints its set env, the key
included (as it would an OAuth token), so never log a built command.

**Why tool search is off (Q0, 2026-11-03).** With it on, Claude Code defers
MCP tools and answers `ToolSearch` with `tool_reference` blocks, which vLLM
refuses ("Unexpected item type in content"); Claude Code recovers after three
failed calls by sending every tool. Off, every tool goes in full on the first
call: about 52K tokens of Qwen's 131,072 for a Hollow chat turn.

**The forwarder (Q2).** `self_hosted_forwarder.rs` (tested in
`self_hosted_forwarder_tests.rs` against a stub upstream): one listener per
endpoint on `127.0.0.1` and a free port. `Forwarder::mint(ttl)` makes a
`RunKey` (32 random bytes, at most 24 h, wall-clock expiry so a sleeping Mac
doesn't stretch it); a request without a live key in `Authorization: Bearer`
or `x-api-key` (either may match), or with an expired or revoked one, gets an
Anthropic-shaped 401 right after its headers, before its body is read, and
never reaches the endpoint. Limits (`Limits`): 30 s for a client's headers,
5 minutes for its body (64 MB, `Content-Length` or chunked with checked
arithmetic and a 1 KB size line), 64 connections at once (more get 503).
Nagle is off and each chunk frame is one write. No redirect is followed (a
302 goes back to the client), and a client that leaves while the endpoint is
still starting ends the upstream call (a client that only half-closes its
write side counts as leaving; Claude Code never does). Refusals close our
side and drain briefly, so the client reads the message, not a reset. An
accept error (a client reset before the accept, no descriptors) is logged and
the loop goes on; `ensure` restarts a forwarder whose loop ended. Limit: the
connection cap also counts keyless connections still sending headers, so a
local process holding 64 silent sockets for 30 s makes runs get 503 meanwhile. Only GET and POST go
up; only `content-type`, `accept`, `anthropic-version` and `anthropic-beta`
are forwarded, so the client's key stays here. The endpoint's token comes from
the entry's token command (run without a shell, `PATH` from `cli_path`, one
minute at most; its first stderr line is the error) and is cached in memory
for 50 minutes; an upstream 401 drops it (only if the cache still holds the
refused token, so many 401s at once run the command once), fetches a fresh
one and retries once.
Responses stream back chunk by chunk (chunked encoding, `connection: close`),
with up to 15 minutes for the upstream's headers (a cold start) and 10
between chunks. A failure the client should see is a 502 whose message names
it (the token command's stderr, an unreachable endpoint). Logs (`[self-hosted]`)
hold method, path without query, status, byte counts and timings, never a body,
a header value, the key or the token; the module writes no file. `ensure`,
`get`, `stop` and `stop_all` keep one forwarder per entry for the app
(`stop_all` runs in `lib.rs`'s `stop_children_once` at quit); an entry whose
upstream changed (compared trimmed, without a trailing slash, one start at a
time) gets a fresh forwarder, and the old one keeps serving its live keys
until they end. `Forwarder::get(path)` is Hollow's
own call to the endpoint (health ping, model list) with its token and no key.
Since the Q4 review a shutdown also closes the forwarder (`is_closed`): an
in-flight `get` and the token command under it end at once with `STOPPED`,
and `send_upstream` starts nothing more; retiring a replaced forwarder is not
a shutdown, so a run's proxied stream outlives an edit. `stop` counts per
entry (`stops`), and `ensure_since(upstream, since)` refuses (shutting down
what it just started) when a stop came after the caller's reading, checked
again under the map lock when it registers, so a remove or switch-off during
a start never leaves a forwarder registered; `ensure` is `ensure_since` with
a reading taken as it is called.

**Effort and context (Q3).** A self-hosted run's effort comes from its
entry's levels (`RunTarget::effort`, used by `agent::run_effort` and the
turn's receipt): the saved level mapped to the nearest one the endpoint takes,
a tie going to the lower level, `medium` when none is saved, ultracode read as
`max`. For Qwen (`low`, `medium`, `xhigh`): low to low, medium and high to
medium, xhigh and max to xhigh; `high` is never sent (vLLM refuses it).
`self_hosted::remember(served id, window, levels)` also registers the levels
with `effort::remember`, and `effort::resolve` checks them ahead of every Claude
rule. The window follows Claude Code's model-config doc ("Correct the window
for a gateway or custom model ID"): `CLAUDE_CODE_MAX_CONTEXT_TOKENS` is the
entry's window (Claude Code otherwise assumes 200K for an id it doesn't
know), `CLAUDE_CODE_AUTO_COMPACT_WINDOW` is `RunTarget::compact_window` (the
window less a 32,000-token answer budget, at least 100,000, Claude Code's own
floor; Q4 refuses windows under 110,000, so 8,192 or more is always left for
the answer), over Settings' own compaction window
(OB22), and `CLAUDE_CODE_MAX_OUTPUT_TOKENS` is the rest, so a full prompt and
a full answer make exactly the window: 100,000 and 31,072 for Qwen. All three
are pinned in `--settings` too, and `agent::compact_window` (the run manifest)
reports the entry's. The context gauge knows a remembered window exactly
(`context_gauge::context_window` and `exact_window`; `register` treats it like
a reported one, so growth steps are 70% and 85% of it), so `sessions.context`
measures a Qwen chat against 131,072, not 200K, and the stored figure
(`chat::store_context_figure`) does too. A chat turn's composed prompt is cut
to fit (`self_hosted::fit_pack`): Hollow's message plus the appended prompt
may hold the compact window less 45,000 tokens of Claude Code's own (its
system prompt, tool schemas, CLAUDE.md files; about 42K in Q0) at 3
characters a token, 165,000 characters for Qwen; a longer one keeps its head
and its last two fifths (the person's message), and a note replaces the middle
and goes to the run manifest's fallbacks; at least the last 2,000 bytes always
stay, cuts move to a line end within 2,000 bytes, and a fresh retry's prompt
is cut the same way. A cut prompt records no per-turn layer or visual guide
as given (`pending` and `guide_given` cleared), so the next turn sends them
again. Q0's first call was about 52K tokens in all, so a normal turn is never
cut. The cap covers a turn's own prompt only; what a resumed session already
holds is Claude Code's to compact (at the compact window). An ultracode
request runs at the top level the endpoint takes, with a note on the receipt.
A chat turn on a self-hosted target calls `remember` before it registers with
the gauge, under the served id.

**Registry and Settings (Q4).** `self_hosted_registry.rs` (tested in
`self_hosted_registry_tests.rs`) keeps the endpoints in
`self-hosted-models.json` beside `agent-settings.json` in app data, written
atomically: `{"version":1,"entries":[…]}`, each entry `id`, `name`,
`baseUrl`, `apiStyle` (`anthropic`, or `openai` since Q9), `servedModel`,
`contextWindow`, `effortLevels`, `auth` (`{"kind":"tokenCommand","program":…,"args":[…]}`
or `{"kind":"none"}`), `healthPath` (default `/health`) and `enabled`. A
missing file is an empty list. Another version, or a file that can't be read
or parsed, is the list's `error` and refuses every change, so Hollow never
overwrites it. Every save is checked (`normalize`), one plain message per
rule: the id matches `^[a-z0-9][a-z0-9-]{0,31}$` and is new on add (an edit
keeps its id); the name is 1 to 80 characters with no control character;
the base URL passes `upstream_url_problem` (`https://`, or `http://` to this
Mac; parsed with `reqwest::Url`: a host, no user name or password, no query
or fragment, no space or control character) and loses a trailing slash; the
served model is 1 to 128 characters with no whitespace or control character,
doesn't start with `-` (Claude Code would read it as an option), is not an id
Claude Code uses for its own models (anything starting `claude`, or
`default`, `best`, `opus`, `sonnet`, `haiku`, `fable`, `opusplan`, any case,
with or without a trailing `[…]` such as `opus[1m]`: Hollow keys an entry's
window and effort levels by served id, so a collision would change ordinary
Claude runs), and holds no `[` or `]` (Claude Code reads a trailing `[1m]` as
its window suffix); the window is 110,000 to 10,000,000 tokens (Claude Code compacts
at 100,000 at the earliest and a Hollow chat turn's first call is about
52,000); the effort levels are a non-empty subset of `effort::LEVELS` with no
repeat, stored lowest first; a token command has a program, at most 16
arguments and no NUL; the health path starts with `/`. `preset()` is Qwen3.8
27B on Cloud Run (`qwen`, 131,072 tokens, low/medium/xhigh, `gcloud auth
print-identity-token`), offered by the form and saved only when the person
adds it. Commands (`commands/self_hosted_api.rs`, all user-only in
`docs/HOLLOW_APP_COMMAND_MAP.tsv`, so no agent action reaches them):
`self_hosted_models`, `save_self_hosted_model` (`edit` false adds, true
replaces by id), `remove_self_hosted_model` and `set_self_hosted_model_enabled`
(both stop the entry's forwarder when it goes or switches off, as does a save
with `enabled` false) and `test_self_hosted_model`. The four that change or
test answer Hollow's main window only (`webview.label() != "main"` is
refused, as for PDF preview and canvas items: a `md-preview-*` window can
call app commands). Test (`test_at`, on the saved entry) reads
`forwarder::stops` first, then the entry, checks it with `normalize` (a
hand-edited entry that breaks a rule is refused before any call), and runs
`test_entry` through `ensure_since`, so a remove or switch-off after that
reading ends it: the health path (up to the forwarder's 15 minutes on a
cold start), then `/v1/models` (`modelsStatus`; `modelListed` is whether it
lists `servedModel`, null when the list couldn't be read: no answer, not 2xx,
or not a list; and its `max_model_len`: `windowMismatch` when it differs from
`contextWindow`), then
one completion exactly as a run sends it: a run key minted for 2 minutes,
`POST /v1/messages` with `anthropic-version: 2023-06-01` and `Bearer <key>`,
`max_tokens` 1, timed to the response headers, and the key revoked at once.
`ok` means the health check and the completion both answered 2xx; an unlisted
model, an unread model list or a window mismatch is a warning beside it. The
result (`SelfHostedTest`: `ok`, `healthStatus`, `healthMs`, `modelsStatus`,
`modelListed`, `reportedWindow`, `windowMismatch`, `firstByteMs`, `status`,
`error`) holds neither the token nor the key, and the log line holds statuses
and times only. A stop during a Test (switched off or removed) closes the
forwarder, so the waiting health check or model list and its token command end
at once, no completion is sent, and the error is `forwarder::STOPPED`. When
the entry was off as the Test began, `stop_unless_on_at` reads it again under
the file's lock afterwards and stops the forwarder only if it is still off,
gone or unreadable, so switching it on during the Test keeps the forwarder
(and any retired one still serving a run). Settings →
Models → **Self-hosted models** (`settings/SelfHostedModelsPanel.tsx`, a lazy
part with id `selfhosted`; helpers in `selfHostedModels.ts`) lists each
entry with its served model, window and sign-in, an on/off switch, Test, Edit
and Remove (named "Test Qwen3.8 27B" and so on for assistive tech); Add a
self-hosted model (the Add… button's name) opens the form (Use the Qwen on
Cloud Run preset fills it; its switch is Use this model), and a running Test
reads "Starting the model, this can take about 5 minutes." until its health
and first byte times or its error show ("Couldn't read the model list (404)."
when it couldn't). While a row's Test runs its switch, Edit and Remove are
off; while the form is open every row's switch and Edit are off, so the form's
copy of the switch is never stale and a draft is never replaced. A result is
keyed to the entry it tested without its switch (`testKey`): a saved edit or a
remove clears it, and one whose entry changed while it ran is dropped. The
form refuses a window outside the list's limits before sending it. A file the
list can't read shows its error with Check again, and a failed change reads
the list again.
A save, switch or removal there drops the app's cached Claude model list
(`forgetCatalog('claude')`), so a chat's model menu reads it afresh.

**The run path (Q5).** A chat's model id for an entry is `self-hosted:<id>`
(`self_hosted::MODEL_PREFIX`, `model_id`, `entry_id`). Claude's model list
(`agent_models::catalog_with`, through `with_self_hosted` and
`self_hosted_options`) ends with every switched-on, Anthropic-style entry as
"<name> · self-hosted" ("Qwen3.8 27B · self-hosted"), read from the registry
(`self_hosted_registry::entries`) on every call and never cached with Claude
Code's own list, so a switch in Settings shows at the next read; each listed
entry's window and levels are `remember`ed under its served id, so effort
resolves only to a level it takes (startup's `warm_effort_catalogs` reads the
list too). A switched-off entry isn't listed, and a file that can't be read
adds nothing (the list never fails for it). Reading the list never contacts an
endpoint. The pickers whose choice isn't a chat leave these rows out
(`withoutChatOnly` in `modelCatalog.ts`: `ModelPicker`, used by Settings'
default model per agent and an automation's model override, and the
Extraction model's suggestions); the composer's model menu and `/model` list
them. A chat turn resolves its model in `execute_turn`, after the effort read
and before the runner and the template (`self_hosted::turn_route`, a pure
`route` plus `ensure` and `turn_run`): an ordinary model never reads the
registry; a self-hosted one on Claude whose entry is saved, switched on and
`anthropic` gets the entry's forwarder (started if it isn't running; only a
loopback listener, the endpoint isn't contacted) and a run key minted for the
turn (`TURN_KEY_TTL`, 12 hours at most), held in a `self_hosted::TurnRun`
until `execute_turn` returns, whose drop revokes the key, so it ends however
the turn ends (an answer, an error, Stop, a dropped future). The template's
`model` is then the served id and `extras.self_hosted` the target, so the
early start, every attempt and a fresh retry carry the same target and key.
Anything else ends the turn before any agent starts, as a failed turn whose
message says why and names Settings → Models → Self-hosted models or the model
menu: no such entry, switched off, a registry that can't be read, an API style
Hollow can't run, or an agent the style doesn't run on (Q9: Claude for
`anthropic`, Codex for `openai`); it never runs on another model instead. Any other run (an automation, a one-shot, extraction) given a
`self-hosted:` id with no target is refused by `agent::validate_run_spec`
(`self_hosted::chats_only`), so the id never reaches the agent's own provider.
The run never changes account or model: `drive_turn_attempts` takes no
account for a self-hosted attempt, so R1's failover and the ceiling switch
never act on it (Claude never backs off either), and a `ProviderQuota` result
from the endpoint isn't read as Claude running out (no "Waiting for Claude"
stop, no continuation on another agent). Where it runs is Q7's (below):
always this Mac. Switching models: the chat's own model id
(`self-hosted:qwen`, never the served id) is the stint fingerprint's `model`
part (`chat_resume::PARTS`, `execute_turn`'s `input_at`), so a switch
between a self-hosted model and an Anthropic one, either way, or between two
entries, is a config-changed reset naming
`model`: Claude Code never resumes a session holding the other side's
thinking blocks (Anthropic's carry signatures, Qwen's don't). Limit: editing
an entry's served model under the same id keeps the session. Tested in
`self_hosted_tests.rs` (ids, routes and refusals, `validate_run_spec`, the key
guard), `agent_models.rs` (the menu rows) and `chat_self_hosted_tests.rs` (a
whole turn through `drive_turn_attempts`, the real builder, registry and
forwarder against a stub Anthropic-style endpoint, with a shell stand-in for
Claude Code that calls `mcp__hollow_tasks__list_tasks` and answers; Stop
mid-call; no account move; the reset on a switch). Not covered there: Claude
Code's own requests and SSE parsing, and Hollow's MCP server answering the
call (the stand-in reports it as Claude Code does); the `#[ignore]`d
`real_claude_code_calls_hollow_tasks_through_the_forwarder` covers both with
the installed `claude` and the built `hollow` binary (`cargo build --bin
hollow`, then `-- --ignored`), at no cost since no call leaves the Mac.

**Q5 review.** What else keeps a self-hosted chat off Anthropic and the run
key in its own run. `self_hosted::chat_on_self_hosted` (Claude or, since Q9,
Codex, and a `self-hosted:` model, checked before the route resolves) decides three things
in `execute_turn`: claude.ai connectors (`claudeai-proxy`, Anthropic's proxy
on Claude Code's own login) are left out of the turn
(`connectors::without_claude_login`), with a
`<hollow-connectors-set-aside>` note naming them and saying to switch the chat
to a Claude model for one, in place of the login-room note (the builder also
refuses a self-hosted spec carrying one, `self_hosted::claude_login_servers`);
the model retitle and the learning extraction, which run on the extraction
model, are skipped, so the chat keeps its first title and teaches nothing
(Q10 runs them on the chat's own entry when Background jobs sends that kind
there; see below). `apply_env` marks the run
with `HOLLOW_SELF_HOSTED=<entry id>` (`self_hosted::MARKER_ENV`), and the
builder writes the same variable into the config `env` of each Hollow-owned
(`hollow_*`) MCP server (`self_hosted::with_marker`, as the run tag is), so
`hollow team-mcp` sees it whatever the CLI passes its servers; inside such
a run (the marker, or a run key as `ANTHROPIC_AUTH_TOKEN`,
`self_hosted::inherited_from`) `hollow team-mcp` refuses a Claude teammate
(`self_hosted::NO_CLAUDE_TEAMMATE`: switch the chat's model to ask one), and
the command builder strips every self-hosted variable
(`self_hosted::INHERITED_ENVS`: the marker, base URL, run key, served model
and aliases, the window, compact window and answer budget, and the other
settings `apply_env` sets) from any run with no target before its own extra
env, so another teammate never gets the key. The template's model and target
come from one call (`self_hosted::turn_spec_model`, pinned in source), and
`validate_run_spec` also refuses a Claude run on a known endpoint's served id
(`self_hosted::needs_target`) without its target. Every reader checks a
saved entry with `normalize` before using it: the menu leaves out one that
fails, the route refuses it with that reason ("… can't be used as saved"),
and Settings lists it with the same reason under its row
(`SelfHostedModels::invalid`, by id), so a hand-edited `sonnet` or `claude-…`
served id is never routed or remembered and never looks absent.
`turn_route` reads the entry's stop count before the registry, starts the
forwarder only if no stop came since (`self_hosted_forwarder::ensure_since`),
then reads the registry again: an entry now off or gone stops its forwarder
and refuses the turn; a file that became unreadable or an entry now invalid
refuses this turn only (other turns on the forwarder go on); one whose
address changed asks to send it again. The setters that aren't a chat refuse
a newly chosen `self-hosted:` id (`self_hosted::not_for_this`): the default
model (`agent_settings::set_settings` refuses a new one, while `normalize`
drops one already saved, with a log line, so the composer's Effort row,
which re-saves the whole object, still saves), the extraction model (and
`extract::model_for` and `settings.extraction`'s view leave one out), and an
automation's model (`vault_workflows::check_input`). Billing: Q8 (a separate
item) keys a run's `Billing::SelfHosted` on its command carrying a run key as
`ANTHROPIC_AUTH_TOKEN` and its endpoint on `extras.self_hosted`, both of
which every Q5 turn's command and spec carry; the stand-in reports a nonzero
`total_cost_usd` so Q8's checks bite.

**Wake and status (Q6).** A chat on a self-hosted entry has the model id
`self-hosted:<entry id>` (`selfHostedModels.ts` `selfHostedEntryId`). The
endpoint scales to zero after about 10 idle minutes, takes 4 to 6 to start
and costs money while up, so only the person wakes it (Alex's rule,
which replaced the brief's wake on open): the composer's input event and a
send (`store/selfHostedWake.ts` `wakeFromComposer`, at most once per
2 minutes per entry) and the chip's Start now (at once). Typing a `/`
command (`/model`, `/new`) or emptying the box doesn't count
(`typingWakes`); a send does, through the send controller's `followSend`
hook, which a local command never reaches. Opening, focusing, scrolling or
reading a chat never wakes one; a vitest guard lists every call site in
`src/`. `wake_self_hosted_model` (`self_hosted_wake.rs`, user-only in the
command map and `app_actions::USER_ONLY`, Hollow's main window only)
resolves the entry (missing, switched off, invalid or an unreadable
registry each refuse with a message naming Settings → Models → Self-hosted
models), then `ensure`s its forwarder and GETs its health path through
`Forwarder::get` (the endpoint's token, no run key), waiting up to
16 minutes for a cold start; it returns `SelfHostedWake` (`status`, `ms`,
`error`, `shared`, `startedMsAgo`), never a token or key, and logs status
and time only. The backend throttles too (`take_turn`): one wake per entry
per 2 minutes counted from its start; a call while one runs joins it
however long it takes, a call within the 2 minutes gets that wake's answer
(`shared` true, nothing sent), failures included, and a wake whose task
died is never joined. The wake runs on its own task, so a window reload
doesn't cut it short. `self_hosted_wake_status` (user-only, main window)
reads the last wake's age and answer from memory, never calling the model.
Saving, switching or removing an entry in Settings forgets its last wake
(`forget`, called by the registry), so the next keystroke wakes the entry
as it now is; the frontend hears of the same three changes through the API
wrappers Settings calls (`api.ts` `onSelfHostedChange`), so a chip still on
screen re-reads the entries (name, window) and starts that entry's wake
state and 2-minute wait over, dropping any answer still to come. The chip
(`components/SelfHostedStatus.tsx`, a lazy chunk above the composer;
state, words and clock in `selfHostedWake.ts` `createWakes` and
`createStatusView`, shared by every chat on one entry) reads the saved
entries and watches the backend's last wake while it is mounted (`watch`;
a start already running after a reload shows with its real elapsed time
and is read every 2 s until it answers; a failed read is tried again after
4, 8, 16 then every 30 s; such a start ends 17 minutes after it began
whatever the reads say, and unmounting stops the reads). It is idle until a
wake ("Qwen3.8 27B sleeps when idle" with Start now), "Checking…" for the
first 3 s of any wake, then "is starting, about 4 minutes" with the time
so far as text ("still starting" past 6 minutes), "Qwen3.8 27B is ready"
for 10 minutes after the health check answered, or the error, for
10 minutes, with Start now disabled until 2 minutes after that wake's
start. Times are anchored to the wake's own start (`startedMsAgo`), so a
shared answer doesn't stretch Ready or restart the wait. A refusal (the
invoke rejected: missing or switched off) woke nothing, so it leaves the
2-minute wait alone; keystrokes ask again after 10 s and Start now at once.
A message sent meanwhile simply waits: the run's `API_TIMEOUT_MS` is
15 minutes. The dot pulses only while starting, under
`prefers-reduced-motion: no-preference` and not `data-motion="reduced"`,
and pauses while the window is away (`--ambient-play`). The composer's
usage ring passes the entry's window (`selfHostedChatWindow`, 131,072 for
Qwen) where it would infer 200K. `tests/smoke/self-hosted-wake.spec.ts`
walks it in mock mode (not run by the Q6 builder).
**Usage (Q8).** Billed by GPU time, never per token: see
[runs](runs.md#usage-ledger-paperclip-pr-6) (`Billing::SelfHosted`). The row
keeps the served id as both model and requested model
(`UsageRun::set_self_hosted`, on every attempt, only once billing says
self-hosted, so a build failure's stand-in gets neither) and no price; the
transcript's `result` stores no `costUsd`. GPU time is no Anthropic spend and
never unknown: a night turn's guard estimate is 0, its row neither charges
nor fails closed the night budget (`CostRollup::gpu_time`; the morning report
adds "(plus N billed by GPU time)"), and budgets count it under Global only:
an `agent:claude` budget leaves out its tokens and runs (ledger rows and live
reservations), and no spending budget is a lower bound for it or lists it as
having no usage figures. The usage popover shows the endpoint instead of the
Claude account and plan, keeps it while the next turn runs, and knows a chat
model id starting `self-hosted:` (Q5) is one; Settings → Usage gives it its
own By agent row, "Billed by GPU time". A person's own gateway (a base URL
and a token that isn't a run key) stays unknown billing.

**Runs on (Q7).** A self-hosted chat turn runs on this Mac whatever Runs on
says, and never refuses for it: Hollow serves the model through this Mac's
loopback forwarder, which the runner can't reach, and signs in to its
endpoint here (a gcloud token command, say). `remote_session::turn_place`
decides a chat turn's place once in `execute_turn`, right after the resume
state is read and before a stale remote turn's choice, the fingerprint's
folder (`fingerprint_cwd`) or any preparation reads it: the unattended
default (`unattended_runner`, when the turn `prefers` it: N6's unattended or
worker turn starting its native session), else the person's own choice
(`runs_on`: a build workspace's own Runs on, from its Changes tab or a new
conversation's empty state, a vault workspace's own, or Settings → Devices →
Runs on, which Overnight and worker chats on Follow Runs on also use). For
a self-hosted chat (`self_hosted_chat`, the Q5 review check) both come
back empty and `TurnPlace::self_hosted_here` holds the host it would have
gone to (empty when the runner is chosen but none is set up), so no
worktree or folder is synced, `begin_turn` isn't called, no `start` notice
is recorded, and the fingerprint is this Mac's folder (a later turn on the
same model resumes the session; switching to a Claude model resets on
`model` anyway). The route (`turn_route`) resolves right after the place,
so a refused route ends the turn before anything else and says only why
(a stale remote turn then waits, still going there, for a later turn).
The one call such a turn makes to the runner is Overnight R1's: a remote
turn this chat had under way when Hollow quit is stopped and brought back
first (`settle_stale`, its host from `chat::stale_host`, none for such a
turn, so even a Continue brings it back), as for any message, so this turn
starts from its work. What to record then comes from one function,
`chat::settled_notices`: for a turn with `self_hosted_here`, its note
(`self_hosted::runner_note`) first, then the earlier turn's return
(`remote_back_notices`, the notices `record_remote_back` records; an
earlier Hollow's turn writes no History entry), so the note is the turn's
first line; for any other turn, the return alone. The note is a
`remoteRun` notice of phase `stayed` with `selfHosted: {model, signIn,
leftOnRunner}` (`sign_in_kind`: `gcloud` when the token command's program is
`gcloud`, else `command` or `none`), recorded and emitted like N6's stayed
notice (`record_remote_notice`). `leftOnRunner` (Q7 review) is `pending`
when the stale turn's record is still there after the settle (`still_stale`,
read again after it: the runner didn't answer, or wasn't asked because it
didn't answer minutes ago; a later message tries again, and its run may
still be going); else `runner` when the morning report's rule
(`night_queue::left_on_runner`) over the chat's start and back notices
(`session_store::remote_run_trail`, newest 40 of those two phases only, so
a self-hosted chat's own `stayed` notes never push the last `start` out)
and the earlier turn's return finds work still there that only a turn there
brings back (a failed return, or a run that was gone); else null.
`remoteRunNotice.ts` words it: "The runner (runner-host) can't use Qwen3.8
27B: Hollow serves it through this Mac's local forwarder and signs in with
this Mac's gcloud, so this turn runs on this Mac." (a token command: "and
signs in with its token command on this Mac"; none: no sign-in clause). It
never says "no Google Cloud login" (Q7 review: a Google Cloud runner has a
service account of its own, `runner_setup`). An empty host has its own
sentence, "Runs on is the remote runner, but none is set up on this Mac;
Qwen3.8 27B runs on this Mac either way, so this turn runs here."
`pending` adds "The turn that was running on the runner when Hollow quit
wasn't brought back: the runner didn't answer (or wasn't asked, because it
didn't answer minutes ago), so its work is still there and it may still be
running; a later message in this chat brings it back once the runner
answers." (Q7 follow-up: it said "couldn't be reached", though a runner
that didn't answer minutes earlier isn't asked again); `runner` adds "Work an earlier turn left on the runner stays
there until a turn there on a model that isn't self-hosted brings it back." The morning
report's own line adds that a turn on a self-hosted model runs on this Mac,
so use a model that is not self-hosted for it, only when the conversation's newest turn
asked for a self-hosted model (`session_store::last_requested_model`, else
the item's model). The morning report reads the same trail (Q7 follow-up:
it read the newest 40 `remoteRun` notices of any phase, so after 40 turns
of a self-hosted chat whose Runs on is the runner it stopped saying work
was still there). The Runs on hints say a self-hosted model's turns stay
here (`runsOnHint.ts` on Remote runner; Settings' Runs on and Overnight and
worker chats help). Q5's arm that kept a preferred self-hosted turn here
with N6's stayed reason is gone. Runs on itself is unchanged, so a Claude
model's turn with the same settings goes to the runner as before, and Codex
and the other agents are untouched. The builder's refusal of a self-hosted
spec with a remote target ("runs on this Mac only") stays as the last line
of defence. What placing reads besides the vault's stores is
`remote_session::PlaceReads` (`LIVE`: the settings file and
`configured_runner`), so tests give their own: the settings file is shared
by the test process. Tested in `remote_session_tests.rs`
(`a_self_hosted_turn_stays_here_whatever_runs_on_says`: each place Runs on
is chosen, This Mac with no note, no runner set up, and the Claude-model
turn beside each; `an_unattended_self_hosted_turn_following_runs_on_stays_here`:
Follow Runs on with Settings' Runs on the Mac mini), `self_hosted_tests.rs`
(the sign-in kind and the note, which holds no key or address),
`chat_self_hosted_tests.rs` (the source pin: one `turn_place` with
`self_hosted_chat` before the stale turn, the fingerprint and every
preparation call; the route before the stale turn; `settled_notices` after
the settle and before the fingerprint and `wanted`, its notices recorded in
order, and given `still_stale` read again between the two; `a_self_hosted_turn_gives_a_stale_turn_no_host`;
`a_settled_stale_turn_records_the_note_first_and_says_what_is_left`, each
outcome), `session_store` (`the_remote_trail_reads_starts_and_returns_past_any_number_of_notes`),
`night_queue.rs` (the morning report's line, with and without a
self-hosted model, and past 45 `stayed` notes), `remoteRunNotice.test.ts` and `runsOnHint.test.ts` (the
wording).
**Background jobs (Q10).** `self_hosted_jobs.rs` (tested in
`self_hosted_jobs_tests.rs`) sends Hollow's own model calls to an entry per
job kind (`JobKind`): Recording extraction (`extract::ConfiguredModel`, so
Keep transcript's background run, `extract_run` and `extract.run`), Chat and
session titles (`chat_title`'s retitle, `sessions::generate`), Learning
extraction (`vault_learning::propose_from_exchange`), Meeting recaps and prep
packs (`prep_pack::AgentWriter`: Prepare, Recap and `agenda.prepare_meeting`),
About-you scan (`profile_builder::scan`) and Heartbeat (`prepare_beat`).
Person-started helpers (today brief, ask, inline AI, ingest) are unchanged;
the gardener and nightly passes make no model call. The setting is
`self-hosted-jobs.json` in app data, `{"version":1,"jobs":{kind: entryId}}`,
written 0600: no line is "As now". A missing file is As now for every kind. A
file that exists but can't be read, has another version or no version, or an
app-data folder Hollow can't find, fails closed (review): every kind is
skipped with a note naming the file, Settings says background jobs are held
until it is fixed, marks every row held and still shows the routes it can read
of a file that parses (`peek_jobs_at`, never used to route), and every change
is refused, so the file is never overwritten. A change must name a listed,
switched-on, `anthropic` entry; unknown keys are kept; sending a kind to an
entry moves its waiting jobs there (`move_kind_at`). Both files are written
under an flock on a `.lock` file beside them as well as a process lock.
Commands (`self_hosted_jobs`, `set_self_hosted_job`,
`run_self_hosted_jobs_now`, `run_self_hosted_job_as_now`,
`retry_self_hosted_job`, `discard_self_hosted_job` in
`commands/self_hosted_api.rs`) are user-only
(`USER_ONLY`, the command map), and every one that changes something answers
the main window only. Routing (`decide` then `attach`, together `route`): a
kind with no line runs its existing path untouched and never reads the
registry (each call site's As now arm is its old call, pinned in
`every_call_site_keeps_as_now_and_errs_when_held`; `claude_runner::oneshot_spec`
gives the same spec, pinned against the pre-Q10 literal). A routed kind whose
entry is missing, switched off, of another API style or in a list that can't
be read is skipped with a note in its own status or log (the job's error:
extraction's and learning's log lines, Prepare's and Recap's message, the
About-you scan's last error, the heartbeat's log as `skipped`, at most hourly
per reason), and Settings' row shows the reason; it never runs on Anthropic
instead. A routed job runs only when its entry answered within 8 minutes
(`self_hosted_forwarder::last_answered`: the entry's current forwarder notes
the wall-clock time of each 2xx response header, a run's call or Hollow's own
GET, Q6's wake included, and wakes `answered_signal`; a stop or a new address
forgets it) or a drain woke it and is running (`awake`). Then it gets the
entry's forwarder (`ensure_since` with the stop count read before the
registry, then the registry read again), and a key from `self_hosted::run_for`
that lives an hour at most and is revoked when the `TurnRun` drops, and runs
on Claude Code on this Mac whatever the default agent is: `run_on` passes
Claude, the served id and `RunOpts::self_hosted`, which `oneshot_spec` puts on
`RunExtras::self_hosted`, so Q1's builder removes every Anthropic credential.
What a routed job (or a self-hosted chat's own learning) proposes never goes
to Jev: `jev_decide::Job::kept_private` makes `check` send nothing, for
learning, extraction, Recap's proposals and a routed beat's tasks. The
heartbeat decides in `prepare_beat` (before the lease and the budget charge;
the routed beat is Claude's tool pass) and attaches in `run_beat`, holding the
run until it returns; a failed attach ends the beat as an error before
anything starts. Otherwise the job waits: `self-hosted-job-queue.json` in app
data (`{"version":1,"jobs":[…]}`, written 0600 since a job can hold a chat's
words; a damaged file skips jobs that would wait and is never reset), each a
`Job` holding what reruns its own entry point (extraction's vault and source,
a retitle's session and turn, a learning exchange with each side kept to
8,000 characters, Prepare's or Recap's meeting and flags, the scan's or
heartbeat's vault) with its `attempts`, `last_error`, `retry_at` and `claim`,
at most 200, the same job (`Job::key`) once per entry. A new copy replaces
its details (`Job::merged`) and keeps its attempts, error and backoff, so
asking again never cuts a backoff short or stops a job from giving up
(review round 3). Nothing automatic weakens what waits (Q10 follow-up): an
automatic copy merged into a Prepare or Recap rewrite keeps the rewrite
whole (`regenerate` and the person's `expected_hash`), where it used to
leave a plain request that `still_wanted` dropped once a pack was kept, so
the person's rewrite never ran; a person's own newer request still
replaces it. A job that gave up starts over only on a person's request
(review round 4: `route` and `decide` take `Asked`; `Asked::Person` is
Prepare, Recap, Keep transcript and an About-you scan they start, and the
`extract_run` command, which no screen calls), attempts and error cleared,
waiting from the request as a new job would; an automatic one
(`Asked::Auto`: an agent's or check-in's call, the end of a chat turn, the
session list's title backfill, the About-you timer, and the heartbeat, its
Run now included) joins it and leaves it for Try again, so a job a model
can never do doesn't wake it three times every few hours. The heartbeat's
Run now queues `Job::Heartbeat` with `manual_at`, the time it was pressed
(Q10 follow-up; `prepare_beat(vault, BeatRequest)`: `Scheduled` for the
timer and the pump's `prepare_scheduled_beat`, `RunNow` for Settings,
`QueuedRunNow` for its rerun in the drain; a merge keeps the later press,
a scheduled beat has none, and a failed rerun moves it to the failure,
`fail_at`, since that rerun was itself let in as a beat). It is wanted only
while no beat was admitted after that time (`heartbeat::beat_admitted_since`, its `last_beat_at`: a second Run
now once the model woke, or the scheduled beat, would make two beats back
to back) and no hard-stop budget holds a routed beat (`budgets::is_blocked`,
read only), checked by `still_wanted` before any wake and the first again
by `drain_check` right before its rerun; one that isn't is dropped with a
note in the heartbeat's log, which the Activity ledger shows
(`note_dropped_run_now`). A person's Run now a budget holds when it would
start waiting is refused at once with the budget's reason (`waiting_beat`);
a queued Run now's rerun never checks it there, because routing asks for
that job first to find the rerun's pin by its key (its gate drops it on a
budget instead). Switching the heartbeat from on to off in Settings drops
its waiting beats, a Run now included, unless a rerun of one is running
(`drop_heartbeats`, from `set_heartbeat_enabled` when
`heartbeat::switched_off`: it was on, it is off, and `switched`, which only
the person's switch passes, not the window's start-up sync). A request that waits says what it
joined (`waiting_note`): the local time a backing-off job is tried again,
or the later of that and a failed drain's own retry time
(`DrainState::retry_at`, which holds every drain of the model), with its
day when it isn't today (`local_when`: "at 14:05", "tomorrow at 00:30",
"on 5 Nov at 09:00"; Q10 follow-up), or that only Try again runs one that
gave up. The drain's retry time lives in the process that ran the drain,
so a request from a window of another Hollow process says "within 30
minutes" instead. The drain loop
(`start`, queue owner only, from `lib.rs`) looks every minute and right after
`answered_signal` (`due_now`, `DrainState::due_drains`); an entry drains
(`due`) when it answered within 8 minutes, or when its oldest job has waited
30 minutes, or on Run them now (`run_now`); only jobs whose kind goes to that
entry (or moved to another), that haven't given up, whose backoff is over and
that no process is rerunning count, never with none, and not again before 30 minutes after a drain that
stopped with an error (`finish`). A failed rerun's backoff (`fail_at`,
`backoff`) is 30 minutes after the first failure and 60 after the second, so
a model that just answered doesn't burn its attempts within minutes; Run them
now doesn't wait for it. A drain holds its entry's drain lock
(`<queue>.drain-<16 hex of the id's SHA-256>.lock` beside the queue, a
non-blocking flock; hashed, so two ids never share one), so a
drain in another Hollow process (Run them now or Try again in a window of a
process that doesn't own the queue) finds it busy and does nothing, and Run
them now says Hollow is running them already. `drain_with` reads the setting
first (unreadable: nothing moves): a job whose kind now goes to another entry
moves there without running; one whose kind is As now is held for the person;
one that gave up waits for the person's Try again. Each job left is asked
whether it is still wanted (`still_wanted`, no model call: a scheduled beat
still due by its gates, a Run now while no beat ran since and no budget
holds it, the retitle still asked for and the title still Hollow's, a
session title still missing, the source not in the extraction ledger with the
same content, the learning gate, no kept pack or recap unless rewriting, no
successful scan since it was queued), and one that isn't is dropped. Only if
any are left, and the entry can run them, one wake with its health path (the
forwarder waits up to 15 minutes); then each reruns back to back while the
entry counts as `awake`, so a rerun never finds it asleep and queues itself
again. Each rerun is a `Rerun` (review round 2): first it claims the job in
the queue file (`claim_at`: read again under the queue's locks, still waiting
for this entry, its kind still going there under the setting read again, not
given up, not backing off unless Run them now, not a self-hosted chat's own
job on another model, and not running; the claim holds the process id and
that process's start time (`proc_info`), and a claim whose process is gone,
whose pid now names a process that started at another time, whose owner
can't be signalled, or that is older than 2 hours doesn't count), so a job
discarded, moved, sent back to As now
or run as now meanwhile is left alone; then it is pinned (`Pinned`, found by
the job's key in `decide_with`, so it holds on the blocking threads Prepare
and Recap decide on) to the drained entry: every entry point decides again
from the setting, and a pinned decision runs the job only there and
otherwise holds it (never As now, never queued again). That is how a drain
never sends a queued job to Anthropic, even when the kind is switched to As
now during a cold start. A rerun leaves the queue only when it succeeds; a
failure counts an attempt, keeps its error and sets its backoff, and after 3
it waits for the person; a rerun that routing held is kept, not an attempt,
with a 30-minute backoff (`hold_at`), so no hold can make a drain loop and
keep a model awake. Success wins unless no decision for the job ran it and
one held it, so another call of the same job that routing held never marks
a rerun that ran (review round 3). The heartbeat's rerun runs the beat inside the drain
(`heartbeat::run_queued_beat`: the scheduled gates and admission, as the
pump would, through `gate_queued_beat`, where a beat that couldn't be
prepared, a held one included, is an error rather than nothing to do; a
Run now's goes through Run now's own gate (`gate_manual`, shared with
`gate_manual_beat`), so neither work hours, the interval, the day's count
nor the heartbeat's switch drops it; a busy vault is an error, and a budget
that holds it is nothing to do, dropped with a note rather than counted as
a failed attempt, Q10 follow-up).
Settings' view (`self_hosted_jobs`) lists each kind's entry and problem, per
entry the jobs waiting (count, oldest time, whether a drain runs here, its
last error), and the held jobs (their kind As now, or given up) with their
attempts, last error, whether a rerun of them runs in any process
(`running`, from the claim) and which buttons apply: Run as now
(`canRunAsNow`, `run_as_now`: only for a held job whose kind is As now under
a setting that reads, never a self-hosted chat's own job (`onlyOn`), claimed
and pinned to As now, so a kind routed meanwhile holds it), Try again
(`canTryAgain`, `try_again`: a job that gave up while its kind still goes to
a model waits again, attempts cleared, on that model, and a drain of it
starts, unless one runs already in this or another Hollow process, or its
drain lock can't be read: then the next drain runs it;
`retry_self_hosted_job` returns `SelfHostedRetry` (`self_hosted_jobs::retried`)
with `started`, `underWay` (false with `started` false when the lock
couldn't be probed: "the next run of <model> runs it") and the `entry` and
`name` the job waits for now, which `try_again_at` may have moved it to,
and Settings' `retryResult` says which, naming that model (Q10 follow-up)) and Discard (`discard_at`, refused while it runs); the words under
each row name only its own buttons. It reads again on
`self-hosted-jobs-changed`. A self-hosted chat's own retitle and learning (Q5
skipped them) now run when Chat titles or Learning extraction goes to that
chat's own entry: `execute_turn` checks `routes_to` at the turn's start, and
the job carries `only_on`, so a later change of the setting skips it rather
than sending it anywhere else. A queued one never leaves its chat's model
(review round 3): `place` holds it for the person (Discard only) while its
kind goes elsewhere, `move_kind_at` moves it only back to its own model, and
`drain_check` never claims it on another. Limits: a routed heartbeat that keeps finding
the model asleep runs about every 30 to 40 minutes, each run a wake; Prepare
and Recap on a sleeping model say the job waits instead of answering; a job
cut off by a quit mid-rerun runs again at the next drain once its claim lapses
(its process is gone, or 2 hours passed; extraction's ledger and the learning
screen keep a repeat from doubling its proposals).


**Codex on a self-hosted model (Q9).** An `openai` entry speaks OpenAI's
Responses API and runs on Codex: it is in Codex's model menu as "<name> ·
self-hosted" and not in Claude's (`agent_models::self_hosted_options` by
`self_hosted::harness`), a Codex turn on it resolves through the same
`turn_route`, and a mismatch (an `anthropic` entry on Codex, an `openai` one
on Claude) is refused before any agent starts with where to choose it
(`Found::Mismatch`; the builder refuses it too, `wrong_harness`). Why it
needs its own home (`q9-findings.md`, 2026-11-03): Codex's first probe on Qwen
failed with "prompt contains at least 131073 input tokens" because it ran on
the person's `~/.codex`, signed in to ChatGPT with apps on, so Codex sent 728
ChatGPT connector tools (2.6 MB of schemas); a Codex home with no login, or
`--disable apps`, sent 9 built-in tools and worked live.

*The home.* `self_hosted::codex_home_for`: `self-hosted-codex/<id>` in app
data, never `~/.codex`, stable so its rollouts persist and resume and the
rollout watch (`codex_rollout::Watch::for_spec`) find them; Codex's MCP
inventory lists against it (`codex_inventory::installed`, which lists
nothing for a target Codex can't run, so `codex mcp list` never reads the
person's `~/.codex` first). `ensure_codex_home` makes it 0700 and refuses
the run, naming the path and deleting nothing, when it holds a file Hollow
never writes and Codex doesn't write in a run (`CODEX_HOME_REFUSED`:
`auth.json`, `config.toml`, `managed_config.toml`, `requirements.toml`; a
turn with full access could have written one to send later turns
elsewhere), or when Codex would read settings that can outrank the command
line (`codex_outranking_files`: `/etc/codex/config.toml`,
`/etc/codex/managed_config.toml`, the managed preferences
`com.openai.codex.plist`; none exists on this Mac), saying to set the
model's API style to Anthropic's Messages API to run it on Claude or to
remove the file (an organisation's managed settings only by its admin).
Tests check no such system file (`checked_outranking_files` is empty under
test, so a host that has one doesn't fail them); `codex_home_problem` is
tested with its list given. Removing an entry in
Settings leaves its home, with that model's Codex conversations, in app
data; Remove's confirmation says so and names the folder, and the person
deletes it if they want (Hollow never does). An entry added again under the
same id finds the old home.

*The provider and the run.* Set on argv (`self_hosted::codex_overrides`,
top-level `-c` before the subcommand, so `exec`, `exec resume` and
`app-server` get it): `model_provider="hollow-self-hosted"` and its table
with `base_url` the forwarder's `/v1`, `wire_api="responses"`,
`env_key="HOLLOW_SELF_HOSTED_KEY"`, `requires_openai_auth=false` and
`stream_idle_timeout_ms=900000` (a cold start, as Claude's
`API_TIMEOUT_MS`); `model` (the served id, also `-m` and the app-server
thread's model) and `model_context_window` (the entry's window); Codex's own
telemetry off (`otel.exporter`, `otel.trace_exporter` and
`otel.metrics_exporter` `"none"`, `analytics.enabled=false`,
`feedback.enabled=false`); and its shell environment policy keeping its
default excludes and excluding `HOLLOW_SELF_HOSTED_KEY` by name. No profile
is pinned: Codex 0.166.0 refuses a legacy `profile` key with an error and
selects one only with `--profile`, which Hollow never passes. Effort is Q3's
mapping through the entry's levels with `max` never sent (Codex has none;
the registry refuses `max` on an `openai` entry and `RunTarget::effort` drops
it): for Qwen low to low, medium and high to medium, xhigh, max and
ultracode to xhigh. `model_auto_compact_token_limit` is the entry's compact
window (`agent::compact_window`, over Settings'), 100,000 for Qwen.
`self_hosted::apply_codex_env` (the last env step) removes every inherited
or set variable starting `OPENAI_`, `CODEX_`, `AZURE_OPENAI_`, `ANTHROPIC_`
or `CLAUDE_CODE_USE_` and the Claude credentials, then sets `CODEX_HOME`,
the run key in `HOLLOW_SELF_HOSTED_KEY` (Codex sends it as `Authorization:
Bearer`), the marker, and the loopback hosts in `NO_PROXY`; no Anthropic or
OpenAI account step runs. The key is never on argv or on disk. The commands
Codex runs don't get it (the shell policy above; checked: an `env` call's
output held neither it nor `OPENAI_API_KEY`), but like Claude's run key it
is readable by the person's own processes while the run lives (`ps -E`),
and the forwarder revokes it when the turn ends. Hollow's own MCP servers
reach Codex as on any Codex run, with the marker in their config env.

*Probes, 2026-11-03, Codex 0.166.0 on this Mac* (where self-hosted runs
happen; `$0`, two loopback stub endpoints, a logging proxy as
`HTTPS_PROXY`, `lsof` sampled on Codex's process tree): with the overrides
as first built, Codex opened `CONNECT ab.chatgpt.com:443` (its metrics);
with telemetry off, nothing but loopback, no proxy request. A home
`config.toml` setting `model_provider` and its own
`[model_providers.hollow-self-hosted]` at the second stub, and a
`<name>.config.toml` profile file, lost to `-c`: the request went to
Hollow's stub (`-c` outranks the user and system config). A legacy
`profile = "…"` in the home's `config.toml` made Codex exit with an error
(fail closed). A project's `.codex/config.toml` with the same keys was
ignored ("Ignored unsupported project-local config keys"). `exec` and
`app-server` both finished a turn with an empty home and no sign-in, sent
`/v1/responses` with the run key as the bearer and 9 built-in tools, wrote
the rollout under the home, and wrote no `config.toml` there. Earlier the
same day with Codex 0.164.0 on the Mac mini: an MCP server's tools go as one
`namespace` tool (`mcp__hollow_tasks` holding `list_tasks`), and Codex
expects the call back as a `function_call` with that `namespace`; an answer
stream needs `response.output_item.added` before its text deltas
("OutputTextDelta without active item" otherwise). Which version checked
what: the provider overrides, the config precedence, the telemetry and the
agent's shell env on Codex 0.166.0 here, where self-hosted runs happen; the
grouped MCP tool call through the real forwarder (the opt-in test below) on
Codex 0.164.0 on the Mac mini, rerun there with the final overrides,
telemetry and shell policy included. Managed settings
(`/etc/codex/managed_config.toml`, the managed preferences) outrank `-c` by
Codex's design and weren't probed (they need root); Hollow refuses to run
while one exists.

*Guards and the rest.* The served-id guard is per harness, on purpose
(`needs_target_on`): a Claude run refuses every served id Hollow knows, of
either style (it would reach Anthropic with the person's login, and no
served id may be one of Claude's own); a Codex run refuses an `openai`
entry's served id, and an `openai` entry may not serve an id shaped like
OpenAI's own (`gpt-`, `codex-`, `chatgpt-`, or `o` and a digit,
`self_hosted_registry::openai_model_id`) nor one Codex's own catalog
(`models_cache.json`) lists, checked on save, so no ordinary Codex run is
refused; an `anthropic` entry may serve `gpt-oss-120b`, which Codex still
runs. Codex's guard is the set of served ids of the valid `openai` entries
in the app's registry, switched on or not (`self_hosted::registry_changed`,
replaced whole at startup through `agent_models::warm_effort_catalogs` and
on every save, switch and removal of the app's registry, under its lock),
so an edited, restyled or removed entry's old id stops being refused at
once, and a menu read feeds it nothing. Windows and Claude's guard are only
added to (`remember_entries`; an old id can only be a self-hosted model's,
never one of Claude's own). Only Claude registers the levels with
`effort::remember` (a Codex run's effort comes from its target). A delegate inside such a run loses
`HOLLOW_SELF_HOSTED_KEY` and the self-hosted `CODEX_HOME`
(`strip_inherited`), and `hollow team-mcp` refuses a Claude teammate with
`NO_CLAUDE_TEAMMATE`, which asks for a model that isn't self-hosted.
Background jobs (Q10) stay on Claude: they are Claude one-shots, so
Settings' Background jobs offers only switched-on `anthropic` entries
(`jobOptions`, with `JOBS_CLAUDE_ONLY` saying why), `set_at` refuses an
`openai` one, and a kind saved to an entry that later became `openai` is
held, skipped with `self_hosted_jobs::reason`'s note ("runs on Codex in
chats only; background jobs run on Claude"), never run As now and never
queued; a Codex chat's own retitle and learning stay skipped the same way.
Runs on
(Q7): a Codex chat on an `openai` entry is a self-hosted chat for placing
(`chat_on_self_hosted`), so it runs on this Mac with Q7's first-line note;
its placement skips the Codex sign-in check (`runs_on_checked`), which is
for turns that go to the runner, so the note names the runner, and the
builder's own refusal stays as a backstop. Usage (Q8): the run key in
`HOLLOW_SELF_HOSTED_KEY` classifies a Codex command as `Billing::SelfHosted`
(`usage::classify`), so the turn's tokens go under the served id with
"billed by GPU time" on the endpoint and no price. The forwarder needed no
change: any path goes up as sent (only `content-type`, `accept` and
Anthropic's two headers are forwarded, so Codex's `session-id`, `originator`
and `x-codex-*` stay here), the key is checked in `Authorization: Bearer`,
and its 401 body already carries `error.type` and `error.message`, where
OpenAI's clients read them. Settings' Test of an `openai` entry sends `POST
/v1/responses` with `max_output_tokens` 1
(`self_hosted_registry::one_token_request`). Limits: Codex reports "Model
metadata for `<id>` not found" each turn (a non-fatal `error` row under
`exec`, a `warning` under `app-server`); its `web_search` tool is in the
request though vLLM can't run it; whether Qwen through vLLM calls a
namespaced MCP tool is not yet checked live (Q9's live turns used Codex's
shell tool).

*Tests.* `self_hosted_tests.rs` (the command with its overrides, env, effort
and billing, a normal Codex command byte for byte, refusals including the
home's files and an outranking file, the listing skipped for a target Codex
can't run, startup remembering, delegates), `self_hosted_registry_tests.rs`
(the style, `max`, OpenAI's ids and the catalog, Test),
`self_hosted_forwarder_tests.rs` (`/v1/responses` streamed through, a
missing key refused), `agent_models.rs`, `codex_rollout.rs`,
`remote_session_tests.rs` (a Codex turn kept here) and
`chat_self_hosted_tests.rs` (a whole Codex turn through
`drive_turn_attempts`, the real builder, registry and forwarder against a
stub Responses endpoint, with a shell stand-in for Codex that calls
`mcp__hollow_tasks__list_tasks` and answers, billed by GPU time; and the
`#[ignore]`d `real_codex_calls_hollow_tasks_through_the_forwarder`, which
runs the installed `codex` and the built `hollow` binary the same way,
`cargo build --bin hollow` then `-- --ignored`, at no cost; it passed with
Codex 0.164.0 on the Mac mini, with the final overrides too, and skips with
a line when either binary is missing).

**Bake-offs (Q11).** A self-hosted model can be a model bake-off variant, so
the routing table can learn which work kinds it handles: a pair names
`self-hosted:<entry>` on the agent the entry's API style rides
(`self_hosted::harness`: Claude for `anthropic`, Codex for Q9's `openai`),
shown everywhere as the model menu names it ("Qwen3.8 27B ·
self-hosted"). A switched-off, removed or mismatched entry is refused when
the bake-off starts. If that happens later, or the entry is pointed at
another address, sign-in or served model than the start saw, the variant is
skipped when its turn comes; if it is switched off, removed, made invalid
or mismatched after its turn was queued, the chat route refuses the turn
before it starts and the variant is skipped with that reason, while one
pointed at another address, sign-in or served model in that wait runs as edited
(runs.md's limit; Q11 follow-up: this said any change stopped it). The
start's view of the entry is kept as a SHA-256 of its normalized address,
sign-in and served model (`SelfHostedPin`, Q11 follow-up), never in clear
(a file Q11 wrote with a clear pin is written again hashed when it loads);
a bake-off started before Q11 has none and its variant is skipped with its
own reason. It never runs on another model under the start's name (a rename
only renames it). The variant is an ordinary chat turn through `send_chat`,
so Q1's isolation, Q3's effort and window, Q7's runs-here rule and Q8's usage
apply, and nothing wakes the model before that turn's own first call. It is
billed by GPU time: it reserves nothing against the bake-off's cap or the $50
monthly cap and takes no share of the cap from unpriced pairs, reads "billed
by GPU time" once it ran (hidden while blind like every cost, its time and
the report's `unknownCostRuns` too),
and a routing suggestion never calls it cheaper. Bake-offs of self-hosted
pairs alone are limited to 10 a UTC month. A self-hosted chat turn (a variant
or any other) gets at least `self_hosted::COLD_START_FLOOR`, 16 minutes,
before the no-output watchdog or a run deadline ends it
(`self_hosted::cold_start_limits` in `execute_turn`), so a shorter Settings
value doesn't cut a cold start; off stays off. Details:
[runs](runs.md#model-bake-off-master-plan-14-2026-10-09). Tested in
bakeoff.rs (removed 11-10, S6) (start checks, cap math, the monthly count, skip at launch and
on a changed entry, a turn that never started, masking, routing, the offered
pairs), `self_hosted_tests.rs`
(`a_self_hosted_turns_limits_wait_out_a_cold_start`), `chat_tests.rs` (the
pin) and bakeoff.test.ts (removed 11-10) (the Routing panel's words).

**Review follow-ups (Q7, Q10 and Q11, 2026-11-03).** Minor findings from the
independent reviews of #1128, #1129 and #1131, each described in place
above: the morning report reads `remote_run_trail` (start and back notices
only), the `execute_turn` pin requires the `still_stale` re-read, the
`pending` note says the runner didn't answer or wasn't asked, an automatic
request never weakens a waiting rewrite (`Job::merged`), a queued heartbeat
Run now keeps `manual_at`, skips the schedule's gates and is dropped (never
counted as a failed attempt) once a beat ran after it or a budget holds it,
switching the heartbeat off drops its waiting beats, `waiting_note` names
the day and the later retry time, Try again names the model the job waits
for now and what runs it, and a bake-off keeps its self-hosted pin as a
SHA-256 (a file with a clear pin is written again hashed when it loads),
hides `unknownCostRuns` while blind and words a missing pin on its own ([runs](runs.md#model-bake-off-master-plan-14-2026-10-09)). A browser run
also found that Settings search listed "Use the Qwen on Cloud Run preset"
while the button showed only inside the open form: one `PresetButton` now
sits beside Add (it opens the form already filled) and inside the form. Tested
in `night_queue.rs` (`the_morning_report_names_work_left_on_the_mac_mini`),
`chat_self_hosted_tests.rs`, `remoteRunNotice.test.ts`, `heartbeat.rs`
(`a_queued_manual_beat_skips_the_schedule_gates`), `self_hosted_jobs_tests.rs`
(`an_automatic_request_never_weakens_a_waiting_rewrite`,
`a_waiting_note_names_the_day_and_the_later_retry`,
`try_again_names_the_model_it_waits_for_now_and_what_runs_it`,
`a_queued_run_now_a_budget_holds_is_dropped_before_any_wake`,
`a_queued_run_now_never_runs_a_second_beat_after_one_ran`,
`switching_the_heartbeat_off_drops_its_waiting_beats`), bakeoff.rs (removed 11-10, S6)
(`a_self_hosted_pin_is_a_hash_and_an_old_clear_pin_still_compares`,
`a_self_hosted_variant_with_no_pin_is_skipped_with_its_own_reason`,
`loading_a_file_with_a_clear_pin_writes_it_again_hashed`),
`selfHostedModels.test.ts` and the `settings-shell` smoke spec.
