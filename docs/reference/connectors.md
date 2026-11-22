# Connectors library and context prioritizer

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Connectors library and
context prioritizer" and "Onboarding and directory setup", moved there word
for word on 2026-10-13 (only relative links were re-based). Read it before
changing connectors, the context prioritizer or onboarding. Keep it current
with the code in the same change, as AGENTS.md asks.

## Connectors library and context prioritizer

**Services (chat-first port).** Settings → Connectors starts with Calendar
and GitHub setup. The Calendar selector retains explicit Mac access and
Microsoft sign-in. The Notion token, Test connection and sync editor were
removed with Notion sync (2026-11-10); old “Hollow’s own sources” jumps land on
Task tracker. Existing
imported/custom connector settings remain available under Your connectors.
GitHub opens an existing HTTP/local connector without enabling it or changing
access, otherwise fills an unsaved form at the read-only GitHub endpoint.
Saving that new preset uses relevant-only sharing, both supported agents,
automations off and autoApprove off. Replacing a form asks first and checks
the exact draft identity again after the answer; failed library reads disable
setup. Opening setup never saves or signs in. Account-access copy does not
promise imported credentials or future messaging features.

**Connectors** (`connectors.rs`, `settings/ConnectorsLibrary.tsx`): MCP
servers added once in Hollow (`connectors.json` in the app data dir, 0600)
and shared with Claude and Codex, per connector for chats and/or automations,
`always` or only when `relevant`, optionally `autoApprove`. Hollow runs Claude
with `--strict-mcp-config` and switches off every user Codex server, so a
CLI's own MCP servers reach a Hollow run only through this library. Env and
header values are secrets: views return only their names. Import reads
`~/.claude.json` and `~/.codex/config.toml` and never edits them. **Cua Driver (CU9, 2026-11-18):** a stdio connector whose command or any argument names `cua-driver` or `CuaDriver` (`connectors::runs_cua_driver`, any case or version suffix, `npx`/`uvx`/shell wrappers included) is never preapproved: its discovered row carries `CUA_DRIVER_WARNING` (its tools bypass Hollow's cards, modes and audit; Computer control is the supported way), import saves it with `autoApprove` off and that warning as its description, saving one stores `autoApprove` off, and both the view and the attachment report it off whatever the saved row says. Bypass preapproves every library connector regardless of its switch (`mode_contract::connector_preapproved`), so `eligible_with` leaves a Cua Driver connector out of a bypass run altogether. The match is fail-safe: any shell word of the command or arguments (split on whitespace and shell punctuation, cut at a version suffix) whose last path part is the name counts, so an unrelated server given a path ending in `cua-driver` also loses preapproval. Codex's own Computer Use app (`./Codex Computer Use.app/...`, relative, so refused at import and skipped at attach) shows "Replaced by Computer control (Settings, Connectors, Built into Hollow)" in the discovered list. Attaching is `connectors::attach(agent, perm, automation, query)`, which returns `Attached { connectors, selection }`: the attachments and the prioritizer's per-candidate decisions, which chat and workflow runs put on their context manifest (below).
`query` (`context_rank::ConnectorQuery`) is the run's request (chat:
`turn_inputs`' connector query; workflow: `compose_run_context`'s), never the
prompt expanded with profile, recall, evidence or workspace context, so a long
profile can't push a named connector past Jev's 2,000-character query. Jev is
sent `query.request` only; `query.local` adds resolved antecedent names for
the local rules.
The run's `<hollow-connectors>` note (`connectors::prompt_note(attached,
perm, automation)`) names them, says which run without a prompt in this mode
(`mode_contract::connector_sentence`, from `connector_preapproved`) and carries
the external confirmation (`mode_contract::external_confirm`).
**claude.ai connectors** (transport `claudeAi`) work only on the claude.ai
account they were signed in on, so a Claude run carrying one goes on Claude
Code's own login whatever the routing (`agent.rs` → `claude_accounts::apply_login`)
and never fails over or moves at the ceiling. **Login override (2026-10-09):**
such a run gets a receipt row (`ReceiptSwitch` kind `login`, "claude.ai
connector: …"; chats on the first attempt, automations in `workflows.rs`) and
the `accountLogin` trace event; Settings → Accounts lists the connectors that
force it (`loginOverrideLine`); a claude.ai connector set to Always shows a
warning with **Attach only when relevant** in Connectors (`alwaysLoginWarning`).
"When relevant" is the one filter for chats, `start_session` children (chat
turns) and automations (`connectors::attachments`); night-queue turns get none.
A login run whose own `rate_limit_event` puts the login at or over the Spread
ceiling (100% in Single) while an added account has room warns once per window
(`claude_accounts::{login_low, first_login_warning}`, one key per window past
the line: window + `resetsAt`, or a slice of the window's length when the reset
is unknown): a `loginLow` chat row and a bell notification
(`notify_login_low`). An automation on the login warns the same way at its
start, from the login's last-seen windows (`login_low_last_seen`, from
`claude_quota::login_view`). While the login has room, Hollow never drops an
"Always" connector to move a run: whether a run needs it can't be known from
the request alone (claude.ai connectors often have no description), so "Only
when relevant" is the way off the login. **Spread usage across accounts (2026-10-21):**
once the login is out of room (`claude_accounts::login_out_of_room`: the
failover setting on, the login past its line from its last-seen windows, an
added account with room), a Claude run treats its claude.ai connectors set to
Always as "Only when relevant" (`connectors::demoted_ids`): they join the
relevance candidates, so a message that names or needs one still gets it (and
stays on the login), and the rest leave it out (`Attached::set_aside`) and can
spread. If another claude.ai connector is attached anyway, the demoted ones
ride along (`attachments_with`), since the run is on the login regardless.
Chats only (`login_out_for`): an automation's prompt is fixed and no one can
name a connector back, so it keeps its Always connectors and warns at its start
as before. A follow-up that keeps its last decision (`attach_kept`) demotes only
connectors that decision covered (`kept_demotions`), so "ok, send it" keeps the
one it used. Chats tell the agent (`<hollow-connectors-set-aside>`,
`set_aside_note`) every turn and the person once per conversation and window (a
`loginLow` row, `first_set_aside_notice`). A claude.ai connector a signed-in
Hollow connector takes over never pins a chat (`takenOver`, mirroring
`eligible_with`), so Settings doesn't count it. Settings → Accounts says all this: the header names the
login (`~/.claude.json` → `oauthAccount`) and the Always connector
(`routingSummary(view, pin)`, `loginPinFrom`), a **Claude Code's login** row
shows its last-seen windows (`AccountsView.loginUsage`, never a Keychain read)
with **Open Connectors**, and the added account's badge reads "Next run
without a claude.ai connector" while chats are pinned. Connectors' Attach
choice warns that Always turns spreading off (`alwaysSpreadHint`).
"Added accounts exist" means one could run now (`has_added`, the
routing's candidates). A run that carries the connector never moves. A Finder-launched Hollow has a bare `PATH`, so
stdio connectors and agent CLIs are launched with a reconstructed `PATH`.

#### Built-in connectors: Microsoft 365 and Plaud

`builtin_connectors.rs` (`ALL` = Microsoft 365, Plaud): **Microsoft 365** (`hollow_m365`,
`m365_mcp.rs`, read-only calendar and OneDrive/SharePoint files on Hollow's
own Microsoft sign-in, `m365_auth.rs`) and Plaud below. Hollow's own Notion
connector (`hollow_notion`, the `hollow notion-mcp` server) was removed on 2026-11-10; a stored
library row for it is dropped from the library view and never attached. They are `Transport::Builtin` rows,
present by default, never removable; the subprocess reads its credential
itself, and only the GUI writes Microsoft tokens. The GUI refreshes the token
(`m365_auth::warm`) in `connectors::attach` only when Microsoft 365 is among
the run's attached connectors, after selection, waiting until the preparation
deadline at most (the refresh runs in its own task and still saves; F8 O2,
2026-10-07: before, any eligible library refreshed it for every turn). List reads follow pagination
and say when a result is partial; `xlsx_cells.rs` reads Excel files as typed
cells. A ready built-in leaves out the claude.ai connector at its
`supersedes_host` (`microsoft365.mcp.claude.com`, `mcp.plaud.ai`).
**Plaud (`hollow_plaud`, overnight 2026-10-13 S8)** is the one built-in with a `program` instead of a
`hollow` subcommand: its entry runs Plaud's installed `plaud-mcp` (`@plaud-ai/mcp`) resolved on the tool
PATH, exactly like a local connector. `plaud-mcp` keeps its **own** sign-in in `~/.plaud/tokens-mcp.json`,
not the `plaud` CLI's `~/.plaud/tokens.json` that Recordings use. Ready = that file holds an `access_token`
and `plaud-mcp` is on the tool PATH (`plaud_mcp_auth::ready`; no network). Sign in / Sign out
(`plaud_mcp_sign_in` External, `plaud_mcp_sign_out` Confirm) start the server in its own process group,
speak MCP over stdio (`initialize`, `notifications/initialized`, one `tools/call` of `login` / `logout`)
and end the group; sign-in waits up to 5 minutes (the tool's own limit is 2; port 8199) and says a timeout
in words. **Never run `plaud-mcp install` or `setup`** (they rewrite other apps' agent configs): Hollow
starts it with no arguments. **Agents never get its `login`/`logout` tools** (`Builtin::hidden_tools`):
Claude runs get `--disallowed-tools mcp__hollow_plaud__login mcp__hollow_plaud__logout`
(`agent::hidden_mcp_tools`), Codex runs `disabled_tools=["login","logout"]` on the server's entry; with those hidden every remaining Plaud tool
only reads, so the row defaults to `auto_approve: true` like the other built-ins (`false` would refuse every
call in headless runs, which can't ask). One Plaud browser sign-in at a time across Hollow (`plaud_mcp_auth::SignInSlot`): this sign-in,
its sign-out and Recordings' `plaud login` share it (both logins use port 8199). Settings offers **Sign in**
only while signed out and **Sign out** only while signed in; "Already logged in." comes back as
`already`. **A Plaud the person added wins:** any stdio/http library connector whose command or an argument
is `plaud-mcp`, `@plaud-ai/mcp[@version]` or a file inside that package (`node …/@plaud-ai/mcp/dist/index.js`),
a remote at `plaud.ai` / `*.plaud.ai`, or one named Plaud, on or off (`builtin_connectors::is_own`,
`connectors::own_instead`), makes the built-in stand aside for every agent: never eligible, and its row is
`builtin_connectors::aside` (`own` = their connector's name, "Off" in Settings and the composer). Not per
agent: a personal Plaud shared only with Claude also keeps the built-in from Codex (accepted; remove yours to
use the built-in). A claude.ai Plaud is not "theirs": the ready built-in replaces it. Tests use a fake
`plaud-mcp` shell script on a PATH without `/opt/homebrew/bin`, so they can never start the real one.
**Removed 2026-11-10 with Notion sync:** a canvas to a Notion page (`toNotion`,
`AddToNotionDialog`, `notion_append_canvas`, `notion_search_pages`; replaced by
**Attach to issue…**, [Tracker sync](../TRACKER_SYNC.md#status-and-canvas-copies)) and
`notion.move_page`. Notion's official connector (`notion-official`, a library row, once
signed in) is an ordinary connector: Hollow hides or denies none of its tools.

#### Remote connectors on Hollow's own sign-in (2026-10-10)

A `Transport::Http` connector with `oauth: true` (optional `oauthClientId`,
and with it an optional `oauthClientSecret` for a confidential app such as a
Zoom Marketplace one: stored in the library like env secrets, never in a
view (`oauthClientSecretSet`), dropped when the client id or the URL changes
(the exact address: the server's metadata picks the token endpoint, so the
same host's other path is another server), sent at the token endpoint by
`secret_method` (Basic unless the server takes only `client_secret_post`; an
id with a secret may not contain `:`); a changed id, secret or URL forgets the
old sign-in before the library is rewritten (under the token lock); a grant
that arrives after such an edit is not saved (`SignIn::still_current`,
checked under the token lock); `connectors.json` is 0600 after every save
(`setup::publish_private_config`); the form's `savedSecretFate` says when a
save will drop the secret or sign out; overnight 10-11 Y6, finished in 10-12
B2) is signed in by Hollow itself (`remote_auth.rs`, MCP authorization spec:
RFC 9728 protected-resource metadata from the 401 challenge or well-known
paths, RFC 8414/OpenID metadata, RFC 7591 registration unless a client id is
given, PKCE S256 with a loopback redirect, RFC 8707 `resource` on every
request; endpoints https only, loopback http in tests). Runs get it as a stdio
server, `hollow remote-mcp --id --url --auth-dir` (`remote_mcp.rs`): JSON-RPC
lines POSTed with the bearer, `Mcp-Session-Id` and `MCP-Protocol-Version`,
JSON or SSE answers, one refresh-and-retry on 401, a 404 on a session
restarts it with the agent's own `initialize` and retries once, no
redirects, DELETE on exit. Requests run side by side; `initialize`,
notifications and the agent's answers go in order. No custom headers (the
bridge sends only the token). The metadata's `resource` (checked to cover
the URL) is the audience sent; the authorization server's `issuer` must
match. Loopback http endpoints only in tests and debug builds. Tokens are one 0600 file per connector, `<app data>/RemoteAuth/<id>.json`
(not the Keychain item: rotated refresh tokens must be saved by the subprocess
too), refreshed under a `flock` on `<id>.lock` with a re-read, bound to the
URL they were granted for; a refused refresh records `problem` and the
connector stops being eligible. `connectors::signed_in` gates eligibility and
`supersedes` makes a signed-in one replace the claude.ai row at the same host.
Settings → Connectors → a claude.ai row → **Use Hollow's sign-in**
(`connector_use_hollow_sign_in`: same URL and switches, Claude + Codex, then
`remote_connector_sign_in`); `remote_connector_sign_out`; removing the
connector deletes its tokens. Refused for mail/Slack/Teams hosts
(`messaging_host`, except the approved Superhuman Mail host) and Anthropic-only addresses (`anthropic_hosted`).
Settings → Connectors → Add connectors has a one-click **Superhuman Mail** tile (10-11 B4).

**Reliability (2026-10-10, B3).** A claude.ai row's `keepClaudeai` ("Use
Hollow's instead" off) keeps it and leaves Hollow's connector for that host
out instead (`eligible_with`; Codex still gets Hollow's). The login override's
receipt reason names the remedy: `connectors::remedy_for` at attach time,
with the whole library (turn "Use Hollow's instead" back on, turn on / sign in
to / share Hollow's equivalent, or "Use Hollow's sign-in on <name>"; nothing
for mail/Slack/Teams), carried on `Attachment.remedy` → `SharedConnector.remedy`
→ `connectors::login_remedy` (deduped). Microsoft 365: a refresh refused with
OAuth `invalid_grant`/`interaction_required`/`consent_required` on 400/401
(`m365_auth::grant_over`; not Conditional Access by place/device, AADSTS50196,
proxies, 429 or 5xx) is saved as `Tokens.problem` until the next sign-in, only
if no newer tokens were saved meanwhile (`save_marked`). Then
`m365_auth::signed_in()` (the built-in's readiness) is false, Settings and the
Agenda say why, and a status check ("Check again", `status_in`) retries once.
Today and the Agenda show such a calendar error (one starting with
`NEEDS_SIGN_IN` or `EXPIRED`; there is no error code, so
`dayRing.calendarNeedsSignIn` matches those words and a vitest guard holds it
to `m365_auth.rs`) as "Microsoft 365 needs you to sign in again" with **Open
Connectors** instead of "Calendar unavailable" and Retry (overnight H6).
Settings shows each row's "Works on" line (`connectorCapability.worksOn`,
aware of kept and replaced rows).

**Context prioritizer** (`context_rank.rs`, `settings/ContextPrioritizer.tsx`):
local rules attach a "relevant" connector when the request names it or shares
a meaningful word with its name or description, and order retrieval passages.
`select_connectors` returns exactly one `ConnectorDecision` per candidate, in
order (`id`, `attach`, `basis`, Jev's `score`, a `reason` when Jev's answer
wasn't used). A connector the request names (id or name) is attached first
and never sent to Jev (`named`). With Jev on, one request carries at most
`jev::MAX_CANDIDATES` (24) of the rest, chosen locally (locally relevant
first, then library order). Those past it keep local relevance
(`beyond-limit`); a failed call (`local-fallback`) or a missing or invalid
score (`unscored`) also falls back to local relevance, per candidate, with
the reason. `jev::score_with` errors on more than 24 items rather than
truncating. Contract in [docs/JEV.md](../JEV.md#connector-cardinality-contract).

**Preparation deadline** (`prep_deadline.rs`, F8 O2, 2026-10-07): a chat turn
(`execute_turn`) and a workflow run (`prepare_spawn`) prepare retrieval
(`directory_connections::prioritized_context`) and connector selection
(`connectors::attach`) at once (`prep_deadline::concurrently` in a workflow;
a chat turn settles connector selection first, retrieval polled beside it, so
a steerable Claude turn's agent can start while retrieval finishes, run 15
S2), under one
`PrepDeadline` (`BUDGET` 3 s from the start of that stage) for the optional
parts: both Jev calls, the broker's semantic half (`retrieval_broker::
retrieve_by` starts it on its own thread beside the lexical scan) and the
Microsoft 365 refresh. Past it: Jev → local decisions (`jev::outcome_of` →
`deadline`, reason `prep_deadline::JEV_MISSED`), semantic → lexical only
(`semantic: "deadline"`), the refresh finishes on its own. A call is never
sent once the deadline has passed. Required local context (the lexical scan,
recall, attached sources, named and "always" connectors) is never cut.
Preparation's two Jev calls go through `jev::rank` and a `jev::Guard` (one
per process; `Target::app()` reads the key once for both): an identical
request (SHA-256 of the endpoint and request body) reuses its scores for
120 s, at most 32 kept, scores only (never text, a key, a grant or a
decision), failures never kept (`cached`); three failed calls in a row
(timeout, deadline, error, malformed) pause Jev for 60 s (`circuit-open`,
nothing sent), and the next call after the cool-down closes it on an answer.
`save_jev_settings`/`remove_jev_key` reset the guard. The study runner
(`score_with`/`score_each_with`) and the duplicate check bypass it. Each request still builds its own client: reuse was measured at
33 µs a call and left out (wave 3 rule).

**Codex native inventory cache** (`codex_inventory.rs`, F8 O3, 2026-10-07):
before a Codex command is built, `codex mcp list --json` names every server
the user configured so the run can switch each off. The names are reused
while nothing they depend on changed: the key is the binary (canonical path,
size, mtime, and the `package.json` beside a node wrapper), `CODEX_HOME`, the
working folder and a SHA-256 over every file Codex reads servers or policy
from (`config.toml`, `managed_config.toml`, `requirements.toml` in
`CODEX_HOME` and `/etc/codex`, `com.openai.codex.plist` managed preferences,
and `.codex/config.toml` in the working folder and every folder above it).
Any change lists again; an entry also expires after 10 minutes, at most 16 are
kept, a failed listing is never kept (the run still fails closed), and only
names are cached: overrides and approvals are built per run from its own
mode. In memory per process (the `hollow team-mcp` process has its own). The
manifest's `preparation.codexInventory` says `listed` or `cached`.
`agent::build_command_via` takes the inventory as a function (tests and the
bench pass a stand-in CLI).

**Per-run context manifest** (`run_manifest.rs`, agent effectiveness F11):
what prepared one run's context and why, built on its `EvidenceCollector`
(`context_mut()`) by chat turns, workflow runs and (since O3) heartbeat
beats (not ingest, teammates or one-shots). A beat's manifest has its
prompt `layers` (`toolGuidance`, `recall`, `operatorProfile`,
`alreadyProposed`, `request`, `snapshot`) and its duplicate check. There is no process-global "last report" any more (the old
`context_rank` `LAST` and IPC `context_prioritizer_report` are gone): two
turns preparing at once overwrote each other's. Schema 1 (`RunManifest`,
camelCase): `runId`; `layers` (`{name, bytes, sha256}` per non-empty layer in
reading order: `visualPrompt`, `capabilityIndex`, `environmentBrief`, `policy`, `workspace`,
`operatorProfile`, `recall`, `connectorsNote`, `retrievalEvidence`, and from
the composed message `conversationContext`, `taskState`, `operatorUpdate`,
`visualGuide`, `pageContext`, `chatMode`, `chatIntent`, `request`, `attachedImages`, `attachedFiles`); `retrieval` (`scope`,
`scopeBasis`, `workspace`, `engine`, the broker's whole ranked pool (≤ 24) as
`candidates` with vault-relative `path` (a connected project's file keeps its
absolute path), `line`, `rank`, `localScore` (query terms matched),
`semanticScore`, `jevScore`, `kept` (false past the 8-passage budget, with
the reason), `basis`, `reason`, the excerpt's `bytes` and `sha256`, capped
coverage `notices`, and `stats`: the prefilter (`fts`, `fts-read-only`,
`linear`), `candidateSource` (`bm25` or `linear`) and `candidates` (OB12),
vault notes eligible/read/matched, bytes read and scored, why the
scan stopped, project files read, `pool`, `budget`, `semantic`
(`blended`/`off`/`unavailable`/`deadline`) with its reason, hits and time, and the
broker's `ms`; F3, 2026-10-06); `attachedSources` (a workspace run's
attached sources, F3 part 2: `budget` 24,000 and `used`, `reader`
(`read_source` or `own-tools`), the request's term count, `bytesSearched`,
chunk counts, `trimmed`, per file `{path, coverage, lines, totalBytes,
includedBytes, sha256}`, and per kept or matching chunk `{path, fromLine,
toLine, score, kept, bytes, reason}`, at most 96, the rest counted);
`lifecycle` (F7: `history`, the `cue` that made the request historical, one
of Hollow's own words or `a past date`, never the request; per decision
`{route: recall|retrieval, path, status: superseded|current|duplicate|
unapproved, action: excluded|historical|promoted|current|collapsed|labelled,
current, reason}`, at most 32 per run, the rest counted; absent when nothing
had a lifecycle); `connectors` (the F5 `ConnectorSelection`
with its `note`); `jev` (per use, `passages` or `connectors`: `outcome`
`ok`/`timeout`/`malformed`/`error`/`off`, and since O2 `cached` (a reused
ranking: `items` covered, no latency), `deadline` (cut, or not sent because
it had passed: 0 items) and `circuit-open` (paused, 0 items), `items`,
`latencyMs`, and `costUsd`, always `null` because TypeSafe reports none;
since O3 also `proposals`, a beat's duplicate check); `dedup` (O3: the
duplicate checks the run itself ran, each a `JevDecisionsReport`); `capabilities`
(`ownedServers`, attached library `connectors`); `fallbacks` (`{stage,
detail}`: Jev failed or scored only some, a fresh native session);
`preparation` (`retrievalMs`, `connectorsMs`, and since wave 3 · O1 (F8,
`prep_timing.rs`) `timeline` `{origin: "admission", totalMs, phases}`: one
`{name, startMs, durationMs}` per step, offsets in ms from admission's start
(its completion less its vault snapshot) on the monotonic clock, both `null`
for a step that didn't run, the ones that ran by start, then the rest in
step order. Steps: `admission` (the snapshot), `recall`, `attachedSources`,
`retrieval` with `retrievalSemantic` and `retrievalJev` inside it,
`connectors` with `microsoftAuth` and `connectorsJev` inside it,
`contextCompile` (inside a chat's `promptAssembly`), `promptAssembly`,
`repoSnapshot`, `command` with `codexInventory` inside it, `spawn` (the
spawn call → the activity clock's stamp), `firstStdout`, `firstText` (each
from the spawn). `totalMs` is admission's start → the spawn, `null` when it
never spawned. Marks are taken in memory (`prep_timing::InMemory`, never
serialized or compared) and turned into the timeline when the receipt is
written; absent on a manifest built outside an admitted run; since O2
retrieval and connectors start together (since S29 a chat turn's
`repoSnapshot` with them, on its own budget, before `promptAssembly`), and
`deadline` `{budgetMs, cut}`
lists the optional parts the preparation deadline cut: `retrievalJev`,
`connectorsJev`, `retrievalSemantic`, `microsoftAuth`, each but the Jev ones
with its own `fallbacks` entry; since O3 `codexInventory` `listed`/`cached`
on a Codex run); `compiler` (chat turns only,
F4: `entry` `composer`/`appSession`, `session` `new`/`resume`/`reset`/`retry`
with `resetReason`, `profile` `{enabled, sent, bytes, sha256, files,
truncated}` where `sha256` is the digest's revision, recorded even when not
sent, and `continuity` `{sent, available, entries, bytes, continuedFrom,
seeded}`, the last two counting the earlier conversations read and the
entries they gave, `taskState` `{revision, throughTurn, items, sent, bytes,
merged}` (K2: folded every turn, sent only fresh), `profileDelta`
`{baseline: fresh|stored|adopted|off, sent, added, removed, files, bytes}`
and `visualGuide` `{delivery: prompt|turn|given|onDemand|off, cue, file, bytes,
sha256}` (G1: how the body got the visual guide; `cue` is Hollow's own word,
`numbers` or `data file`, never the request), `budget` `{provider, total,
configuredChannel: systemPrompt|prompt, used, over, layers: [{name, tier,
wanted, granted}]}` (G2: a grant below its ask is a cut) and `stableLayers`
`{configured: systemPrompt|full|held, workspace: none|full|delta, reason:
noBaseline|otherSession|refresh|changed|budget, fullBytes, sentBytes,
unchanged, changed, excerptsReferenced, resumedSinceFull}` (G2); a layer
`configuredHeld` replaces the four appended-prompt layers when the prompt
carried the reference instead; a failed resume
retried fresh replaces it and adds a `session` fallback); `agentReads`
(`{readSource, readSourceFailed, searchVault}`: the agent's own
`hollow_vault` calls, counted from the run's tool calls; present when the
server was attached or called, 2026-10-07; plus `notesOpened`, up to 32
vault notes it opened outside its folder, S26); a retrieval candidate's
`cited` and the record's `citedChecked` (S26: the final reply named that kept
passage; flags only). **Hashes and metadata only**:
never prompt, request, passage or Jev request text, and there is no raw-text
switch (the run's transcript already holds the text). **Storage:** the
receipt carries `contextManifest` (`schemaVersion`, `bytes`, `sha256`) and the
manifest is the sidecar `{vault}/.evidence-manifests/{runId}.json`, written
and pruned under the receipt store's lock, so it lives exactly as long as its
receipt (the newest 500). A symlinked folder is refused, and a sidecar that
doesn't match its receipt's hash is refused on read. It is diagnostic:
nothing decides on it. **Read:** IPC `run_context_manifest({runId?,
sessionKey?})` (and the `history` read app action) returns
`RunContextView` (generated): with `runId`, that run's or none; else with
`sessionKey`, that conversation's newest or none (never another
conversation's); with neither, the newest chat or workflow run's (a beat's
is read by its id); Settings → Context's "Last request" line shows the
newest run's and names it. Since O3 the view also has `afterRun` (that
run's duplicate checks that ran after its receipt: a chat turn's learning
proposals, from memory, lost on restart) and `jevCalls` `{sent, notSent,
latencyMs, outcomes, costUsd: null}` over the manifest's `jev` and
`afterRun`'s requests, so a run's Jev use reconciles. Since F10 D1 it also
has `delegations` (`ManifestDelegation`: each bounded consultation or
durable child task the run started, with `kind`, `tool`, target `agent`,
`childSession`, the packet's `packetBytes`/`packetSha256`, counts, the
`budgetSeconds`/`budgetUsd` asked and the `deadlineSeconds` Hollow enforces),
read from the session store's `delegations` table, never the packet's text;
a child's own manifest measures its packet as the `taskPacket` layer.
**Duplicate-check
reports** (`jev_decide`): each check names its `Job` (`heartbeat` or
`learning`) and run, and its `Report` (with `job`, `runId` and its `request`,
a `JevUse` of use `proposals`, or `null` when nothing was compared) is kept
per run, the newest 64 in memory, instead of one process-global "last"
report a concurrent check overwrote. `jev_decisions_report({runId?})` answers
that run's newest or the newest overall (Settings).

Jev (`jev.rs`, `jev_decide.rs`) is optional and off unless its key is set:
context ranking and suggestion duplicate checks, each with a local fallback. A confident duplicate that brings a number, amount,
date or version the matched record doesn't have, where that record has values
of its own, is flagged for review instead of dropped ([JEV.md](../JEV.md)).

## Onboarding and directory setup

First run (`Onboarding.tsx`) shows the storeless `DirectorySetup`: one Hollow
folder (default `~/Hollow`, with `Projects/` and `Vault/`; existing folders are
linked in as symlinks, never moved), then an explicit restart when the vault
changed. The same component is Settings → Sources → Hollow folder. Contract:
[The Hollow folder](../DIRECTORIES.md).

**Setup sheet (H6, 2026-11-19).** After that restart a sheet opens over the
app (`components/FirstRunSheet.tsx`, mounted by `firstRunMount.tsx` in its
own root, never from App.tsx): Connect your context (the Context layer rows),
Find what's already here (`components/ImportConversations.tsx` for Claude
Code and Codex, asking before it reads, and the CX19 scan into CX20's review
card), Where your work lives (Settings, Sources' role rows, the task tracker
panel behind Connect a task tracker), Your agents (read-only) and Start (a
first workspace or Chats). Each step has Skip, Back and dots; Close finishes
later. Progress is config.json `setup` (`first_run.rs`): the folder setup
writes a fresh block only into a config with no `vaultPath`, a config with no
block is complete, and Settings → Sources → Where your work lives → Run setup again reopens every
step. Browser mock: `?firstRun=1`.

---
