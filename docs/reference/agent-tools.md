# Agent tools and permissions

Conversation organization has COO parity (2026-11-16): read saved state with
`sessions.list_conversation_organization`, then call `sessions.organize_conversation`
with `key`, `action` (pin, unpin, snooze, wake or settle), `expectedRevision`
and `expectedActivity`. Snooze also needs `until`, a future RFC 3339 time.
An unlisted key has revision zero; `read_session` supplies current activity.
The command shares the window's guards and side effects. Settle expires pending
questions; Wake restores the conversation but not those questions. These are
read/write actions, with no COO exception.

`sessions.configure_conversation_settlement` is also a write action, with
`key`, `action` (idle, merged or off), `expectedRevision` and `expectedActivity`.
It shares the window's guarded configuration handler. The policy is one-shot;
the window checks it each minute while open. Read its status and exact PR
binding with `sessions.list_conversation_organization`. No COO exception applies.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the subsections of CLAUDE.md's
"Chat: how the headless agent integration works", moved there word for word on
2026-10-13 (only relative links were re-based). Read it before changing
permission modes, teamwork, computer control, the messaging
policy, the vault search tools, owned MCP servers and their brokers, the task
tools, `hollow_app` or question cards. Keep it current with the code in the
same change, as AGENTS.md asks.

These subsections sat under CLAUDE.md's
`## Chat: how the headless agent integration works`, now in [Agent
integration](agents.md#chat-how-the-headless-agent-integration-works).

The Session importers subsection (2026-09-30) that opened this doc went with
the feature in run 15 P4 (10-27): 0 of 1,104 conversations it found in other
apps were ever added. Its modules, the background scan (Claude Code, the
Claude app, Codex and `~/Downloads`), the 11 IPC commands and the `imports.*`
app actions are gone; `session-imports.json` and the session database's
`external_*` tables stay on disk, unread (nothing dropped). The IPC commands
that read Claude Code's own history (`list_sessions`, `read_session`,
`delete_session`) stay for agents and other readers.

### Agent teamwork

`hollow_team` (`team_mcp.rs`) gives an owned-MCP primary `ask_agent(agent,
task, cwd?, packet?)` over the other installed agents, each at its saved model with a
180-second timeout. Delegates never get the team server (no recursion) and
run inside the primary's process group and run tag, so the turn's stop sweep
reaches them. A teammate is capped by its parent (see Permission modes).

**Two delegation contracts (F10, wave 4 · D1, `delegation.rs`).** Every text
an agent reads (tool descriptions, the capability index, the "Delegating"
guidance after the app tools) names them the same way: a **bounded
consultation** is `ask_agent` (synchronous, nothing durable, no Hollow tools,
profile or conversation; Hollow ends it after 180 s, or the packet's
`budget.seconds` if lower), a **durable child task** is `hollow_app`
`start_session` / `send_to_session` (async, its own History, Hollow's context
and tools; read the answer with `read_session`). Both take a `packet`
(`constraints`, `sources` as `{path, excerpt?}`, `expected`, `budget
{seconds, usd}`; checked by `delegation::parse`, bad input runs nothing;
`constraints` or `sources` may also be one string, a JSON array or one item
per line). The
host adds the asking chat's settled constraints and corrections and the files
its turns changed or its request names (`Parent::of_conversation`: the stored
task-state record plus the unfolded turns, read only) and renders one
`<hollow-task-packet>` block (≤ 6,000 characters, 10 constraints, 12
sources, excerpts ≤ 600, `expected` ≤ 1,200; host items drop before the
parent's): first in a consultation's prompt; first in a child's message body
(after the backend's wrapper), ahead of the bundle, task state, profile
and a visual turn's guide (`context_compiler::compile_with`;
`chat::TurnGuide::apply` places the guide against the text past the
packet). A child task's budget is advisory; a
consultation's `usd` is too. Each delegation is a `delegations` row in the
session store (≤ 32 per run, 90 days), listed by the run's manifest view.
Codex gets `tool_timeout_sec=240` on `hollow_team` (its 60 s default ended
long consultations). The `delegate_grant` intersection is unchanged.

**The handoff record (OR1, 2026-11-17).** `sessions.handover`, `sessions.start`
and `sessions.send` take an optional `handoff` argument beside `packet`
(`delegation::parse_args`; without it nothing changes): `objective`, `state`,
`nextAction` (exactly one; a one-item list is taken, more are refused),
`evidence` and `omissions` (lists, or one string a line each; at most 20 of
500 characters; objective and next action at most 600, state 4,000). The host
refuses, with nothing queued, a record with no next action or no state, and
evidence that names nothing Hollow can find (`delegation::check_evidence`: an
http(s) URL by form only, a session key the store has, or a relative path that
exists under the calling chat's working folder, then the vault, a trailing
`:line` or `:from-to` allowed). Absolute paths, `..`, NUL, `file:` forms and
links leading out are refused before the disk is touched, with the same
sentence as a missing file, so nothing outside the roots is revealed; no file
is read). Refusals name the fix and classify as `INVALID_ARGUMENT`. The objective
is fixed by the caller: a start or send must state one; a handover keeps one
a validated record set (`app_sessions::predecessor_objective`: the chain's in
the handover registry, which now keeps `objective`, or the one a
`start_session` with a record gave, `.hollow/handoff-objectives.json`),
ignoring a different one the caller passes, and otherwise takes the caller's as
written; `handoffNote` says which. It is never derived from a first message or
task state. The agent taking over gets the
record as a fixed `<hollow-handoff-record>` block after any task packet and
before the task text, naming who wrote it (a handover's "your predecessor
<key>") and that only its shape and evidence were checked. Every `<` in its
fields is escaped, and a handoff tag in any case or spacing in the caller's
own text too (every child prompt, record or not). The block's body is the **lane-ledger
format**, so a ledger can paste or hand-write the same block:

```
### Handoff record
Objective: <what the work is for>
State: <where things stand>
Next action: <exactly one step>
Evidence:
- <path, session key or URL>
Omissions:
- <what was left out>
```

**The child verdict (LF1, 2026-11-17, `child_verdict.rs`).** The return half of
the handoff record. A parent starts or messages a durable child with
`packet.verdict: true` (child tasks only; `ask_agent` ignores it); the packet
then carries a `finish` line, and the child ends by calling
`sessions.submit_result {verdict: pass|rework|fail|human, reason, findings?,
action?, reworkTarget?}` (catalog action, no new first-class tool, so the tool
list is unchanged; allowed in unattended turns). `reason` is required (600
characters); `human` needs `action`; findings at most 10, each at most 400
characters; refusals use the envelope's phrases (`is required`, `must be`). A
`rework` with no `reworkTarget` is recorded as `fail` (`submitted: "rework"`
and a `note` say so). The first valid verdict stands (a second call is
refused); a new `verdict: true` send starts a fresh expectation. Without owned
MCP the child ends its answer with a fenced `hollow-verdict` JSON block (same
fields plus the `nonce` Hollow put in its packet's `finish` line; the last such
block wins). A fenced `pass` without the nonce is refused (so text the child
read can't forge one; the hint goes in the retry); fail, rework and human don't
need it. **One retry:** the chat-end hook
(`main_agent::after_chat_finished`) finds a child whose turn ended `Done` with
neither, marks the retry before sending it ("do not redo the work, call the
tool"), and if the retry turn also ends without one records `fail` with
`via: "host"`. A failed or cancelled turn is left open (`state: expected`); a
held send is not tracked. **Reading it:** `sessions.request_status`
(`childVerdict`, for the call that started the child) and `sessions.conversation`
/ `read_session` (`childVerdict`) return `state: recorded|expected`,
`mergeAllowed` (true only for `pass`) and the record; a landing script can read
`<vault>/.hollow/child-verdicts.json` (entries by child key, newest 300:
resolved ones are trimmed first, never an open one; an unparsable file is
kept as `.bak` and never overwritten, readers get `state: unknown`).
`state: stale` (never mergeable) means the chat got a later turn since the
verdict; a verdict from a chat nobody asked is `unsolicited` (never
mergeable); a claimed retry that never finishes becomes a host `fail` after 30
minutes.

**Completion, stop and cost (F10, wave 4 · D2, `descendants.rs`).** Every
tool that waits on a child or teammate (`delegation::WAITING_TOOLS`) ends at
least 15 s before the caller's own tool timeout (`caller_tool_timeout`: Codex's
`tool_timeout_sec` from `agent::codex_tool_timeout_secs`, 60 s where Hollow
sets none; Claude Code's is unset, so none is shorter). The team and app
servers run tool calls as they arrive (`team_mcp::serve_lines`,
`app_mcp::serve_lines`, a thread per `hollow_app` call), so each bound counts
from the agent's send: each used to answer one at a time, so a second
consultation waited 210 s before its own began (past Codex's 240 s) and a
second `wait_for_session` up to 315 s (past 480 s). A SIGTERM still drops
every consultation in flight. A child task's delegation row keeps its
turn's run-queue row (`childQueueId`). Its end reaches the asking agent once
(`session_store` `child_deliveries`, by queue row): `wait_for_session`
returns it (at once when it ended, or when it is queued behind the vault the
caller's own turn holds), or the asking conversation's next turn opens with a
`<hollow-child-finished>` note (an injected block, after a packet, before the
bundle; budget tier reserved; a retry of the same turn repeats it). The
same note lists, under its own line, turns that ended further down the chains
the conversation started (a child's child, a handover's successor; overnight
C1, `descendants::chain_lines`: it follows only the turns the chain caused,
from each child task's run to the child tasks that run started, one query
per level, ≤32 levels and 512 runs below the seeds, reporting the last 7
days), once per
conversation (`root_deliveries`, keyed by root and asking turn, separate from
the direct parent's report); the COO's daily threads share one root,
so last night's chains reach today's thread. A stopped
chat turn (status `cancelled`) stops the child turns it queued before its
lease goes (`chat::stop_child_turns`: queued → cancelled, claimed → stop
requested, running → signalled); a normal, failed or stalled finish leaves
them running, as before. `descendants::rollup` gives a run's own cost, each
consultation (`delegate` rows), each helper one-shot (the learning check now
carries its turn's run id, `Meter::oneshot_for_run`) and each child turn's own
roll-up; unknown stays `null` and nulls every sum it is in, `knownUsd` adds
what is known, and a child that never ran cost 0. The receipt seals it
(`descendants`, with each child's `stop`); the manifest view reads it live
(`costRollup`).

### Computer control (vetted desktop verbs)

`hollow_computer` (`computer_mcp.rs`, chat only, Settings → Computer control)
offers fixed, parameterized verbs and **no free-form AppleScript**. Tier 1
(`open_url`, `search_bookmarks`, `open_app`, `list_apps`) runs without a card
where the permission mode allows it; tier 2 (`browser_current_tab`,
`browser_list_tabs`, `quit_app`) shows an Allow/Deny card through the
`approvals.rs` broker (120 s timeout), bound to the exact tool call. While it
is attached, `Bash(osascript:*)` and `Bash(open:*)` are denied. The tab tools
know Safari, Chrome, Arc, Edge, Brave and Helium, plus any running Chromium app
whose bundle declares Chrome's AppleScript dictionary (PER-35); Firefox and
LibreWolf have no AppleScript tab dictionary and aren't supported.

**See and act (Cua, CU1, 2026-11-18).** `computer_broker.rs` is the GUI-side
broker for the see-and-act tools Cua Driver (MIT) provides, per the design in
the hollow workspace (`outputs/cua-computer-use-design-2026-11-15.md`). It
holds an MCP stdio **client** to a pinned `cua-driver mcp` and is the only
thing that talks to it; no agent gets the Driver's own server. It serves on a
per-pid `computer-<pid>.sock` with a proof (`sha256(secret, "computer", run id,
scope, mode)`) and the `run_auth` peer check, refuses `plan`, and serves
nothing until `computer-engine.json` in the support folder pins an exact
version and SHA-256 (none ships; a bad pin serves nothing). The **tier map**
(observe, act, reach) fails closed: a tool with no row is never listed and is
refused; `install_extension`, `parse_visual_regions`, `set_config`,
`get_config`, `check_for_update`, `install_ffmpeg`, `run_script`, `history_*`
and recording tools are hidden; a Driver tool named like one of the seven
verbs above keeps Hollow's meaning; `launch_app` and `kill_app` are sandbox
only; `run_steps` runs only when every step is classified. Results are cut
to the size caps (text, two images, nothing else). The `Target` seam is
`ThisMac` (the standalone `CuaDriver.app`, its binary hashed before each
session) or `SandboxVm` (`ssh -T user@host cua-driver mcp`, version
checked; the guest binary's hash is checked when its image is built). Until
CU2's cards, the sandbox serves all three tiers; This Mac's
screen serves nothing and starts no Driver until CU6. The Driver's environment
is cleared to HOME, PATH and LANG, and a Driver request back (sampling, roots)
is answered "not supported". A Driver error or missed deadline ends the
run's session; a changed pin ends it too; switching Computer control off ends
every session at once. A chat's tool list comes only from the last handshake
under the same pin and never waits on one (agent CLIs drop a slow server, the
seven verbs with it): with nothing learned it lists none and starts one
warm-up handshake whose session ends at once; only a call keeps a session.
ssh runs with `-F /dev/null`, `BatchMode` and `StrictHostKeyChecking=yes`;
how the broker reaches a VM on the runner is CU5's.
`run_steps` must be exactly `{"steps": [{"tool", "arguments"?}]}` and is
forwarded rebuilt from the checked fields. **The link:** `server_entry(vault,
perm, run id, scope)` puts the broker socket, run, scope, mode and proof in
the chat server's environment (`HOLLOW_COMPUTER_SOCK` and siblings, the run
id as `HOLLOW_RUN_ID` so `run_auth` registers it) when the
broker listens and the mode isn't `plan`; `hollow_computer` then lists the
broker's tools after its seven verbs (dropping any named like one) and
forwards every other name. The seven verbs never go to the broker. A
forwarded call's vault log line keeps its argument keys and size and whether
it ran, never values, the broker's error text or the result. The list asks
the broker for at most 5 s, a call for 125 s. **Audit (CU3, 2026-11-18):** every
call whose proof checks out (ran, failed or refused; never a tool list) appends
one `audit_log` row of kind `computer_action` (`computer_broker::AUDIT_KIND`):
`session_key` and `asked_by` the chat, `action` the tool, `outcome` `ok`,
`failed` or `refused`, `detail` a fixed category (never the Driver's words), and
`fields` with the target, tool, tier, argument names, the element and app
(cut to 80 characters), `x` and `y`, `typedChars` (plus `typedSha256` only from
16 characters up, since a short PIN's hash can be guessed), the SHA-256 of each
screenshot's base64 and `card` (the card kinds that let the call run, or null). The row's key-based redaction
still applies, and `audit::verify` covers it. The mini's append-only trail for
the sandbox target waits on CU12's `computer-audit` wrapper. **Cards (CU2,
2026-11-19):** `computer_broker::required_asks(mode, target, tier, flags,
signed_in)` decides which cards a call needs; the broker answers `{ok:false,
needsCard:{kind,text}}` (nothing ran, one audit row with `detail` `card
needed`) and `hollow_computer` shows each (`ask_with_cards`, at most four per
call), then sends the call again with the allowed kinds in `approved`. Only
that server speaks to the socket, behind the run proof. `default` asks once
per chat to use the sandbox (`useSandbox`, kept per chat, cleared by Stop all
and Computer control off); `acceptEdits` and `auto` ask nothing for observe
and act; tier 2 (`reach`) asks each time in every mode but bypass. **Always
asks, bypass included:** an outward act (`outward`: a click or key on a
label like Send, Submit, Pay, Delete, Cmd+Return, or in Mail or Messages and
similar; only on This Mac or a sandbox whose pin says `signedIn: true`) and
typing into a password or secret field (`secureField`: Deny and the person
types it). The guard reads only the call's own arguments (labels, keys, app
ids): a heuristic that misses unlabeled icon buttons, and the sandbox is
the real boundary. Reading the label from the last window-state snapshot
waits on CU0 showing the Driver's snapshot format; CU15 measures the misses.
This Mac asks `firstUseThisMac` in every mode (served from CU6). Plan
attaches nothing (the broker lists none and refuses a call). Each run is
bounded (`run_manifest`: 30 minutes, 5 minutes idle, 300 acts) and the broker
enforces it itself, plus a loop guard (the same call three times in a row, or
five failures in a row, ends the session); CU5 and CU6 pass the manifest to
the Driver they start. `mode_contract::computer_screen_brief` tells the
agent what each mode does. **Settings sheet (CU7, 2026-11-19):** Settings, Connectors, Built into Hollow,
Computer control has a "Set up…" button that opens a sheet
(`src/settings/ComputerControlSheet.tsx`, its own lazy chunk): the Basic
switch, **Sandbox Mac** (default on) and **This Mac's screen** (default off),
**Stop all computer control** (`computer_stop_all`: ends every desktop session
and forgets the per-chat grants; works with no chat open) and the Doctor's
checks from `computer_status` (switch, broker listening, pinned engine, each
with a copyable fix; CU10 adds the checks that need the Driver and the VM).
The two target switches ride `set_computer_control_enabled` (the launch call:
no new launch IPC) into `computer_broker::set_targets`; a target switched off
refuses its calls (`SANDBOX_OFF`, `THIS_MAC_OFF`, one audit row) and turning
one off ends live sessions. This Mac's screen stays refused until CU6 whatever
its switch says. **This Mac's screen (CU6, 2026-11-19):** with the Settings switch on, the
broker serves observe and act on This Mac through the standalone
`CuaDriver.app`, bounded by the person's grants (`computer_thismac.rs`).
**Exactly one app is named per call** (`bundle_id`, or a name, however many
keys repeat it; a bundle id and a name together, two different values, a
non-string, a nested object, a process or window id, or any other key that
looks like it aims the call is refused), so the app Hollow checked is the only
one the Driver is told about. An unnamed action is refused; an unnamed
observe is a look at the whole display (its own `display` card, **in every
mode, bypass included**, since it can show a denied app). First use in a chat
always asks (`firstUseThisMac`, bypass included, naming who receives the
screen); outside bypass each app is its own per-chat card (`app:<bundle id>`).
**A default deny list** (`DENY_GROUPS`: Terminal, iTerm, Ghostty, Warp,
Alacritty, kitty, WezTerm, Script Editor, System Settings, Keychain Access,
Passwords, 1Password and its family, Bitwarden, LastPass, Dashlane, KeePassXC,
Cua Driver and Hollow itself, sub bundles included) is refused in **every**
mode, with no card. A value is tested as a bundle id and as a name whichever it
was given as, and names are normalized (`.app` and spacing dropped, "1Password
8" is 1Password). The person allows or blocks a group in Settings, Connectors,
Computer control, Apps agents can't use on this Mac
(`computer_denied_apps`, `computer_set_app_allowed`, webview-only, no agent
tool): it writes `computer-this-mac.json` (`{"allowDenied": [...]}`, 0600, in
Hollow's support folder, replaced atomically; an unreadable file or a loose
wildcard widens nothing; a shell-capable agent in bypass could write it, the
same trust as the engine pin). The Driver is started per run with a `bounded`
manifest file (0600, removed with the session) naming exactly the apps and
display that run has passed the checks for; a wider grant restarts the Driver
wider, an app the person blocks later drops out (it restarts narrower), and
the tool list is learned by a warm-up whose manifest allows nothing. A sweeper
(every 30 seconds) ends a run past its 30 minute expiry and a This Mac
session idle for 5 minutes, with no call needed. **Unverified until the
Driver is installed** (Alex's step, CU0): the argument names that carry the
app (`APP_ID_KEYS`, `APP_NAME_KEYS`, `TARGET_KEYS`, `AIM_WORDS`) and the
manifest's field names and `--manifest` flag, each in one place in
`computer_thismac.rs` and `EngineConfig::command_with`; if the real Driver
needs a process or window id, that is a decision for the install, with the
manifest enforcing apps on the Driver's side. The first-use card says "the
company behind the agent's model" (the request doesn't carry which agent; the
card naming it is a follow-up). Tests use a stub Driver only. **Shell path (CU4, 2026-11-19):** `hollow computer
tools` and `hollow computer call <tool> '<json>'` (`computer_cli.rs`, hidden, like
`code-context`) run the same `computer_mcp::handle_with` the MCP server runs, over
the same broker socket and run proof (`computer_mcp::broker_env`), so the result,
cards, modes, limits and audit rows are identical for Claude, Codex and a shell;
images are saved to new files and their paths printed; exit 1 for a refusal, a tool error or no run link
(the CLI never runs a native verb without one), 2 for bad input. It never reads
the bypass flag from the shell's environment (an agent controls that), so tier 2
always asks there. Grok and Antigravity have no per-run MCP flag, but
whether their sandboxes can reach the Unix socket is **unverified** (it needs one
paid test turn, which is Alex's call), so Hollow puts the environment in no
non-owned run and `agent::capabilities` carries `desktopUnavailable` for them
(the picker note is CU8's). Wiring `broker_env` into their launch waits on that
probe. **The chat's desktop (CU8, 2026-11-19):** `computer_activity.rs` keeps, in memory by chat
(the broker's `scope`), whether the person paused or took over the desktop, the
last 50 actions (tool, tier, target, outcome, a fixed detail, the app on This
Mac) and the screenshots of the newest 5 (a `data:` URL the broker already
passed to the model, never written to disk; at most 32 chats). A paused or
taken-over chat's calls are refused ("paused" / "taken over") and recorded,
other chats are unaffected. `computer_chat_state`, `computer_chat_frame` and
`computer_chat_control` (`pause`, `resume`, `takeover`, `handback`, `stop`) serve the
composer chip (`ComputerChip.tsx`, shown once the chat has made a
`hollow_computer` tool call) and the side card's Computer section
(`ComputerCard.tsx`: the frame, the actions with their tier, Pause, Take over,
Hand back, Stop), polled every 2 seconds while visible, never before the chat
used the desktop. Each control leaves a `computer_control` audit row. **Stop on a chat**
ends that chat's runs' sessions and forgets its grants; the tray's **Stop all
computer control** and Settings' Stop all end every session and clear every
chat's state. The agent picker's rows say "Can't use the desktop in this chat"
for an agent with `desktopUnavailable`. **Doctor (CU10, 2026-11-19):** `computer_doctor` (`computer_doctor.rs`) is the list under
Settings, Connectors, Computer control, Set up, Doctor: each check has a state
(`ok`, `warn`, `fail`, `unknown`), a line and, for a failure or warning, a
copyable fix in words (Hollow never runs it and installs nothing). It checks the
switch, the broker, the engine pin (missing, invalid with its reason, or pinned),
This Mac's Driver against its pinned SHA-256, the sandbox's key file (present,
mode 0600) and whether its host answers on the ssh port (name lookup included, one second at most, off the main thread), free disk
space (warn under 5 GB, fail under 2 GB) and the allow file. What needs the
Driver running or the VM (the Driver's permissions and telemetry, Lume, the VM
volume) is `unknown` and says "Not checked"; CU0 and CU5 add those. **Wave 5 additions (2026-11-19):** *Host scope (BG1, `host_scope.rs`):* one matcher,
deny wins, an allow list makes the rest denied; `example.com` matches itself and
subdomains, `*.example.com` subdomains only; a scheme, path, port, mid-host
wildcard or non-ASCII pattern is rejected, and one bad entry in
`host-scope.json` closes the scope (every host denied). `ui.read_tab` and `ui.inspect_tab` refuse a
page on a built-in webmail or banking host or the person's deny list (their
`allowDefaultDenied` lifts a host); `open_url` applies the person's lists only
when they wrote any. *Restrict-only ratchet (ND1, 2026-11-20):* an agent adds a host to its own deny list with `computer.deny_host` (tier Write, no card, because it only narrows access); entries live in `host-scope-agent-deny.json` in the support folder, capped at 100 (20 per chat; a shared suffix such as `co.uk` or `github.io` is refused, name the site), read at every decision (also by `open_url`, `ui.read_tab` and the touch audit), checked as text by the same parser as the person's lists (a pasted address reduces to its host; no link or redirect is followed, each navigation is judged by its own host), and an unreadable file closes the scope. Only the person removes an entry (`host_scope_remove_agent_deny`, Settings → Computer control → Sites agents blocked for themselves). *Touch audit (BG2):* a computer call's row lists the hosts
its arguments and result name (`hosts`) and each the lists do not allow
(`outOfScope`); a flag is evidence, never a refusal. *Blocked hotkeys (OC3,
`computer_keys.rs`):* lock screen, log out, Force Quit, empty Trash, quit the
front app and shut down are refused before they are sent, in every mode, also
inside `run_steps`. *Connector hygiene (CD1, `connector_hygiene.rs`):* a table of
known connectors (chrome-devtools-mcp) with a version-pin and opt-out check;
the Doctor warns per missing item. *CLI (CD5):* `hollow computer status`, and a
tool's own name works as `call`; the verbs are the broker's list, with a test
that none shadows a subcommand. Waiting on CU0 and CU5: OC2 (observation
shaping needs the Driver's real output), OC12 (VM capability flags need CU5).
The shared HTTP host (`shared_mcp_host`) has no link.

### Messaging is unavailable to Hollow agents (one approved mail connector)

No built-in Hollow tool sends or reads messages; the built-in Microsoft 365 connector
refuses mail, Teams, chat, meeting and contacts routes
(`m365_mcp::allowed_path`). Claude runs use strict MCP configuration and Codex
runs disable inherited servers, apps and plugins. **Exception (10-11 B4, Alex's
call):** Superhuman Mail's MCP server is an approved mail *connector*
(`remote_auth::APPROVED_MAIL_HOSTS`, mirrored in `src/connectorsLibrary.ts`):
- Hollow signs in to it with OAuth (dynamic registration) and asks for `offline_access`
  among the standard scopes.
- It is shared with Claude and Codex and attached when relevant.
- Its `hollow remote-mcp` bridge asks on Hollow's approval card before every tool that
  isn't one of Superhuman's listed reads (`remote_mcp::gate`, `is_read_tool`). The gate
  is decided by the URL (https only), and a deny or timeout never reaches the server.
- Batches and id-less `tools/call` are never forwarded. A write cancelled by its agent
  isn't sent after a late Allow, and Codex waits 180 s (`MAIL_TOOL_TIMEOUT_SECS`) past
  the card's 120.
- The approval socket is trusted only when the process behind it is an ancestor
  (`approvals::broker_is_ancestor`); otherwise the native dialog asks. Cards bind to the
  chat's `mcp__*superhuman*__<tool>` call.
- A plain URL connector to the host is refused, and "Use Hollow's instead" can't keep
  claude.ai's row over Hollow's.
- **Limits:** a shell-capable agent in bypass can read `RemoteAuth/<id>.json` (a
  plaintext refresh token, like every remote connector's). Before Hollow's connector is
  signed in, the claude.ai Superhuman row still attaches for Claude, ungated by Hollow,
  as before.

It is still no inbox: no mail sync into the vault, no inbox UI, no message approvals.

### Ask your vault (read-only search and readers)

`hollow_vault` (`vault_mcp.rs`, chat only) offers three read-only tools.
`search_vault {query, limit?, kinds?}`: every word must appear, file
name/title ranks above headings above body, plus the ranked passages
automatic retrieval draws from, cut to `limit` instead of the 8-passage
evidence budget. Both halves come from the host retrieval broker
(`retrieval_broker.rs`, agent effectiveness F3), which automatic retrieval and
the `vault.vault_search`, `vault.semantic_search` and `vault.retrieve` app
actions also call, so candidates, ranking, scope and paths can't drift. Root
and scope are separate fields: `HOLLOW_VAULT_ROOT` is always the configured
vault, never the chat's cwd (a workspace folder or a build worktree is not
the vault), so the keyword half searches the whole vault;
`HOLLOW_VAULT_SCOPE` is the conversation's connected project
(`retrieval_broker::resolve_scope`, the rule automatic retrieval uses; empty
for none) and limits only the connected-project half. The same note gets the
same vault-relative path here, in the app actions and in retrieved evidence.
The server never reads `HOLLOW_VAULT` (the other servers' cwd). It reads the
app's FTS database read-only (never writes it; notes changed since the index
saw them are always read). It has no semantic index or embedder, so its
passages are lexical and its answer's `ranking` field says so; when the app
blends semantic candidates, automatic retrieval can rank differently from
`search_vault` for the same request. When the same run also has `hollow_app`
(`HOLLOW_VAULT_APP=1`, set by `mcp_attach`, which attaches the app server
first; empty otherwise), a lexical-only answer says "This search is lexical
only" and names `run_action vault.retrieve {"query": …}` for ranking by
meaning "when the app's semantic search is on" (S03): the app ranks lexically
too when it is off or not ready, without search terms, or for
connected-project passages alone. The flag is all it
gets: the server has no channel to the app. Its passages follow the lifecycle
policy (a replaced learning is left out unless the query asks about the
past, then labelled historical), and its keyword list labels a replaced
learning with its current version.

`read_source {path, from_line?, to_line?}` (agent effectiveness F3 part 2,
`source_evidence.rs` through the broker): numbered lines of one file, at
most 600 lines or 24,000 bytes per call, with the line count and a "call
with from_line N" pointer, so an agent reads the rest of an attached source
or a retrieved file instead of guessing. It reads only what Hollow could
already put in front of the run: the run's own workspace folder
(`HOLLOW_VAULT_WORKSPACE`, from `resolve_scope`; a relative path is tried
there first, so the envelope's `sources/…` paths work), vault notes
retrieval may cite (`vault_eligible`: not `me/`, `sessions/`, `workflows/`,
or other workspaces' files outside `outputs/`), and connected project files
in `HOLLOW_VAULT_SCOPE`, or of any connected project when that is empty
(`directory_connections::read`'s membership check); its tool description
names whichever bound the server has.
Hidden and secret-looking names are refused, and the policy is applied to
the resolved path, so a link can't lead out of the vault or into `me/`.
Antigravity and Grok get no owned MCP, so their envelope says to open the
file with their own tools, with the workspace folder's path.

**Near misses (NM1, 2026-11-17).** When `search_vault`'s keyword list is
empty, or `read_source` misses a vault-relative path ("No such file"), the
answer names up to five vault notes nearest the query or path by spelling
(`vault_mcp::nearest_notes`, no model): names are lower-cased with every
other run of characters one space, and a note scores the better of the
normalised edit distance and trigram Dice overlap against its title (its file
name) and its path (the run's own `workspaces/<id>/` prefix left off), per
mille, at least 450 to count. Only notes `read_source` may read count (eligible,
no hidden or secret-looking component); the edit distance runs only on the
2,000 best by trigram overlap. Order
is score, then path, so the list is byte-stable. `search_vault` adds the
lines after its old text and a `nearest: [{title, path}]` field in its
envelope block; with none close it says so in one plain line and `nearest`
is `[]`. A `read_source` miss keeps its sentence ("Did you mean `a`, `b`…?",
so the refusal's `didYouMean` holds the first three) and its refusal block
adds `nearest` with all five.

`read_skill {name, source?, reference?}` (F9, wave 4 · V2, `skill_index::read`): one
skill's whole `SKILL.md`, at most 64 KB, by the name a turn's
`<hollow-skills>` block lists, from the vault's `.agents/skills` or the
shared library `HOLLOW_VAULT_SKILLS` names (`~/.agents/skills`, passed
explicitly because an MCP server's environment may have no home folder) and
nothing else. `.agents` and `.agents/skills` must be real folders and a
skill's file must resolve inside its root, so no link leads out; `source`
(`vault` or `shared`) picks a clashing name's other copy. OC8: a skill whose
folder has `references/*.md` is a kernel plus named references; the answer
ends by naming them and `reference` (a name, without `.md`) appends exactly
that one, at most 64 KB, a regular file directly in that folder (never a
link). `hollow_app` `skills.read_skill {path, reference?}` does the same
(`skill_index::app_answer`). A skill without references answers byte for
byte as before.

**The retrieval broker** (`retrieval_broker.rs`; contract in its module doc
and [AGENT_CONTEXT.md](../AGENT_CONTEXT.md#retrieval)). Scope
(`resolve_scope`): a connected project's workspace folder, a build
workspace's bound repository, a folder inside a connected project, or a
linked git worktree of one, from records and paths, never a folder name (an
arbitrarily named worktree keeps its project). A project's workspace
(`workspaces/local-…`) whose project left the index keeps its own id
(`workspace-disconnected`): no connected-source evidence, never every
project's; so does a build workspace whose repository no connected project
holds (and `read_source`'s description then promises no project). Candidates: connected projects
(≤ 4,000 files, 32 MB read), the org context clone's files while its root is set (kind `org`, ≤ 400
files, 4 MB; CX5b) and eligible vault notes newest first: with
the FTS index, only its candidates (OB12: the best 200 by BM25 over a
word-tokenized table, the terms ORed with a prefix match from 3
characters, and under 200 the trigram matches of every term of 3+
characters too, at most the rest of the 200 and 4 MB, best first (RF-737),
plus up to 50 notes whose file name names a term, the 20
newest and any note changed since it was indexed); with none, or with a
kinds filter, every eligible note (the linear scan, also the test
oracle). With no project passage, a vault note's BM25 band leads (one per
factor of 1.25 below the best score; a note's later lines one band
lower), then line coverage. With any project passage the order is the
pre-OB12 one, coverage first, the band only a tiebreak. The 32 MB vault budget is charged only for notes that
match, so newer unrelated notes can't starve an older exact match, and a
256 MB read limit bounds a linear scan. With semantic search on and its
index open with embedded notes, the request is embedded locally (the bundled
bge-small model; no network, no download) and notes at cosine ≥ 0.6 join by
reciprocal-rank fusion (k 60, equal weights, total tie-breaks); otherwise
lexical only, with the reason recorded. The whole candidate set is ranked
(a sort plus linear passes, so tens of thousands of passages stay cheap), a
file's third and later passages are demoted (diversity is a ranking rule,
not a cut), the lifecycle policy runs on the whole ranked set (F7, see
"One lifecycle policy" under Alignment), the first 24 form the pool the
reranker (Jev, when on) sees, and the one evidence budget, 8 passages, is
applied after it (a historical passage's current version is put back ahead
of it), with a coverage notice when it leaves ranked passages out. A
labelled passage carries a `lifecycle` field in the evidence JSON, which
then says what the label means. The `vault.retrieve` read action
returns the pool for the caller's own conversation or automation scope.

**Attached sources** go through the broker too (`select_attached`,
`source_evidence.rs`; F3 part 2, 2026-10-06). Each readable file under a
workspace's `sources/` (≤ 1 MB read each, 16 MB per request) is cut into
chunks of ≤ 40 lines and 3,000 bytes, 8 lines overlapping (one longer line
is cut to the window around its best match, with "…" marks); chunks are scored
against the request plus resolved reference names
(`TurnRequest::source_query`) with the broker's coverage rule. Since OF06
(2026-10-17) only chunks past a relevance floor are sent (≥ 2 informative
terms, or ≥ 40% of the best chunk's; generic and common terms don't count;
when no chunk passes, the 3 densest go within 8,000 bytes),
best first (a file's third and later behind every other file's first two),
into one 24,000-byte budget across files, with no backfill: each file's
opening chunks fill leftover budget only when the request asks for the whole
document or all attached text fits, and a one-chunk file goes whole
([AGENT_CONTEXT](../AGENT_CONTEXT.md#attached-sources-and-the-reader-s2) has
the rule). Each file in `attachedSourceEvidence` gets
this request's `coverage` (`complete`, `excerpt`, `not_selected`,
`budget_exceeded`, `requires_reader`), a one-line `note` ("excerpt: lines
120–160 of 400; read_source reads the rest",
or a presence note with its size) and `excerpts` `[{from, to, text}]`. Name order no longer decides
anything, so a fourth file by name is reached. A piece that fits whole is
always taken, however little budget is left. The Sources panel's listing
has no request: when all attached text fits 24 KB it fills them as a
request with no terms would, so `complete` only for a file a request gets
whole (not a single line over 3,000 bytes); else `excerpt` ("Sent when
relevant"). The run manifest's `agentReads` counts the run's `read_source`
calls (and failures) and `search_vault` calls, and lists the vault notes the
agent opened outside its folder (`notesOpened`, S26); a `read_source` call shows in
the chat as "Read a source" with the file and lines.

### Look up code (read-only callers and impact, CC2)

`hollow_code_context` (`code_context_mcp.rs`) gives Claude and Codex chats and
workflows in a git checkout outside the vault (never the home folder or a root
that holds the vault) nine read-only tools: `map`,
`symbols`, `callers`, `callees`, `impact`, `read`, `slice` (CC4: a symbol's
or task's cards with a `handle`; called again with it, only changed cards or
`notModified`), and `changes` and `affected_tests` (CC5: a diff's changed
definitions, and the cargo filters and test files worth running first, with
`cannotMap` and a measured `missRate`; never instead of the full gate). Every name match is listed
with `resolved` and `how`, grouped by file with its hash, and the result says
which index it read (`repo.head`, `fingerprint`, `indexedAt`, `dirtyFiles`).
Grok and Antigravity run the same tools as `hollow code-context <tool>
'<json>'`. Contract, index and attach rules: [CODE_CONTEXT](../CODE_CONTEXT.md).

### Owned MCP preapproval (Paperclip PR 13)

Headless CLIs never prompt for an MCP tool: Claude `-p` denies an
un-preapproved call and `codex exec` fails it. So
`agent::OWNED_PREAPPROVED` preapproves `hollow_questions`, `hollow_tasks`,
`hollow_app` (its broker applies each action's tier), `hollow_vault` and
`hollow_team` (Claude `--allowed-tools mcp__<server>`, Codex
`default_tools_approval_mode="approve"`). `hollow_computer` and
`hollow_services` are preapproved only in `auto` and bypass. `hollow_team`
never attaches in `plan`. Which servers attach per surface is
`mcp_attach::server_map_for` (chat: team unless plan, computer, vault,
questions, tasks, app; workflows: team unless plan, tasks, app). Library
connectors are preapproved by `mode_contract::connector_preapproved(autoApprove, perm)`: all in
bypass, the ones marked `autoApprove` in `acceptEdits`/`default`, none in
`plan` or `auto`.

### Broker proofs off argv and peer checks (merged plan slice 6)

On macOS any process can read any process's argv and any same-user process's
environment. So a run's MCP config never rides on argv:
`run_auth::write_config` writes it to
`~/Library/Caches/hollow/run-mcp/<run tag>.json` (0700/0600); Claude gets
`--mcp-config <file>`, and Codex starts each stdio server through
`hollow mcp-launch <file> <name> <sha256>`, which refuses a file that is not
the one Hollow wrote. The app, task and services brokers then check **who
connected** (`LOCAL_PEERPID` / `getpeereid`, `SO_PEERCRED` on Linux) and walk
the peer up its parents to a registered run leader; a request is served only
when that run's ids include its `runId`, before the proof check. A copied
proof is worthless outside its run. Not covered: processes inside a run's own
tree, and a same-user process that can write this user's files (details in
the archive).

**Prompts on stdin (Q-R33, 2026-11-14).** The prompt is not on argv either.
Every local Claude and Codex run reads it from stdin (`agent::stdin_prompt`):
Claude as `-p` with no prompt argument, Codex as `exec -` or
`exec resume <id> -`. Hollow writes it to an unlinked private temp file
(`prompt_stdin_file`) and hands that as the child's stdin, so callers need no
change and nothing can block on a pipe. Steerable turns already wrote their
messages to stdin. Audit `fields` mask any string under a secret-named key
(`audit::secret_key`: `token`, `apiKey`, `password`, `Authorization`, ...)
whatever its shape (Q-R35).

**The rest of R5 (R5b, 2026-11-14).** Same rule, every other route:

| What | Route now | Proof |
|---|---|---|
| Grok's prompt | `--prompt-file /dev/stdin`, the run's stdin file (Grok reads the file before it checks its sign-in; `-p` is only the fallback for a spec with no stdin file) | `ps -E` test |
| Claude's appended system prompt | `--append-system-prompt-file /dev/fd/<n>`, an unlinked private file on a descriptor of 10 or more that Hollow holds open (`secret_fds::pass`); proven with a login present | `ps -E` test, fake-endpoint request with the real login present |
| Claude's account token | **stays in `CLAUDE_CODE_OAUTH_TOKEN`**: Claude 2.1.325 ignores `CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR` when a keychain login exists and uses the login | open until a probe passes with a login present |
| A runner run (the Mac mini) | the leader's header carries the prompt as `stdin_text` and the appended prompt as `fds`; the runner's supervisor opens each as an unlinked private file (stdin, descriptor 7) before it `exec`s the agent, so `ps` there shows neither (the token is still in its env) | `remote_session_tests::the_runner_hands_the_agent_its_secrets_on_private_files_not_argv_or_env` |

Antigravity's prompt left argv on 2026-11-17 (build run R5 residual): `agy -p= --input-format stream-json
--output-format stream-json` reads one `{"event":"user","message":{"content":<prompt>}}` line from the run's stdin file
(`agent::antigravity_stdin_message`; a bare `-p` takes the next flag as its prompt, a non-empty `-p=` value is refused
beside stream input, and a message without `event` or with a string `message` is rejected). Still on argv or env, and why: **the Claude account token** (env, see above), the **self-hosted run key**
(`ANTHROPIC_AUTH_TOKEN` or Codex's key variable: a per-run key valid only at
Hollow's own loopback forwarder, and Claude's gateway-token descriptor is not
a drop-in for it), **Codex's API key** on a runner run (`CODEX_API_KEY`), and
HTTP connector header values for Codex (its environment). A descriptor is shared with the
process's own children that inherit it, so a second Claude started with the
same descriptor finds the file already read: every run gets its own file.
Q-R34 (`CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`) stays off.

Credential masking in logs and audit (`claude_runner::redact`, Q-R32): a quoted
value is masked whatever its length, also JSON inside JSON
(`{\"token\":\"abc\"}`); camelCase token keys (`accessToken`, `idToken`, `refreshToken`, `apiToken`, `authToken`, `sessionToken`);
`passphrase`, `private_key` and `access_key` names; a short `Authorization`
value (a digit, or a capital after a scheme word: `Basic dXNlcjpw`); and a
flag's value (`--password hunter2`, `--token=x`, `--api-key 'a b'`, curl's
`-u user:pass`). Prose and code keep their words (`Authorization: token
required`, `token: String`, `--password-file ./pw`).

### Permission modes across agents (2026-10-02)

User decision: a mode means the same thing in every agent. **`auto` approves
everything and asks only when something truly needs the user; `bypassPermissions`
runs everything with no prompt at all**, Hollow's own cards included. Headless
CLIs can't prompt, so "asks" means Hollow's inline card; what a CLI would have
prompted for is otherwise denied (except Grok, which runs it; its sandbox is
the limit, see below).

Where a mode is set (P0-19, 2026-10-23): Settings → Chat sets the
**mode for new chats**, one per agent since 2026-11-12 (`newChatModeFor`:
the agent's own, else Auto for Claude and Codex, Accept edits for Grok and Antigravity), which an empty chat takes
(`applyNewChatPermissionMode`: New chat, `/new`, a workspace's new
conversation); the chat's own menu and `/permissions` set the chat on screen.
A change to the chat on screen is refused while its turn runs or it is
opening, and the setter returns the reason.

| Mode | Claude | Codex | Antigravity (`agy`) | Grok |
|---|---|---|---|---|
| `plan` | `--permission-mode plan` | `-s read-only` | `--sandbox --mode plan` | `--permission-mode plan --sandbox read-only` (`default` gets read-only too) |
| `acceptEdits` | `--permission-mode acceptEdits` | `-s workspace-write` for a runner turn or a glob in the cwd; a local turn runs the `hollow_auto` profile with only the nested-`.git` deny and network off (N22, [per-agent modes](../PER_AGENT_PERMISSIONS.md)) | `--sandbox --mode accept-edits` (commands denied) | `--permission-mode acceptEdits --sandbox workspace` |
| `auto` | `--permission-mode auto` (its classifier) | `-s workspace-write -c approval_policy="on-request" -c approvals_reviewer="auto_review"` (= `--approve-for-me`) plus `-c sandbox_workspace_write.writable_roots=[chat folders, linked worktree gitdir, shared .git] -c sandbox_workspace_write.network_access=true` ([PER_AGENT_PERMISSIONS](../PER_AGENT_PERMISSIONS.md)) | **unavailable**: refused before spawn (see below) | `--permission-mode auto --sandbox off` |
| `bypassPermissions` | `--permission-mode bypassPermissions` | `--dangerously-bypass-approvals-and-sandbox` | `--dangerously-skip-permissions` | `--permission-mode bypassPermissions --sandbox off` |

How a chat names them (per-agent menus, 2026-11-12, `src/agentPermModes.ts`,
passed through unchanged): Claude Default, Accept edits, Plan, Auto, Bypass
permissions; Codex Read only (`plan`), Ask for approval (`default`; a stored
`acceptEdits` shows as it, the same sandbox), Approve for me (`auto`), Full
access; Grok Default, Accept edits, Plan, Auto, Bypass permissions (its
`dontAsk` has no canonical mode and isn't offered; its Auto counts as danger,
sandbox off); Antigravity Accept edits (`default` shows as it), Plan, Auto,
Skip permissions. A person's agent switch moves the chat to the
nearest mode there and says so in one line (`agentSwitchMode`).

- **Antigravity's `auto` is its own reviewer (back 2026-11-20, agy 1.5.13).**
  It was refused while agy 1.2.15's only prompt-free sandbox left its file
  tools unconfined. Hollow's probe, rerun on 1.5.13 (`agy -p --mode auto`, with
  and without `--sandbox`, and over `agy acp`; workspace and oracle folder under
  `~/Hollow/Worktrees`, outside the system temp tree; shell write, write tool,
  edit of an existing outside file, `../` and a symlink out): every outside
  write failed (shell `operation not permitted`, write tool
  `denied_actions: [write_file]`), a write inside the folder worked, and
  `git status` ran. When the oracle sat under `$TMPDIR` every write escaped,
  shell included: system temp is a writable root, as it is for Claude's and
  Codex's sandboxes. Never the skip-permissions flag outside bypass.
  Headless 1.5.13 `accept-edits` and a run with no mode write inside the
  folder and deny commands (`denied_actions: [command]`), unlike 1.2.x:
  `mode_capability` still lists them read only (conservative, noted).
- **Antigravity chats ask instead of denying (2026-11-20).** A chat turn runs
  `agy acp` (`agent::antigravity_acp`; one-shots keep `agy -p`; set
  `HOLLOW_AGY_HEADLESS=1` to keep everything headless). Modes map to agy's own
  (`antigravity_acp_mode`: plan, default, accept-edits, auto); bypass starts the
  server with `--dangerously-skip-permissions`, every other mode with
  `--sandbox`. A `session/request_permission` is Hollow's inline approval card
  (`chat::spawn_acp_card`, the AG4b card path): Allow answers `allow_once`, Deny,
  a timeout, a Stop or no app answer `reject_once`, never an allow by default.
- **Auto preapproves no library connector** (each call goes to the agent's
  own layer, because a connector can reach off this Mac); bypass preapproves
  every one.
- **Services are bounded in every mode but bypass** (decision 1,
  `services::bounded`): a bounded service runs inside the bound worktree,
  belongs to its chat and stops when that chat is archived, on stop, when the
  worktree is removed or when Hollow quits. The grant is tied to the run,
  chat and mode by its proof, and refused once the turn ends.
- **Hollow app cards** (`app_actions::asks_in_mode`, chats only): bypass shows
  none; auto shows them only for external actions, the confirm actions in
  `app_actions::AUTO_ASK_CLASSES` (deletes and overwrites, secrets, accounts
  and connectors, what agents may do, restart and the Hollow folder), sensitive `change_setting`
  keys and a session started in a stronger mode
  (`every_confirm_action_is_classified_for_auto` pins the list). A skipped card
  still binds its approval. A card that would show while Hollow's main window
  is gone or closed to the tray (`lifecycle::in_tray`; minimized or hidden with
  ⌘H still counts as open) is refused at once with no card instead of waiting
  two minutes (`Asker::can_show`, S15).
  `tier_audit_tests.rs` checks this end to end
  through the broker's `execute`:
  - every confirm and external action, and every write call with a card, in
    every mode (an unknown spelling asks everything);
  - automations, including an external action named in the automation's
    stamp, which is still refused;
  - a Deny leaves the vault unchanged;
  - the COO's unattended `SAFE` list holds write actions only, whose
    `sessions.start` is capped at `UNATTENDED_START_CAP`;
  - no agent-facing module calls a `USER_ONLY` command directly. Computer-control tier 2 skips its card only in
  bypass.
- **Teammates are capped by their parent** (`team_mcp::delegate_grant`): the
  parent's mode carried from the parent's agent by `agent::inherited_mode`
  (2026-11-13, [PER_AGENT_PERMISSIONS](../PER_AGENT_PERMISSIONS.md): a Codex
  auto parent's Grok runs acceptEdits; a `plan` or unknown parent mode
  refuses, as does a mode the teammate can't run), allowed tools intersected, denied tools accumulated,
  and a deny always wins; never widened. A refusal starts "Hollow didn't
  start". `a_teammate_can_do_nothing_its_parent_cannot` (a Claude parent) and
  `a_teammate_of_a_codex_parent_can_do_nothing_it_cannot` (a Codex parent in
  each sandbox setting, fresh and resumed) compare capabilities, not mode
  names.
- **No Hollow cards for Grok and Antigravity:** neither CLI takes a per-run
  MCP config, so their runs have no confirm cards, question tool or app tools
  (a fenced `hollow-questions` block at the end of a reply still becomes a
  card). The composer and a teammate result say so (`src/hollowCards.ts`,
  `team_mcp::no_cards_note`).
- **Grok's sandbox is the limit, not its mode name:** `build_grok` passes
  `--sandbox read-only` (plan, default), `workspace` (acceptEdits) or `off`
  (auto, bypass), plus `--deny` for the computer-control pair.
- **The wording is generated from the contract** (`mode_contract.rs`, F6,
  owner decision Q1 2026-10-06): every sentence an agent reads about asking,
  cards or preapproval comes from one table per mode and agent capability.
  The adapters append `WORKSPACE_POLICY` plus a permission paragraph
  (`agent::workspace_policy`); the app guidance (`app_mcp::prompt_guidance`:
  every card / the auto classes by label / none in bypass / the automation
  rule / read-only), the brief's computer line and computer control's
  tier-2 descriptions (`ENV_BYPASS`) and the `<hollow-connectors>` note
  follow the mode. Bypass carries no Hollow-internal confirm wording; every
  mode keeps "ask the person before a shared connector sends, publishes,
  deletes or spends anything outside this Mac" (`EXTERNAL_CONFIRM`, in the
  note; an automation does that only when its own prompt asks for exactly
  that, as its boundary says). Only words change: behaviour stays in
  `asks_in_mode`, `server_entry` and `connector_preapproved`, and
  `mode_contract::tests` plus `chat::mode_wording_tests` pin the text to
  them. The generated text per mode is checked in at
  `tests/fixtures/prompts/mode-wording.json` (regenerate with
  `HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib mode_wording`).
  Since HB01 (evening 2026-10-16) a chat says its mode per turn: the
  `<hollow-mode>` line in its message (`mode_contract::turn_line`) carries
  the policy, the app card sentence and the computer tier-2 sentence, and
  the chat's appended prompt and policy only point at it
  (`MODE_POINTER`, `APP_CARDS_POINTER`, `COMPUTER_POINTER`), so a mode flip
  no longer changes the native session's fingerprint. Plan keeps its own
  paragraph; automations and teammates keep the mode in their policy.

### The terminal drawer (parity 7.2)

A person-driven shell in the Preview pane (`terminal.rs`, `TerminalPanel.tsx`).
It is the person's click and typing, never an agent action: the six
`terminal_*` commands are `plumbing` in the command map and appear in no agent
tool table, so an agent can neither open a terminal nor type into one. Its
sandbox, in the order it is applied:

- **Mode gate** (`terminal::gate`). A terminal is not held to a folder the way
  a sandboxed agent's shell is, so it starts only where the chat's agent
  itself could run commands beyond a folder sandbox: `agent::mode_capability`
  with commands and reach at least `Checked`. That is Auto and Bypass for
  every agent (Claude Auto, Codex Approve for me, Grok Auto and Bypass,
  Antigravity Auto and Skip permissions). Refused, with the reason in words:
  Plan everywhere, Claude Default and Accept edits (headless Claude declines
  commands), Codex Read only and Ask for approval (commands held to the
  working folder's sandbox), Grok Default and Accept edits, Antigravity
  Accept edits. An unknown or missing agent reads as the most restrictive
  agent that runs the mode. The window re-asks on every mode, agent or chat
  change (`terminal_gate`, `terminalChatLink.ts`, even while the tab is not
  showing) and a mode that forbids a shell ends that chat's terminals; a
  failed gate call also ends them (fail closed).
- **Folder.** The chat's own working folder (the vault when it has none),
  absolute, canonicalized (symlinks followed) and never a system or shared
  root (`agent::unsafe_working_dir`, the same check agent runs get).
- **Environment** (`terminal::scrubbed_env`): an allowlist, never inherited.
  HOME, USER, LOGNAME, LANG, LANGUAGE, LC_*, TZ, TMPDIR, PATH and
  `__CF_USER_TEXT_ENCODING` from Hollow's own environment, plus TERM
  (`xterm-256color`), COLORTERM, TERM_PROGRAM=Hollow and SHELL. No API key,
  token, `HOLLOW_*` variable, run tag, broker proof or `SSH_AUTH_SOCK`. The
  shell is the person's login shell when `/etc/shells` lists it, else zsh,
  started `-l`, so it reads the person's own rc files: anything they export
  there is theirs, and Hollow adds nothing.
- **Ending.** Each shell leads its own session and process group (the pty
  crate calls `setsid`). Closing one, ending the chat (archive, settle,
  `sessions_api.rs` next to `services::stop_chat`), closing the Terminal tab
  (after a confirm while a shell is running), a forbidding mode and quitting
  Hollow (`stop_children_once`) all send SIGHUP to the group, wait 1.2 s and
  SIGKILL what is left. A crash closes each pty's master, which hangs up the
  shells; a job that left its group with `setsid` survives that, as it does a
  terminal app's own close. At most 4 shells per chat and 12 in all.
- **Screen.** Hollow's own VT emulator (`terminalScreen.ts`, no dependency).
  OSC 52 clipboard writes, OSC 8 links and mouse reporting are read and
  dropped; only the window title is honoured. Pasted text loses escape and
  control characters (`pasteBytes`), so a paste cannot close the bracketed
  paste or drive the terminal. Cmd chords are never taken by the terminal
  (`keyToBytes` returns null), so it binds no chord of its own; Esc and Tab go to
  the shell, Shift+Tab is left alone so keyboard focus can always leave, and the
  pane's close is its own button.
- **Not built (v1):** remote machines, a terminal for a chat on a peer or the
  runner, persisted scrollback (a terminal lives until closed or Hollow quits),
  and mouse support for full-screen programs.
- **Crate:** `portable-pty` 0.9 (MIT, wezterm's pty layer; no network or
  telemetry code). New to the lock with it: `downcast-rs`, `filedescriptor`,
  `serial2`, `shell-words`, `shared_library` (Windows only), `cfg_aliases` and
  `nix` 0.28, all MIT or Apache-2.0.

### Task tools (hollow_tasks, Paperclip PR 12)

Claude and Codex chats and automations get Hollow's task list as tools
(`tasks_mcp.rs`, this binary re-invoked as `hollow tasks-mcp`; modeled on
Paperclip's native-runner semantic action catalog). Antigravity has no owned
MCP and gets none.

Reads return `contentHash`; writes (`create_task` needs an `idempotencyKey`,
`update_task`, `set_status`, `claim_task`, `release_task`, `add_comment`) cross to the GUI's `task_broker`
with a proof and run inside `automated_history::with_live_run`, so each is in
the run's History, or refused once the run has finished. From a workspace's
conversation (a scoped run) each write takes the full-vault lease in its name
for the call, as before, but snapshots only `tasks/`, `projects/` and a
project op's own note on both sides (`with_live_run_touching`, `Touches`;
chat concurrency F7, 2026-10-20), not the whole vault twice: about 26 ms
against 480 ms on a 20,000-file vault in a debug build. A write whose folders
hold a link (it could write through it), and every Hollow app action, still snapshot the whole vault; a new broker write that names its folders must list every one it can
change. A file its logged vault writes touch elsewhere is still recorded, with
no earlier version (so not restorable); a delete or a plain `fs::` write
elsewhere is not. Plan mode and
tool-restricted automations without a write grant get a read-only server.
`propose_tasks` only queues `agent_task` review items. Claims (S09):
`claim_task {ref, expectedHash?}` moves an open, unclaimed task (not done,
cancelled, on hold or already in progress) to `In progress` and writes
`claimed_by: <surface>:<scope>` (`chat:<session key>`, `workflow:<slug>`; the
identity the broker's proof vouches for) and `claimed_at:`. The server fills
`expectedHash` from its own read, so of two claims from one read exactly one
lands and the other is `TASK_CONFLICT`; a held task is `TASK_CLAIMED` naming
the owner, and a task whose status its tracker issue owns is `TASK_LINKED`. Both tools
answer `disposition: "applied"` with `already` (true for the owner's repeat
claim, which changes nothing; S24 replaced its old `duplicate`).
`release_task {ref, expectedHash?}` is owner-only and returns `In progress` to
`Not started`. A claim counts only while the task is in progress: any status
write that leaves it (finish, recurring roll-forward, reopen), or that moves a
task carrying a leftover key into it, drops `claimed_by`/`claimed_at`
(`vault_tasks::status_change_ends_claim`, in `patch_task`). A leftover key on another status is stale everywhere
(`live_claim_of`): `claim_task` overwrites it, `release_task` says the task
isn't claimed, and `get_task` reports `claimedBy: null`. `src/workflowPreview.ts`
`TASK_WRITE_TOOLS` mirrors `tasks_mcp::WRITE_TOOLS`; a Rust test holds them equal.
`next_claimable {project?}` (a read) returns `{task}` (or `null`): ready, open,
unclaimed, highest priority then oldest `created`. The keys are local only
(not modelled fields; the tracker maps neither). A patch that changes the status drops
them unless both the old and new status are in progress. A field a task's
tracker issue holds (`heldFields` in `get_task`) is `TASK_LINKED` unless sent as
`update_task`'s tracker fields, which queue a durable edit for the next tracker
pass ([Tracker sync](../TRACKER_SYNC.md)). Since Notion two-way sync was removed
(2026-11-10) a task or project with only legacy `notion_id` / `notion_url` /
`notion_project_ids` keys is a plain vault file and takes the plain write path;
`list_projects` lost its `notionLinked` filter and `get_task`/`get_project`
their `notion` view. `update_task {projectUrl}` moves a task between projects.

Task-project records above are separate from the sidebar's local projects.
Use `hollow_app list_workspaces` / `read_workspace` / `create_workspace` for
ordinary Hollow project requests. A missing legacy `projects/` directory is
not proof of tracker ownership. `create_project` preserves its legacy record
contract and points to local workspace creation when that directory is absent.

### OC5 lane rows

Attempt-bound completion, a circuit breaker and a stale-base refusal
(2026-11-17, from Orca's dispatch rules). Core only so far: the
`hollow_tasks` tools don't pass attempt ids yet (see the end).

- **Attempt ids.** Every `claim_task` counts up `claim_attempts:` in the task
  file and returns that number as the claim's `attempt`; the key outlives the
  claim, so a later claim always gets a new id. A `claim_attempts:` that isn't
  a number refuses the claim (`TASK_CLAIMED`, fix the key) rather than reuse
  an id. `vault_tasks::complete_claim(owner, attempt, status)` writes the
  status (recurrence as `set_status`) only while `owner` holds the live claim
  on that attempt; anything else is `TASK_STALE_ATTEMPT: …` and writes
  nothing, so a replaced run's late retry can't close a newer claim. A
  terminal status (a recurring roll-forward included) also clears
  `failed_attempts:` and `last_failure:`, so failures never carry past a
  success. It writes through `set_status_finishing`, which refuses a
  tracker-linked task (`TASK_LINKED`): a tracker task's status belongs to its
  provider, so completing one with an attempt check is not supported yet;
  `fail_claim` does work on one (it writes only Hollow's own keys).
- **Circuit breaker.** `vault_tasks::fail_claim(owner, attempt, reason)`
  checks the attempt the same way, ends the claim, counts `failed_attempts:`
  and keeps `last_failure:`; failures below the limit carry over from claim
  to claim. At `CIRCUIT_BREAK_AFTER` (3) it writes `circuit_broken: "3 failed
  attempts; the last: <reason>"` and `circuit_status:` (the status it left
  the task in), and a local task goes `On hold`. The circuit is live only
  while the task's status still equals `circuit_status`: then
  `next_claimable` skips it (local and tracker branches) and `claim_task`
  refuses with `TASK_CIRCUIT_BROKEN: … stopped retrying: <reason>`. Reset:
  any status change of a local task (`patch_task`) drops the breaker keys
  and the failure count, so a later hold for another reason never cites the
  old one; a tracker task's provider status moving away from
  `circuit_status` makes the keys stale, and the next claim clears them.
- **Stale base.** `build_workspaces::create` refuses a base that is
  `STALE_BASE_LIMIT` (20) or more commits behind main (origin's default
  branch as last fetched, else `origin/main`, `origin/master`, `main`,
  `master`) with `STALE_BASE: <base> is N commits behind <main> … merge <main>
  into <base> first, or pass allowStaleBase`. `workspaces.create_build_workspace`
  takes `allowStaleBase` and `staleBaseLimit` (0 turns the check off). An
  explicit base is not fetched first. The friction loop's fix drafts
  (`create_build_workspace_with`) skip the check: they can't send the override. The 20 is Orca's number; Hollow has no
  data of its own on it yet.
- **Not wired yet (lane M's files):** `tasks_mcp.rs` and `task_broker.rs`
  should return `attempt` from `claim_task`, accept `attempt` on `set_status`
  (routing to `complete_claim`) and add a `fail_task {ref, attempt, reason}`
  tool calling `fail_claim`.

### Hollow app tools (hollow_app, PRs A1-A6)

`run_action sessions.start_workers {workspace, count, tasks}` creates an
explicit local batch from a global or project main conversation. Each task
has a distinct title, prompt and optional task packet; count must match the
list (1–25, and at most the COO's swarm limit in Settings), and the
run must have room to record every worker as its child (32 delegations per
run). Every entry is validated before queueing. Provider, model and
permissions come from the coordinator. Existing chat/build/resource holds
still apply, so queued and running are different states. A batch returns each
accepted chat and any per-task failure; identical retries reuse those results,
and a changed second batch in the same run is refused. Child and unattended
runs cannot use this path. Ordinary `start_session` is bounded the same way since L1: at most six of
one conversation's subagents run at once, the rest queued, with a 25-start
runaway guard per turn. Parent stopping, result reads and next-turn completion notices use
the existing durable-child machinery; no new automatic project wake is added.
A turn stopped while its batch waits, or partway through it, queues nothing more.

`hollow app-mcp` exposes 21 first-class tools (`app_mcp::FIRST_CLASS`: list,
read, create and update workspaces; list, read, wait for, start, send to, stop
and manage sessions; automations; settings; build status, commit and publish;
`navigate`) plus `list_actions` / `describe_action` / `run_action` over
`app_actions::ACTIONS`, whose handlers call the Tauri commands themselves.
`list_actions {query?, area?, tier?, limit?, cursor?}` (F9, wave 4 · V2)
lists only what the run may use (a read-only run gets reads; at
`MAX_START_DEPTH` the session starters are left out), best query match
first (its words against the id, the first-class tool name and the
summary), 25 a page by default and 100 at most with a `nextCursor`, each
row an id, tier, optional `tool` and one summary line of at most 140
characters; `describe_action` keeps the full summary and schema.
Each row with arguments also has `args` (overnight 9 A7,
`app_actions::signature`): `workspace*, keepDays:int, dryRun:bool`, a
star on required ones, non-string kinds named, a choice's values joined
by `|` up to 40 bytes (then `|…`), cut at a whole argument to 120 bytes
with "…". A missing
required argument's error lists the same signature, so one retry can fix
every argument. Codex had made 40 describes for 38 runs in 16 turns.
Six of them (`app_mcp::ALWAYS_LOAD`: run_action, list_sessions,
read_session, describe_action, start_session, send_to_session, whichever
the run lists) carry `_meta` `anthropic/alwaysLoad`, so Claude Code loads
them with the session instead of behind ToolSearch (HB05, evening
2026-10-16; about 7.4 KB of schema for a writing run, 2.3 KB read-only;
OB21's arguments below added 415 bytes to both).
The server-level flag stays unset: it holds startup. Other clients should
ignore `_meta` (MCP spec; not checked against Codex or Grok).
`list_sessions` (`sessions.overview`, OB21 2026-10-17) answers 12 short rows
by default (key, title, agent, status: the newest turn's run-queue status,
else the store's in the queue's terms, so `done`/`failed` rather than
`success`/`error`; lastActivity, running, parentKey only for a fork);
`limit` up to 200, ignored with `mine`. `mine: true` lists only the caller's own child tasks (its run's,
and in a chat its conversation's of the last 90 days), each once with its
newest turn, newest first, each followed by its handover successors
(`handoverFrom`): key, title, workspace, agent, status, startedAt, and once
ended finishedAt, costUsd when known, error and OB17's summary (at most 400
answer characters), at most 30 rows (`more: true` when there are others);
no run queue; redacted (`descendants::mine_overview`). A caller that handed
over sees its own successor there too. `detail: true` is the old full answer
(full rows, live turns, run queue; `limit` default 30). `read_session`
(`sessions.conversation`) takes `answerOnly` (the answers without their
prompts; tools default to `none`), and when the session is an ended child
of the caller's (`descendants::is_ended_child`: the conversation's newest
turn is one the caller caused, directly or down a handover chain, and has
ended; a later turn it didn't cause rules it out) `lastTurns`
defaults to 1 and `tools` to `none`; an explicit argument always wins.
Repo features R4: `query` searches the stored conversation instead
(`search_conversation`: the request without Hollow's injected blocks, the
answer, each tool's input and result, each redacted before matching;
`retrieval_terms`' words, ranked by distinct terms, then hits, ties to the
newest) and returns at most 12 hits `{turn, in, tool?, toolId?, snippet}`
(240 characters around the first match), `matchedTurns`, `totalTurns` and
`notFound` terms. `fromTurn`/`toTurn` (`slice_turns`: turn numbers are the
store's `ord`, from 1; at most 20 turns) return those turns in full, tools
cut per `tools` (default `excerpt`), or a short note with `totalTurns` when
`fromTurn` is past the end; with `query`, only they are searched. A query is
at most 200 characters, and redaction placeholders (`[email]`) never match.
Either skips the ended-child default. No handover-chain walk, no FTS5: a
linear scan, under 50 ms over 300 turns in a debug build.
Every call crosses to the GUI broker (`app_broker::authorize` checks the
switch, surface, proof, id, tier and arguments before anything runs) and is
audited in `{vault}/.app-actions.jsonl`. Writes run only while the calling run
is live; confirm and external actions show cards per the mode rules above.
After a write the broker emits `app-changed {action, area}`; the webview
(`appBridge.ts`, its plan `appBridgeCore.planForChanged`) refreshes the
workspaces, the session lists and custom themes, and for any
`settings.extraction` action, `learning.set_learning_suggestions` or
`settings.change` bumps `store/settingsRevision.ts`, so an open Settings
reads Learning suggestions, the extraction model and Retitle new chats again
(overnight H6).
`docs/HOLLOW_APP_COMMAND_MAP.tsv` classifies every Tauri command, plus every
action with no command of its own (verbs and composites, keyed by their dotted
id, file `app_actions.rs`; overnight D3). Tests in `app_actions.rs` keep it,
`lib.rs` and the registry in step. The first, `registry_covers_every_command…`,
checks coverage, area and tier. The second, `the_map_is_well_formed…`, checks
the header, exactly five columns, no duplicate key, that each file exists, that a
command's file matches its `generate_handler!` path, and that a verb row matches
its action's area and tier. Add a verb's row with the verb. **Only a person resolves what
agents propose:** review items, commitment, learning and heartbeat-task
proposals and the About-you builder are `user-only` rows, never actions
(`app_actions::PERSON_RESOLVES`), in every mode. That includes bulk learning review
(`resolve_learning_proposals_batch`, `undo_learning_batch`). `start_session` and
`send_to_session` are `app_sessions::start` / `send`: an ordinary queued chat
turn one level deeper (the task packet, when it carries anything, then the
task text; a send resumes the conversation's native session), whose body the
context compiler completes like any chat's, so a new child gets the operator
profile once on its first turn (F4). They are the durable child task (see
Agent teamwork); the packet is an injected block, so the child's request,
title and retrieval use the task text alone. One Main conversation
(2026-11-07): `start_session` refuses the COO's workspace
(`app_sessions::MAIN_START_REFUSED`), and a `send_to_session` to a chat that
continued (a handover, an earlier COO conversation) goes to the
chain's current one, its answer naming `forwardedFrom` (refused when that is
the caller itself; [COO](../COO.md#one-conversation-2026-11-07)). `sessions.handover`
(`app_sessions::handover`, overnight C2) is the one start allowed at any
depth (depth 2 can't start or send): a successor chat in the caller's own
workspace at the caller's depth (at least 1) and mode, once per
conversation ever (strictly linear chains), at most `MAX_HANDOVER_CHAIN` (24,
overnight 9 A2) per chain (generations and predecessors in `<support>/Handovers/<vault>.json`;
the record is written under the chat gate just before the successor's first row,
so a failed write queues nothing, and an unreadable file is never rewritten from
empty: `app_sessions::write_pending_handover`; startup recovery drops a record whose
successor has no conversation or queue row, `reconcile_handovers`),
and not one of the turn's starts (`app_sessions::take_start`);
the caller names the successor in its last reply and ends its turn. When to
hand over is a number: its description says to check `sessions.context` (OB11,
read-only: the caller's own live main-thread `contextTokens`, the window and,
on a 1M window, a 350K advisory; see [Usage instruments](../USAGE_INSTRUMENTS.md)).
`sessions.task_state` (repo features R1, read-only, not first-class) returns the
caller's own conversation's `<task-state>` record with the running turn folded
in (`task_state::peek`) and no continued conversation's items merged (a fresh
session's block merges them): for an agent whose context compacted mid-turn;
an automation caller gets `available: false`
(see [Agent context](../AGENT_CONTEXT.md), layer 11). Whoever
hears of the caller's ending learns the successor's key from Hollow too
(`descendants::handed_over`, read per turn from that turn's `handover`
record: `, handed over to <key>`, or `, its successor <key> was withdrawn`
after a stop, on the parent's `<hollow-child-finished>` line, chain lines and
the COO's report wake; `successor`/`successorWithdrawn` in
`wait_for_session`'s answer; morning M3). Stop cascades: the successor is
recorded as the caller's child task (tool `handover`), so stopping the
caller's turn before it ends stops it (`chat::stop_child_turns`). Not for
unattended turns (`unattended::SAFE`) or automations. `sessions.wake_me
{minutes, reason}` (`wake_me.rs`, overnight C3) queues a continue turn of
the calling chat 1–120 minutes later instead of a turn that waits: one
pending per conversation (asking again replaces it), ≤24 in a rolling day,
chats only, never a build workspace (checked again when due: a merge or
move into one drops it), never after the chat handed over, refused where the
timer doesn't run (candidates). The woken turn resumes the conversation unattended (the
COO's safe actions only; it can't wake again) at the asking turn's
depth, in the conversation's current mode and never above the asking turn's
(`wake_me::woken_mode`; capped at `acceptEdits` until 2026-11-12), asking for
the model the conversation's newest turn ran with when it ran the same agent
(the CLI default stays the CLI default; `wake_me::wake_model`, morning M2)
(busy, or tonight's spend unreadable: next minute; errors retry; 30 min late,
or due past tonight's overnight stop: missed. Lateness counts from the last
minute a busy conversation held it (`busyAt`, PER-12, 2026-11-14), so a wake
due during a long turn runs once that turn ends; a `queueing` entry no send in
this process holds, left by a quit or crash mid-send, is recovered at the next
claim from the conversation's turns requested since `claimedAt` (else `at`):
its woken turn found, queued; another turn the person's (or with its text
gone), cancelled; only Hollow's own or none, pending). Its cost and its child tasks'
count in the overnight scope (`wake_me::woken_runs` in
`budgets::unattended_spend`), and when the asking turn was a durable child
task the woken turn joins that delegation (a `wake` record for the
delegating run, `wake_me::link_to_delegator`), so the parent and every
conversation up the chain hear when it ends (morning M1). A stopped asking
turn, the person's next message or a handover cancels it; the person's
message spares one already due or held by a turn
(`wake_me::cancel_session_on_send`, PER-12), which runs after the turn. The asking turn's receipt
shows it (`scheduledWake`); `sessions.wakes` lists, `sessions.cancel_wake`
cancels the conversation's own. Registry `<support>/Wakes/<vault>.json`;
the COO's minute timer (queue owner only) runs due ones. The Main
agent's own prompt (`chat::main_agent_prompt`, overnight E7) tells it to end
its turn after starting a chat (it is woken when the chat ends) and to use
`sessions.wake_me` for a later look rather than a wait loop; unattended turns
can't. The prompt is held to `MAIN_AGENT_PROMPT_BUDGET` (4,600 bytes; it is
~4.6 KB) by `main_agent_prompt_tests`; as the dispatcher (M3a) its worker bullet
is swapped for one that routes long work with `sessions.route_to_runner`. `sessions.wake_me {onSessionsEnd:
{keys, mode?}, reason?}` (overnight OB10b, `wake_me::schedule_on_end`; not
with `minutes`) wakes the calling chat when conversations it started end
instead: `all` (default) once every key ended, `any` at the first; 1–12
keys, each one the caller started (`list_sessions` `mine`, successors
included; anything else is refused), a handover chain ending at its last
successor; refused when they already ended, and in a COO thread (woken
by its own mechanism). The chat-end hook (`main_agent::after_chat_finished`
→ `wake_me::arm_ended`, and the minute timer for a child cancelled while
queued or interrupted at startup) marks it due once and sends it; it waits at
most 24 h, and a fired wake whose parent stays busy over 30 minutes is
dropped as missed (the children then reach it through its next turn's note).
`minutes` and `onSessionsEnd` both missing is refused naming both. The woken
turn is the same unattended wake as above (same caps and
cancels) and its message carries each ended child's line and OB17 summary,
recorded as reported just before the send so no note repeats them; a send
that fails or never settles forgets them again (`session_store::undeliver`),
so a wake that then never runs leaves them to the next note.
`wait_for_session`'s "still running" advice names it (not in a COO
thread, which keeps OB10a's wording). See [COO](../COO.md).
`wait_for_session` (a read,
`sessions.wait`, F10 D2) waits up to 300 s for a session's turn to end; the
server waits 315 s for the GUI's answer to it (`app_mcp::broker_wait_secs`).

### Read result envelope (OB1 and TC1, 2026-11-17)

`mcp_envelope.rs` holds the read results of `list_sessions`, `read_session`
(`hollow_app`), `list_tasks`, `list_projects` (`hollow_tasks`) and
`search_vault` (`hollow_vault`) to a text budget: 24576 bytes by default,
`maxBytes` up to 80000 (at least 4096). A result that fits is its old text,
byte for byte. One that doesn't keeps whole items while they fit, lists the
next ones headers only (`headerOnly: true`; the body is the tool's own reader:
`read_session` with `fromTurn`/`toTurn` for a message, `read_source` for a
note or passage) and returns `nextCursor`, which resumes at the first item not
yet returned whole; a first item over the budget on its own still goes whole
(`item_over_budget`). `read_session` pages from its newest message back.
`list_sessions` asks the broker for one row past `limit` to tell a real
`row_limit` from a list of exactly `limit`.

Each enveloped result ends with one more text block,
`{"envelope":{"budgeted":…,"capReasons":[…],"complete":…}}`, plus `maxBytes`,
`items` (`from`, `returned`, `headersOnly`, `total`), `nextCursor` and `hint`
when cut. `capReasons`: `byte_budget`, `row_limit` (the tool's own `limit` or
turn window left rows out), `item_over_budget`. A cursor is
`c1.<offset>.<hash>`, the hash a SHA-256 of the tool name and its other
arguments with keys sorted; a cursor from other arguments is refused. Both
arguments are taken out before the app broker's argument check, so
`run_action` of the same ids is not enveloped. Near misses (`max_bytes`,
`next_cursor`) are refused with the right name.

Tests: `mcp_envelope::tests::tool_list_bytes_stay_under_their_caps` caps each
owned server's `tools/list` bytes per access level (raise a cap in the same
change, with the reason); `contract_is_byte_stable_across_processes` (TC1)
runs the test binary twice with `HOLLOW_TC1_PRINT=1` and requires the same
tool lists and guidance bytes, and a planted `HashMap` (`HOLLOW_TC1_PLANT=1`)
must make them differ.

### Refusal codes and request ids (BI5 and OC4, 2026-11-17)

Every `isError` result of `hollow_app`, `hollow_tasks`, `hollow_vault` and
`hollow_code_context` (CC2) keeps
its readable sentence as the first text block, as before, and ends with one more,
`{"error":{"code":"UNKNOWN_ACTION","didYouMean":["sessions.start"],"next":"list_actions"}}`
(keys sorted; `didYouMean` only when the sentence suggested names, at most three).
`mcp_envelope::classify` maps the sentence to a code of the closed set
`REFUSAL_CODES`: a leading `CODE:` of the set first (the task broker's prefixes),
then phrase rules, then the fallback. `next` is the call to make: a tool name, an
app action id (through `run_action`), `tools/list`, `retry` (the same tool again,
changed as the sentence says, or unchanged later) or `reply` (no call fixes it).
Nothing about what is refused, tiers, cards or access changed.

| Code | Means | Next (app, tasks, vault) |
|---|---|---|
| `UNKNOWN_ACTION` | no action has this id | `list_actions`, `tools/list`, `tools/list` |
| `UNKNOWN_TOOL` | no tool by this name | `list_actions`, `tools/list`, `tools/list` |
| `INVALID_ARGUMENT` | an argument is missing, unknown or the wrong kind | `describe_action`, `retry`, `retry` |
| `NOT_PERMITTED` | tier, access, depth, an ended run or a card's answer | `reply` (a depth refusal: `sessions.handover`) |
| `NOT_FOUND` | no such session, workspace, task, file or skill | `retry`, `list_tasks`, `search_vault` (or the list the sentence names) |
| `CONFLICT` | changed since it was read | `retry`, `get_task`, `retry` |
| `CURSOR_INVALID` | a cursor from another query, or past the end | `retry` |
| `UNAVAILABLE` | Hollow, a broker or the vault did not answer | `retry` |
| `INTERNAL` | an unreadable answer from Hollow | `retry` |
| `REQUEST_ID_REUSED` | this requestId was used for a different call | `sessions.request_status` |
| `REQUEST_PENDING` | this requestId's call started and has not finished | `sessions.request_status` |
| `TASK_CONFLICT`, `TASK_INVALID`, `TASK_LINKED`, `TASK_CYCLE`, `TASK_BLOCKER`, `TASK_KEY`, `IDEMPOTENCY_MISMATCH`, `TASK_CLAIMED` | the task broker's own codes, unchanged | `get_task`, `retry`, `get_task`, `get_task`, `list_tasks`, `retry`, `retry`, `next_claimable` |
| `REFUSED` | the fallback: the sentence says why | `reply` |

A one-value Next is the same on every server. `hollow_code_context` (CC2)
shares the vault's column except `NOT_FOUND` (`symbols`) and its own two
phrases: "no definition named" (`NOT_FOUND`) and "over the impact cap"
(`INVALID_ARGUMENT`, next `callers`), since CC4 "this handle"
(`CURSOR_INVALID`) and "over the slice cap" (`INVALID_ARGUMENT`), and since
CC5 "over the changes cap" (`INVALID_ARGUMENT`);
[CODE_CONTEXT](../CODE_CONTEXT.md).

**Request ids.** `start_session` and `send_to_session` (`sessions.start`,
`sessions.send`) take an optional `requestId`, 8 to 64 of `A-Z a-z 0-9 _ -`.
Before the call runs, a pending receipt is written for the caller (surface and
scope: the chat session or the automation, so a retry from a later turn of the
same chat counts) in `<vault>/.hollow/session-requests.json`; a result completes
it and a refusal removes it. A repeat with the same arguments (key order does
not matter) returns the stored result with `replayed: true`, starts or sends
nothing and shows no card again; a repeat while pending is `REQUEST_PENDING`; the
same id with other arguments or the other action is `REQUEST_ID_REUSED`.
Receipts are kept 7 days and at most 500 (pruned on every write, oldest
completed first; a pending one is never evicted, and all 500 pending refuses
a new call `UNAVAILABLE`, "receipts are full"), written 0600, and survive a
restart. A store file that doesn't parse is left as it is and calls with a
`requestId` are refused `UNAVAILABLE` naming the file to fix or remove;
`sessions.request_status` then answers `unknown`. A refused call removes its
receipt, so a retry runs again: a start whose turn failed to queue after its
tracker issue was prepared leaves that issue, and the retry may create a second. `sessions.request_status` (read) answers `completed` with the result,
`pending` (also after Hollow quit mid-call) or `absent`, and says that absent is
not proof nothing happened. A call without `requestId` runs exactly as before
(`app_sessions::requests`).

Tests: `mcp_envelope::tests::every_refusal_resolves_to_a_code_of_the_closed_set`
(a table of each server's refusal texts with their code and next),
`the_refusal_table_is_closed_and_documented` (every code is in this file),
`app_mcp::tests::refusals_carry_a_code_and_the_next_call`,
`app_sessions::requests::tests`.

### Stability levels (UN3, 2026-11-16)

Every action id has a level, shown by `describe_action` as `stability` (and, when
it is not stable, `stabilityNote`, the reason) and in the map's sixth column,
`stability`, in `docs/HOLLOW_APP_COMMAND_MAP.tsv`: `stable` (safe to build on; a
change is announced and the old id keeps working for a notice period),
`experimental` (may change without notice), `internal` (Hollow's own screens and
tools, not for a skill or an agent's own workflow) or `deprecated` (still works,
has a replacement named in the note, and will go). The default is `stable`; the
exceptions are `app_actions::NOT_STABLE`, sorted by id, each with its reason (for
example `ui.read_tab`: its numbered outline landed 2026-11-17 (CD2) and it stays experimental until real chats have used it; `workspaces.project_card` (PC1, 11-18), whose shape grows with the card's facets; and CX23's two folder-abstract actions, experimental while off by default). To mark
an action, add its line to `NOT_STABLE` and set the same level on its row (its
command's row, or its own id's row when it has no command): a test
(`every_action_has_a_stability_level_that_the_map_and_describe_show`) fails when
they differ, names an unknown id, lists a stable one, or leaves a deprecated one
without "use <id>" in its reason. The level changes nothing about how an action
runs or who may run it. `list_actions` rows show `stability` only when it is not
`stable`, and the page then carries `notStable` (how many matching actions are
not stable), so an all-stable page reads as before (C11, 2026-11-17).

### COO parity (L13, 2026-11-10)

The COO can do anything the person can do in Hollow's window. Every command
still `user-only` in the command map is in `app_actions::COO_COVERED` (an
existing action does the same thing, named there) or `COO_EXCEPTIONS` (why it
stays the person's), and
`coo_parity_every_user_only_command_has_an_action_or_an_exception` fails a new
user-only command until it is one of the two or gets an action. The
exceptions are the person-only gates (accepting a Result, approving a learning
or a proposal, answering a card), macOS prompts the person answers, the
window's own state, and the context sync's remote, off switch and Sync now
(CX6a; agents read `settings.context_sync_status`) and the vault root's on or off
(`configure_vault_sync`, CX6b) and the other Mac's vault path for the
on-demand binary fetch (`configure_file_copy`, V4; agents read
`settings.file_copy_status`, list with `vault.peer_files` and fetch one with
`vault.fetch_peer_file`, which never overwrites). The CFO's policy (L7) is the

(`configure_vault_sync`, CX6b) and the choices on a context sync clash card
(`resolve_sync_clash`, CX6c; agents read `settings.sync_clashes`, paths only). The CFO's policy (L7) is the
COO's through `settings.cfo_settings` and `settings.cfo_view` (reads) and
`settings.cfo_save_settings` and `settings.cfo_approve_extra_usage` (confirm,
asked in `auto`, old to new on the card; the approval is always-ask, so its
card shows in Bypass too). L13 added 117 actions
(self-hosted models, companion and iPhone pairing, the remote runner, tracker
keys, interrupted chats, conflict copies, steering a chat the COO started,
icons, settings files, quit, limits and more); secrets go in only as
`Kind::Secret` and no result returns one. A UI setting the COO changes while
the window is closed is queued in `pending-ui-settings.json` (app data) and
applied when the window next opens (`take_pending_ui_settings`).

### Publish to Main (updates U8, 2026-11-15)

`build.publish_status` (Read) shows what Settings, Updates, Publish to Main
shows: the live Nightly and Main, the kept Nightlies, the proposed build and
what it brings since Main, read from the release builder through the runner.
`build.publish_main` (External, command `update_publish_main`) is on
`app_actions::ALWAYS_ASK`: an agent may propose publishing a named build, and
its card (the build, its commit, the live Main, the PR titles since Main;
destination the update host's `channels/main.json`) asks in every mode,
Bypass included; automations and COO check-ins never run it. Only the
person's Allow publishes, bound to the card's commit and the Main it showed
(a build older than the live Main gets no card). Withdraw
(`update_withdraw_main`) and "This Mac publishes to Main"
(`update_set_publisher`) are user-only. Details: [Updates](../UPDATES.md#publish-to-main-u8).

### Listening ports and orphans (OC14, 2026-11-17)

`build.list_services` with `ports: true` answers `{ "services": [...],
"listeningPorts": {...} }` instead of the bare services array (without `ports`
the answer is byte-for-byte the old array; pinned by
`port_owners::tests::list_services_keeps_its_array_shape_unless_ports_are_asked_for`).
`src-tauri/src/port_owners.rs` reads `lsof -nP -iTCP -sTCP:LISTEN -FpgRcn`, then
the working folder of each pid (`lsof -a -p <pids> -d cwd -Fpn`), and keeps the
listeners whose folder is inside a build worktree (a bound workspace's, or an
unbound `<worktrees root>/<repo>/<name>` folder). The rest are only counted
(`unattributed`), never described. Read-only; a listener has `owner`:

- `hollow-service`: its pid or process group is a `start_service` one;
- `live-turn`: a live scoped run of this Hollow is in that workspace
  (`automated_history::scoped_run_ids` + `live_scope`; scopes are
  vault-relative `workspaces/<id>`, so it matches the bound workspace's id, not
  the worktree path);
- `orphan`: bound worktree, this Hollow has no live run in it, no other Hollow
  holds the full-vault lease, and the process's parent is launchd (pid 1). That
  is evidence the starter is gone, not proof: another Hollow instance's scoped
  turns need no lease and are not visible, so an orphan is a candidate a person
  confirms from the pid, command and cwd shown;
- `unknown-owner`: everything else, never guessed: an unbound worktree, a parent
  that is still alive (it may be your terminal), another Hollow holding the
  full-vault lease, or a parent that couldn't be read.

**Stopping.** `port_owners::check_stop_target` and `stop_orphan` signal ONE PID
(SIGTERM, no group, no name match) and only one the listing re-read at that
moment names as an `orphan` on that port, then look at that pid once more
right before the signal (still listening, parent still pid 1, same cwd, same
start time as listed), so a reused pid or a changed owner refuses. The contract
carries pid AND start time: a listener's `startMicros` is in the listing, and
`stop_orphan(vault, root, pid, port, expected_start_micros)` refuses when the
process started at another time than the card showed. Two bindings at one path
(a removed workspace's leaf name reused) prefer the existing folder and carry the
other ids, so a live turn in either is `live-turn`; a working folder over 64
components deep is not attributed; any live run holding the full-vault lease (this Hollow's own turn, project chat or overnight run, or another's; `lease_holder`) reads every in-worktree listener `unknown-owner`, never orphan; a live turn in ANY enclosing binding (outer and nested) counts; the stop path needs `confirmed_by_person: true` (a person saw pid, command, port, parent, folder) and re-reads the listing right before the signal; the removed-worktree filter compares canonical paths; a failed `lsof` (no status, exit above 1, or
exit 1 with only an error message) reads as unavailable, never as no ports. They are **not wired to any action**: an agent-callable stop needs its
own confirm-tier action id (a card showing pid, command, port and worktree) and a
`stability` row in `docs/HOLLOW_APP_COMMAND_MAP.tsv`, which the OC14 change did not
add. `build.stop_service` is by name and cwd and cannot stop a process Hollow
did not start. Not seen: runs of another Hollow instance, queued turns that
have not started, and processes of other users (lsof shows only this user's).

### Question cards (Paperclip PR 13)

An agent blocked on a decision only the user can make asks through a card, not
prose. `AskUserQuestion` stays in `HOST_ONLY_TOOLS` (the managed build's host
tool has no channel here); Hollow's own tool replaces it.

The card comes from `hollow_questions` → `ask_user_questions`
(`question_mcp.rs`, stateless; schema and limits in
`interaction_schema.rs`), attached to every chat turn of an owned-MCP agent and
never to workflows, the heartbeat or delegates. The GUI records cards in the
`interactions` table; answers are never lost, and a later turn supersedes
open cards. Antigravity and Grok ask with a fenced block at the end of a
reply.

**Docked, one at a time (2026-10-14).** Cards no longer sit in the
transcript: `components/QuestionDock.tsx` takes the message box's place on
the chat page (the COO's included) and steps through a `questions`
card one question at a time ("1 of 3"; logic in `src/questionDock.ts`).
Choosing a single answer goes on; the last sends the answers straight to the
agent (the same `resolve_interaction` turn, "Answered questions", then each
question on its own line and the answer on the next; a custom answer is the
person's words, with no Other: prefix), which is also the transcript's record.
A question's `reason`, or its help text collapsed to one line, shows under
the question. **Decision cards (2026-11-17):** a question may also carry a
`scenario` (one concrete case with numbers, 600 characters at most) and each
option an `outcome` (what choosing it does in that same case, 240 at most);
both optional, whitespace collapsed to one line, over the cap refused like
`reason`, and plain text only. The card shows the scenario as a muted
paragraph before the options and each outcome as a second muted line under
its label (the outcome is the option's `aria-describedby`; the scenario
describes the list). The tool schema's field descriptions ask for them on a
decision (one plain question as the prompt, the one reason for the
recommendation in that option's `description`); the shared guidance texts
stay as they were, because that never-cut layer is under a hard byte budget
(`chat_g2_budget_tests`), and the fenced Antigravity path accepts the fields
but does not advertise them; the answer turn is unchanged and never repeats
them. Checkbox options (an `outcome` there is dropped without a refusal) and the phone's question wire don't carry them. The
selected row and the Recommended tag use the slate accent,
and the round control sits inside the card's left edge. The text written
just before the card stays outside the "Worked for" fold. Back and the trail of earlier answers reopen a
step; `allowCustom` now defaults to true, so a choice question has an
"Other…" row unless the agent turns it off. The cursor starts on the
recommended option. Keys are plain (↑↓, 1–9, Space, Enter, ← for Back, Esc
to hide; nothing with ⌘/⌃/⌥, so no macOS overlap), each with a button. In
the half second after the dock shows, keys that choose or send (Enter in a
text or "Other…" field too, since 2026-10-15) do nothing (`tooSoon`). Esc
hides the dock behind a "N questions waiting" chip over the message box. A
confirmation, checkbox or verdicts card is one step, drawn whole by
`QuestionCard` inside the dock.

Archive: agent integration, importers, tools, permission modes, accounts and connectors
(including the Antigravity probe matrix and the full tool contracts).

---

## workspaces.clone_repository (4.4)

Confirm tier, outside lease. Clones an `https://`, `ssh://` or `git@host:owner/repo` address into the Projects
folder (`url`, optional `name`), so `workspaces.create_build_workspace` can use it. The address is checked before git
runs (no `file:`, `ext::`, plain `http:`, options, whitespace or credentials), git runs with
`GIT_ALLOW_PROTOCOL=https:ssh` and no prompt, a taken folder name is refused, and a failed or ten-minute clone is
removed. Window commands `quick_chat_scratch` and `adopt_chat_scratch` (a chat's scratch folder and Turn into
project) are plumbing: agents write files in their own folder and use `workspaces.create` and `sessions.move`.
