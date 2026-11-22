# Peer API 1: another Mac

2026-11-15 (lane M, M2 PR 1, the server side). Lets one Hollow (the MacBook) open and steer the chats of another
Hollow (the Mac mini's `hollow serve`) over an SSH tunnel. The iPhone app never calls these routes, and **no field is
added to any shape the phone reads**: everything new is here, with fixtures in `docs/phone-api-fixtures/machines/`.
The design and the later PRs (the merged chat list, the UI) are in the M2 plan
(`outputs/hollow-build-run-2026-11-14/m2-plan.md` in the Hollow vault workspace). The client side (PR 2: the record
of paired Macs, the SSH tunnel and the HTTP client) is [The calling Mac](#the-calling-mac-pr-2) below.

**The rule:** the peer link is live control only. A paired Mac may list chats, read one chat's transcript, send text
to a chat that exists, stop it, answer a card at that chat's home, and follow the event stream. Nothing else crosses:
no files or attachments, no tasks, no Waiting on you, no notes, no canvases, no search, no push, no new chats (M3a),
no settings or secrets.

**What a machine token can do (the blast radius).** Whoever holds a Mac's token can read every chat this Hollow
lists (titles, previews, whole transcripts), send text to any existing chat, the Main agent's (COO's) included, where
it runs in that chat's own permission mode, stop any running turn, and answer any open question card. It cannot raise
a chat's mode or choose one, pick a model or effort, attach or read files, start a chat, or answer an app action's
approval card (those are not phone API routes). Treat it as the person's hands on chats, and revoke it like a phone.

## How a Mac reaches this one

- **Over SSH only.** The other Mac runs `ssh -N -L <local socket>:127.0.0.1:47680 <alias>`; the request lands on this
  Mac's loopback and sets `Host: 127.0.0.1:47680` (this Mac's own port, or the Host check answers 421). A machine
  pairing attempt or a machine token that arrives with `Tailscale-User-Login`, `Tailscale-Funnel-Request`,
  `X-Forwarded-For` or `X-Forwarded-Host` (that is, through `tailscale serve` or any proxy) is refused 403
  `forbidden` before a code is tried or the token used. The Host, Funnel and tailnet-user checks of
  [Who may call](../PHONE_CHAT_API.md#who-may-call) apply first, as for a phone.
- **"Loopback only" means a request with no tailnet or proxy headers addressed to the loopback name.** Hollow can't
  tell an SSH tunnel from any other process on this Mac, nor from a raw `tailscale serve --tcp` forward to the API's
  port (it passes on nothing of the caller, so it looks like loopback); Settings flags such a forward as a problem, as
  it does for phones. A local process still needs a token or an open window's code.
- **The server listens while a Mac is paired**, whether or not the iPhone app switch is on, and also once a machine
  window opens. With the switch off a phone's token is answered 503 `unavailable` ("The iPhone app is off in Hollow's
  Settings on this Mac."), and the phone's pairing window can't open. With no Mac paired nothing changes for phones.
- **Live views** (`live` and `append` frames) are copied while a Mac is paired, as they are while the iPhone app is on
  with a phone paired.

## Pairing

1. Someone who can run commands as the serving user opens the **machine window** on this Mac: `hollow serve pair
   --machine` (add `--json` for one line the other Mac reads over SSH), or Settings' Let another Mac connect in a GUI
   Hollow (`phone_access_machine_pairing_start`; user-only, never an agent action). It is separate from the phone's
   window: a phone code never pairs a Mac, nor a Mac code a phone. Same rules: an 8-character code from the same
   alphabet, good for 10 minutes or one use, a new window closes the old one, 5 wrong codes close it. The code goes to
   that command's stdout (or Settings) once; it is never logged, and an agent reading the status gets only
   `machinePairingOpen` and `machinePairingExpiresAt`. An agent with a shell as the serving user can still run
   `hollow serve pair --machine --json` and read the code: the trust boundary is that user account, not the person.

   `hollow serve pair --machine --json` prints:

   ```json
   {"code":"7KQ2-M9XA","expiresAt":"2026-11-16T03:22:44Z","machine":true,"machineId":"m-8d21c4be90f17a3e","slug":"mac-mini","name":"Mac mini"}
   ```

2. The other Mac calls `POST /v1/machines/pair` (no token) through its tunnel.

### `POST /v1/machines/pair`

Request (fixture `machines/pair-request.json`):

```json
{"code": "7KQ2-M9XA", "machine": {"machineId": "m-3f9c1a07e2b45d18", "slug": "macbook-pro", "name": "MacBook Pro"},
 "clientVersion": "0.5.17 (abc1234)"}
```

- `code`: case, spaces and dashes ignored.
- `machine.machineId` (required): the calling Mac's own id, `m-` and 16 lower-case hex digits (its `machine.json`).
- `machine.name` (required): 1 to 80 characters, no control characters.
- `machine.slug`: the calling Mac's machine name (lower-case letters, digits and single dashes), or null.
- `clientVersion`: at most 80 characters.

400 `bad_request` for any of those (no try is spent). 403 `pairing_closed` when no machine window is open; 403
`wrong_code` with `attemptsLeft` as for a phone.

200 (fixture `machines/pair-response.json`):

```json
{"token": "<43 characters>", "deviceId": "mc-1a2b3c4d", "machine": MachineInfo, "api": 1, "peerApi": 1}
```

- `token`: shown once. This Mac keeps only its SHA-256, in `phone.json`'s own `machines` list (below).
- `deviceId`: `mc-` and 8 hex digits, never one revoked in this run.
- `machine`: this Mac, as `GET /v1/machine` says it.
- The code is checked first; only then does this Mac read its own identity, and if it can't (its `machine.json`),
  the answer is 503 `unavailable` with a fixed sentence and nothing is paired (the window is spent; open a new one).
- **The same Mac pairing again** (the same `machineId`) replaces its device: the old token is dead at once and its
  streams close. One device per Mac.

## `GET /v1/machine`

Machine token only (a phone's gets 403 `forbidden`). 200 `MachineInfo` (fixture `machines/machine.json`):

```json
{"machineId": "m-8d21c4be90f17a3e", "slug": "mac-mini", "name": "Mac mini", "app": "hollow", "api": 1,
 "peerApi": 1, "version": "0.5.17", "commit": "abc1234", "headless": true,
 "capabilities": ["chats", "send", "stop", "cards", "events", "usage", "moves", "appends"]}
```

- `machineId`: this Mac's stable id, written once into `~/.config/hollow/machine.json` (`context_roots::ensure_machine_id`)
  and kept by every writer of that file, also when the slug changes.
- `slug`: `machine.json`'s slug (the person's name for this Mac, `home/<slug>.md`), null until one is set.
- `name`: the Mac's name (Sharing settings).
- `api`: the phone API's version (1). `peerApi`: this protocol's (1); a client that finds a larger one than it knows
  asks for an update.
- `version`, `commit`: this Hollow's build. `headless`: `hollow serve` (no window).
- `capabilities`: what a Mac may do here. `usage` (A2, 2026-11-15) is `GET /v1/machine/usage` below; `moves` (M5) the
  chat move routes and `appends` (M5 PR3) an offline sync on them ([Offline sync](#offline-sync-m5-pr3)). Later features
  add to it (`history`); a client shows only what it finds. `coo` (M3a): this Hollow runs the runner COO
  (`hollow serve runner-coo on`) and it has a conversation; `coo` then also appears as a field, that conversation's
  key (`runner_coo::peer_coo`), and is absent otherwise. A client records it (`MachineRecord.coo`) and its COO
  becomes the dispatcher. `coo-start` (M3a follow-up) in its place: this Hollow is the runner and its COO has no
  conversation yet; a paired Mac may start it (`POST /v1/machine/coo`).

503 `unavailable` when this Hollow can't read or write its `machine.json` (a hand edit that isn't JSON is left alone).

## `GET /v1/machine/usage`

A2 (2026-11-15). Machine token only. This Hollow's account snapshot, so another Mac that spreads runs over the same
logins doesn't pick the account this one just picked (`peer_usage.rs`). 200 (fixture `machines/usage.json`):

```json
{"usageApi": 1, "machineId": "m-8d21c4be90f17a3e", "observedAt": "2026-11-15T20:41:12Z",
 "accounts": [{"key": "ak-5c0b8e21d47f9a36", "provider": "claude", "label": "Claude Code login", "billing": "Acme",
   "routed": true, "loadPercent": 42.5, "blocked": false, "blockedUntil": null,
   "sevenDayResetsAt": "2026-11-19T04:00:00Z", "measuredAt": "2026-11-15T20:41:07Z", "recentPicks": 2,
   "lastPickedAt": "2026-11-15T20:40:55Z"}]}
```

- `usageApi`: this shape's version (1); a client that finds a larger one uses nothing from it.
- `accounts`: at most 16. Each added Claude account that is usable and has a key, the Claude Code login only when it
  is what runs here use (no account added, or none usable: the mini's case), and the Codex sign-in when it is a
  ChatGPT login. An account with no key yet is left out.
- `key`: `ak-` and 16 hex digits of a SHA-256 over a fixed domain and an id both Macs can read for the same login and
  that is not a secret: a Claude account's organisation uuid (its usage probe's `anthropic-organization-id` header for
  an added account, `~/.claude.json`'s `oauthAccount.organizationUuid` for the login), and the ChatGPT user and
  workspace ids of the Codex `id_token` claims. No token, email or account id crosses.
- `provider`: `claude` or `codex`. `label`: the account's name (the login is "Claude Code login", Codex is "Codex
  (<plan>)"). `billing`: its organisation when known (Settings' organisation for an added account, the login's
  `organizationName`, the Codex claims' default organisation). `routed`: this Hollow's runs use it now.
- `loadPercent`: the fuller live window (five-hour or weekly), null when never measured; `measuredAt` when.
  `blocked` and `blockedUntil`: out of plan allowance, and when a window resets. `sevenDayResetsAt`: the weekly
  window's reset, the same instant on both Macs for one account.
- `recentPicks`, `lastPickedAt`: runs and new conversations this Hollow placed on it in the last ten minutes (a
  conversation's next turn is not a pick; kept in memory only). A label or billing name holding `@` is sent empty.

**How the calling Mac uses it.** Only for a Mac whose `GET /v1/machine` lists `usage`, in the background at most
every 45 s and only when a pick asks (two or more usable Claude accounts with a key), kept 3 minutes; a run start
never waits, and a Mac that is down, older or answering as another Mac contributes nothing. When it chooses, an
account known by the same key on both Macs, whose weekly reset both sides know and agree on (within an hour), takes
the peer's measurement when it is fresher, at most 5 minutes old and a load in 0 to 100 (a future time counts as
now), and the peer's block when it has a load and a `blockedUntil` at most a week ahead (a block counts until then
however old the reading). An account the peer picked
(placed a run or a new conversation on) in the last ten minutes ranks after every eligible account it didn't pick,
also when a reset is unknown, kept under the ceiling so a peer's pick never makes an account ineligible. A key is not
used when it names two accounts on either side (seats of one organisation), nor at all when the resets disagree. A
failed fetch keeps that Mac's last snapshot until its 3 minutes are up; each Mac's answer is kept as it arrives, and a
whole fetch gives up after 20 s.
With no peer snapshot the choice is exactly the single-Hollow one. Only the MacBook reads the mini's snapshot (the
tunnel runs one way), so the mini still picks alone: with no account added it has one, its login.

## `POST /v1/machine/coo`

M3a follow-up (2026-11-15). Machine token only. Starts the runner COO's first conversation, the person's Start its
COO on the paired Mac: `{"text": "...", "clientMessageId": "coo-start-..."}` (text only, 1 to 100,000 characters;
other fields are 400). It starts in the `main` workspace as a phone's new chat there does (this Hollow's new-chat
mode, `mac-<clientMessageId>` turn id, a repeated id answers the first reply) and answers 202 as `POST /v1/sessions`
does (`{accepted, key, state, queueId, waitingFor}`). Two calls at once start one. 404 `not_found` when this Hollow
isn't the runner; 409 `busy` when its COO has a conversation already (its key is `GET /v1/machine`'s `coo`). No other
chat can be started by a Mac.

## `DELETE /v1/machines/me`

Machine token only. 204: the calling Mac is unpaired (its Forget); its streams close and its next request gets 401.
This Mac's Settings (or `phone_access_revoke` with the `mc-` id) revokes a Mac the same way; a Settings revoke of
the last Mac with the iPhone switch off also stops the server, while a Mac's own `DELETE` leaves it listening (loopback
only, every route needing a token) until the switch moves or Hollow restarts. A revoke never stops the server while a
machine window is open.

## What a machine token may call

Default deny: `routes::ROUTES` gives every route a scope, a test walks every route against a machine token and a phone
token, and another test fails until every route `dispatch` answers is in that table.

| Route | Phone | Mac |
|---|---|---|
| `GET /v1/sessions`, `GET /v1/sessions/{key}` | yes | yes |
| `POST /v1/sessions/{key}/messages` | yes | text only (below) |
| `POST /v1/sessions/{key}/stop` | yes | yes |
| `POST /v1/sessions/{key}/questions/{id}/answer` | yes | yes |
| `GET /v1/events` | yes | yes |
| `GET /v1/machine`, `GET /v1/machine/usage`, `DELETE /v1/machines/me` | 403 | yes |
| `POST /v1/machine/coo` (the runner COO's first conversation only) | 403 | yes |
| `POST /v1/machines/moves/{id}/parts/{n}`, `POST .../commit`, `GET` and `DELETE /v1/machines/moves/{id}` (M5) | 403 | yes |
| every other route (`me`, `chat-controls`, new chat, attachments, workspaces, tasks, notes, search, waiting, push, live-activity, canvases) and any unknown path | as before | 403 |

A refusal is 403 `forbidden` (fixture `machines/error-machine-forbidden.json`):

```json
{"error": {"code": "forbidden", "message": "Another Mac can't use this through Hollow's link."}}
```

A phone at a Mac's route gets 403 `forbidden`, "Only another Mac paired with Hollow uses this, not an iPhone."

`GET /v1/me` and `GET /v1/chat-controls` are refused to a Mac, although the M2 plan allowed them: `GET /v1/machine`
is a Mac's identity, and a Mac's send takes no controls. The client's commands never call either (a test scans for them).

**A Mac's send** is text only: a body with `attachmentIds` (even empty) or `controls` is 400 `bad_request`. The turn
runs as the chat's own composer send would at its home: its agent, model and the permission mode this Mac keeps for
the chat (the window's choice for it, else its last turn's; a headless home has no window, so the last turn's).
Its turn's `clientTurnId` is `mac-<clientMessageId>` (a phone's is `phone-<...>`). Everything else about sending,
`clientMessageId` repeats, `busy` and the card answer is as for a phone.

## Moving a chat here (M5)

2026-11-15 (M5 PR1, the receiving side; `chat_move.rs`). The one exception to "the peer link never copies a
transcript" (S32): a Mac moves one of its chats here, its history sent once, so the chat keeps going here and the
person opens it on either Mac. Design, the trigger (PR2) and the offline path (PR3): the M5 plan
(`outputs/hollow-build-run-2026-11-14/m5-plan.md` in the Hollow vault workspace). `GET /v1/machine` lists `moves`
in `capabilities`.

**The bundle** is one JSON object (`chat_move::MoveBundle`, version 1): `moveId` (`mv-` and 16 lower-case hex),
`key`, `from` (the sending Mac's `machineId`, which must be the caller's), `session` (title, agent, created and last
activity times, `parentKey`, `forkKind`), `place` (`{"workspaceId"}`, with `"worktree": true` when the chat ran in that build workspace's worktree,
`{"vaultPath"}` relative to the vault, or `{}` for the vault root; never a path of the sending Mac), `native` (the newest native session's agent and id, or null),
`effort`, and `turns` (each `ord`, `runId`, times, `status`, `asked`, `permissionMode`, `effort`, `askedBy` and the
stored `transcript` object). At most 2,000 turns and 16 MiB (a longer chat moves in slices, below). Every turn must be finished (a `status` of lower-case
letters that isn't `running`, `queued`, `claimed`, `pending` or `starting`), and each `permissionMode` one of Hollow's
five, or the commit is 400. An effort Hollow doesn't know (`effort::canonical`) is dropped, never refused.

### `POST /v1/machines/moves/{id}/parts/{n}`

`{"of": N, "data": "<base64>"}`: part `n` (from 0) of `N` (at most 103), at most 160 KiB of bundle each. 200
`{"moveId", "parts": <received so far>}`. Parts are kept in memory only: at most 4 moves at once and 2 from one Mac
(409 `busy` past that), dropped 10 minutes after their last part. 400 for a bad id, `of`, part number or base64; 413 `too_large` past a
part's or a bundle's size; 403 `forbidden` for a move another Mac began.

### `POST /v1/machines/moves/{id}/commit`

`{"parts": N, "sha256": "<hex of the whole bundle>"}`. The parts are joined, checked and imported in one SQLite
transaction (`chat_move::import_move`): the chat's `sessions` row (its folder is this Mac's for `place`), turns and
transcripts as sent, its effort, a closed stint with reason `moved` holding the old native id and a fresh open
stint (the next turn starts a fresh native session with the conversation bundle; a send naming the old id resets),
and the home record `arrived` from the caller. 200 (`chat_move::Imported`):

```json
{"moveId": "mv-1f2e3d4c5b6a7980", "key": "s-20261115-ffffffff", "turns": 12, "already": false}
```

- **Once:** a commit for a move already imported answers the same with `already: true` and changes nothing.
- 409 `conflict`: a chat with this key is here and isn't one this Mac moved to the caller earlier (that one comes
  back: its rows are replaced); or the Main agent's chat (each Mac has its own). 409 `not_here`: the workspace or
  folder isn't in this Mac's vault yet (sync first), or a `worktree` chat's workspace has no live worktree here. A
  `vaultPath` is resolved with links followed and must stay inside the vault, outside the Main agent's folder and
  outside any dot-folder (400 or 409 `conflict` for the Main agent's). 409 `cancelled`: the move was called off. 400: parts missing, a
  digest or count that doesn't match, a bundle Hollow can't read, another version.

### `GET /v1/machines/moves/{id}`

200 `{"moveId", "state", "key"?, "parts"}`: `state` is `receiving` (parts staged, no commit), `imported`,
`cancelled`, or `unknown` (nothing landed). The sending Mac asks this when a commit's answer was lost.

### `DELETE /v1/machines/moves/{id}`

Call a move off. 200 `{"cancelled": "notImported"}` writes a tombstone so a late commit is refused (the newest
200 per Mac are kept);
`"removed"` undoes an import no turn has used (a chat that came back stays here, `away` again); `"already"`. 409
`started` when a turn already ran or waits here: the move stands and the sending Mac finishes it instead, so the chat
is never owned by both Macs or by neither.

**Who owns a moved chat** (`chat_homes` in each Mac's own Sessions DB, never the vault): `leaving` while a move
runs (also while it is called off with this Mac out of reach: the sending Mac keeps the move `aborting` and runs
nothing until this Mac answers the `DELETE`), `away` on the Mac it left (the M2 list merge's `movedTo`), `arrived` on the Mac it came to (`movedFrom`). Since the
M5 UI (2026-11-15), for a **machine token only**, the list rows and `session` in `GET /v1/sessions/{key}` carry them:
`movedTo` (an `away` chat, the machine id it lives on now) and `movedFrom` (an `arrived` chat, the machine id it came
from), each absent otherwise (`routes::mark_moved`). A phone's rows never have them: the iPhone contract
(`docs/PHONE_CHAT_API.md`, shared with the iPhone repo) is unchanged. The
run queue never runs a `leaving` or `away` chat's turn (`chat_move::admission_refusal` in
`chat_queue::admit_checked_turn`).

**The sending Mac** (M5 PR2, `chat_move_send.rs`, `chat_move_trigger.rs`): it writes `leaving`, sends the parts and
the commit, and settles only on what this Mac answered for that move id, never on a timeout, one delivery per move id
at a time: 200 to the commit finishes the move (`away`); a 4xx `conflict`, `not_here`, `too_large` or `cancelled` to
the commit gives the chat back to it; anything else (no answer, a 5xx, a 404, a 400, a 409 `busy`) makes it ask `GET`: `imported` finishes,
otherwise it sends the `DELETE` and settles on that answer (200 back home, 409 `started` finishes). With no answer at
all the chat stays `leaving` (it runs on neither Mac) and the move is sent again under the same id every minute and at
wake. A running turn is first asked to stop at its next safe point, or stopped outright when the time runs short;
once the chat landed, it gets one continuation here through `POST /v1/sessions/{key}/messages` (`clientMessageId`
`moved-<moveId>`, so a retry runs it once): "This chat moved here from <Mac> while a turn was running. Continue from
where it stopped." On macOS, when the person turned it on, the sending Mac moves its person's running chats to the runner
when the system's sleep notice comes (`kIOMessageSystemWillSleep`, 20 seconds, then it allows the sleep); with Keep
Hollow running with the lid closed on, no notice comes and nothing moves. `hollow serve` never does this. A move open
past 15 minutes is logged and told once in a notification; a retried move refused before sending (its folder gone,
too large) is settled with this Mac's answer and comes home.

### Offline sync (M5 PR3)

2026-11-15 (`chat_offline.rs`; capability `appends`). A chat that moved here and was continued on the sending Mac
while this Mac was out of reach comes back as turns, not as a second copy. On the sending Mac, Continue here offline
(`chat_continue_offline`) makes a new chat of its own from its copy of the moved chat K (`parentKey` K, `forkKind`
`offline`), with a base: K's last turn there (`ord`, `runId`). On the next contact with a Mac listing `appends`, that
chat (O) is sent as a move over the routes above (`leaving` while it is sent, the same proof rules) whose bundle is
version 2 with `"append": {"to": K, "base": {"ord": 12, "runId": "..."}}` and only O's turns after the base, each with
a `runId` of its own (at most 200 characters). A Hollow from before PR3 refuses version 2 with 400, which is no proof:
the sender asks `GET`, sends the `DELETE`, and O stays with it.

The commit, in one transaction (`chat_offline::import_append`), in this order:

- **An earlier sync of O forked it here** (the sender released O before folding the answer and sent it again): the
  same base and common-turn checks as below against that fork, then the turns go to it, each `runId` once: none
  added when it has them all, the rest after its last turn when it hasn't moved on since the base, else 409
  `conflict`. 200 with `"outcome": "forked"` and `key` O.
- K must be here and `arrived` from the caller (409 `conflict` when it is gone; 409 `busy` while it is `leaving` or
  lives on another Mac, which is no proof: the sender calls the sync off and tries again later), with its turn at
  `base.ord` carrying `base.runId`, no offline `runId` naming one of K's turns up to the base, and the offline turns
  K has already being the first ones, in order; otherwise 409 `conflict` and nothing lands.
- **Every offline `runId` is in K already** (a sync sent again): nothing is added, whatever K did since or whether it
  is archived, and no fork is made. 200 with `"outcome": "appended"`, `turns` 0.
- **Appended:** every turn K has here after the base is one of the sync's run ids and nothing is queued or running in
  K: the turns K lacks (by `runId`) are added after its last turn, and K's native session is closed as a moved
  chat's is (its next turn starts a fresh one with the whole conversation; a send naming the old id resets). 200
  with `"outcome": "appended"` and `key` K.
- **Forked:** K moved on here (or is archived, or has a turn queued or running, or its agent process is still
  warm): a new chat under O's key, `parentKey` K, `forkKind` `offline`, in K's folder, holding K's turns up to where
  the histories part (the base, or the last offline turn K has already) and then only the offline turns K lacks,
  `arrived` from the caller; K is not touched.
  200 with `"outcome": "forked"` and `key` O.
- The same sync again answers with `already` and changes nothing. `GET` answers `imported` with `outcome` and `key`.
  A `DELETE` of a sync whose turns went into a chat here (appended, or into an earlier fork) is 409 `started`; of a
  fork this sync made, as a move's. Sent times (the chat's and each turn's) are read as times and stored in this Mac's
UTC form; one later than this Mac's clock, or not a time, is taken as now. Appended turns also drop K's paused record
and folded task state (folded again).

The sending Mac then folds the answer in: appended, O's turns join its copy of K after its last turn, with their
transcripts, trace, steps, cards (and a card's answered-after turn) and feedback votes, and O goes; with no copy of K
left there it writes nothing under K's key (a send to O is refused from then on: "Continue
there"); forked, O is `away` like a moved chat. Until the answer is had O runs nowhere, and the next contact (every
minute, at wake) folds it; a continuation owed to it is dropped rather than sent to a key that may not be there.

**Slices (2026-11-15):** offline turns past one sync's caps go in several syncs (`chat_offline::Slice`: the first
turns in order, at most 2,000 and 16 MiB less 1 MiB for the rest of the bundle). Each is an ordinary sync to the
receiver. When one lands, the sender moves O's base to the last turn it carried (how many it carried is the move
log's `turns`), O comes home, open, and the next slice follows at once (up to 8 per contact, on the retry and on a
Move to runner or sleep move alike): into K when K took the first, into the fork when the first forked (the
earlier-fork case above). The same slice's answer folded again (read twice, or a retry after the fold) changes
nothing (stale): its move is done and O is no longer `away` under it, and an owed continuation stays with the
slice that is out. Its copy of K takes all the offline turns
in one fold at the end, counted from that copy's own last turn.

**A chat longer than one sync (Lane M, 2026-11-18):** a chat that doesn't fit one move (2,000 turns or 16 MiB) used
to stay on its Mac. It now moves in slices when it moves from the sending Mac (`chat_move::begin_move_out_sliced`). A
chat that fits goes exactly as before (version 1, no `slices`). A longer one sends **earlier slices** first: bundles of
version 3 with `slices {root, no, last: false}` under ids of their own, each at most one sync's caps (the same
`Slice` rule), sent over the same parts and commit routes. The other Mac keeps each, in order, in its Sessions DB
(`chat_slices_in`, nothing of the chat is visible there yet), answering 200 with `outcome: "staged"` (`already` when it
had that slice id). The move's own bundle comes last (`slices {root = moveId, no = how many wait, last: true,
beforeTurns, beforeOrd}`, the rest of the turns): its commit checks that exactly those slices wait (one Mac, one chat,
turns in order, `beforeTurns` and `beforeOrd` match) and imports the whole chat in one transaction, so ownership still
flips once. A missing or out-of-order slice is refused 409 `conflict` (the sender calls the move off). At most 16
earlier slices per move (so 17 syncs), 64 waiting on a Mac in all, dropped 24 hours after the last one arrived; the
move's `DELETE` drops them and tombstones the id (a late slice is refused `cancelled`). The sending Mac keeps progress
in `chat_slices_out` and the chat stays `leaving` throughout (it runs nowhere): after a restart a slice that was out
is sent again under its id and one that landed never is. A slice the other Mac never answers makes the move wait
(`Waiting`, retried each minute); one it refuses calls the whole move off. A receiver from before refuses version 3
(400), which is no proof: the move is asked about and called off as any unanswered one, and the chat stays here.

**Kept here:** O becomes an ordinary chat of the sending Mac's, never sent as a sync again (a Move to runner carries
it whole), only while it is home and only for good: a commit refused `conflict` or `too_large`, a turn with no
`runId`, one offline turn larger than a sync carries, or the person's Keep it here. The reason is kept with it
(`chat_offline.note`), and the window shows it on O's notice (`chat_offline_rows`, [peers](../reference/peers.md)). A `busy` K, a `cancelled` tombstone, an
archived O, a missing folder or a race leave it to sync on the next contact; once K came back to the sending Mac, O
is not sent at all and stays a chat there.

## The record

`<support>/Companion/phone.json` gains a `machines` list beside `devices`; a device gains `kind` (`"machine"`; a
phone's record never says, and a device without `kind` is a phone) and `machineId`:

```json
{"enabled": false, "devices": [], "machines": [{"deviceId": "mc-1a2b3c4d", "kind": "machine",
 "machineId": "m-3f9c1a07e2b45d18", "name": "MacBook Pro", "model": null, "tokenSha256": "<64 hex>",
 "pairedAt": "...", "lastSeenAt": "..."}]}
```

- Macs are kept apart from `devices` on purpose: a Hollow from before M2 reads only `devices`, so it lets no machine
  token in (fails closed) instead of taking a Mac for a phone with every phone route. Its next write drops the list,
  and the Mac pairs again after an update.
- Fields a later Hollow adds to the record or to a device are kept on every write from this version on.
- Settings' status (`phone_access_status`) lists the paired Macs (`machines`: device id, `machineId`, name, paired and
  last seen) and the open machine window (`machinePairing`, never in what an agent reads).

## The calling Mac (PR 2)

2026-11-15. What the MacBook side does, in `peer_machines.rs`, `peer_tunnel.rs` and `peer_client.rs`. A Rust API only;
PR 3 adds the commands and the webview's stores ([peers](../reference/peers.md)); PR 4 the UI.

**The record.** `<support>/Peers/machines.json` (0600, folder 0700), never Settings or the vault:

```json
{"version": 1, "machines": [{"machineId": "m-8d21c4be90f17a3e", "slug": "mac-mini", "name": "Mac mini",
 "sshHosts": ["runner", "runner-lan"], "remotePort": 47680, "deviceId": "mc-1a2b3c4d",
 "pairedAt": "...", "lastSeenAt": "...", "lastVersion": "0.5.17 (abc1234)", "peerApi": 1}]}
```

- At most 4 Macs; `sshHosts` 1 to 8 aliases (`remote_session::valid_host`), tried in order. A new pairing tries the
  alias it was given, then the same alias with `-lan` when `ssh -G` shows `~/.ssh/config` gives that one a host of
  its own (`peer_machines::hosts_for`). `tokenRevoked: true` appears after a 401 whose body says `unauthorized`
  and goes at the next good answer or pairing; a marked record still gets a client, so the mark never locks a Mac out
  on its own. Its `slug` must be a valid slug and its `deviceId` `mc-` and 8 hex digits. Unknown fields are kept; a file that doesn't parse is left in place and every read says so.
- **The token is never in this file**, argv or a log: `secret_store` key `peer:<machineId>` (the app's Keychain
  bundle). Forget removes both, after a best-effort `DELETE /v1/machines/me`.

**The tunnel** (`peer_tunnel::Tunnel`, one per Mac): `ssh <SSH_OPTS> -N -T -o ExitOnForwardFailure=yes -o
StreamLocalBindUnlink=yes -o PermitLocalCommand=no -L <socket>:127.0.0.1:<remotePort> -- <alias>`. The socket is
`$TMPDIR/hollow-peer/<machineId>.sock`, its folder made 0700 (and checked to be this user's) by `Tunnel::new`, refused
past 103 bytes (`sun_path`) or with a colon (it would split `-L`). `ensure()` removes an old socket file, starts ssh and
polls the socket every 100 ms for 12 seconds per alias (that is its own bound: a request's 10 seconds start only once
the socket is up, so a hanging first alias never cancels the try of the next); an ssh that exits
first failed, and its stderr's last line plus `remote_status::likely_causes` is the message. Aliases are tried in
order; when all fail the next try waits up to 2, 5, 15, 30, then 60 seconds (jittered, see Reconnect rules below; `retry_now()` skips the wait). A dead ssh is
restarted by the next `ensure()`. The ssh leads its own process group with a run tag and is recorded in the run
ledger (`process_group::GroupGuard`), so dropping the tunnel, Hollow's quit and quit signals, the crash reaper and the
next launch's sweep all end it; nothing is killed by name. `close_if_idle(IDLE_CLOSE)` (10 minutes) runs from the
peer hub's minute sweep for a Mac with no stream open; every `ensure()` and every event-stream frame read through it counts as use. Text from the other side (error
messages, names, ssh's stderr line) is shown through `peer_client::remote_text`: control and direction-changing
characters dropped, at most 200 characters.

**The client** (`peer_client::PeerClient`): hyper's HTTP/1 client, one connection per request, over `Dial::Tunnel`
(the app) or `Dial::Tcp` (tests). Every request sets `Host: 127.0.0.1:<remotePort>` and
`Authorization: Bearer`, never a forwarding or Tailscale header; 8 requests at once, 10 seconds each, bodies up to
4 MiB. Calls: `machine()` (`GET /v1/machine`, 404 means the other Hollow needs an update, a larger `peerApi` means
this one does), `sessions(cursor, limit)`, `detail(key, before, limit)`, `send(key, text, clientMessageId)`,
`stop(key)`, `answer(key, questionId, clientMessageId, answers)`, `unpair_me()`, `events(session, watchdog)`. Keys
and card ids that aren't Hollow's shape are refused before any request. A 401 saying `unauthorized` marks the
record, every other answer but a 401 touches `lastSeenAt` (at most once a minute; the file is written off the async
threads). `machine()` also checks the answering Mac's id against the record's (another Mac now is `PairingNeeded`).

Errors are `PeerError` variants with a sentence a person can read, each mapped to the UI's state with `state()`:

| `PeerError` | From | `PeerState` |
|---|---|---|
| `Offline` | tunnel or ssh failed, nothing answers, no answer in 10 s, stream silent 40 s | `offline` |
| `TokenRevoked`, `PairingNeeded` | 401 (record marked when it says `unauthorized`), no token saved, another Mac answering | `pairing-needed` |
| `UpdateNeeded` | `GET /v1/machine` 404, a newer `peerApi`, a stream `hello` that isn't api 1 (the message says which side is newer and names its `build`) or has no integer `api` | `update-needed` |
| `Forbidden` (the server's sentence), `Busy`, `QuestionClosed`, `NotFound`, `BadRequest`, `TooLarge`, `Unavailable`, `Misdirected` (421), `PairingClosed`, `WrongCode`, `Internal` | the error body's `code` | `online` |
| `Local` | this Mac's record or Keychain | `online` |

`PeerState` also has `this-mac` (the webview's own row) and `connecting` (the hub, while a tunnel opens), serialized kebab-case.

**Events** (`peer_client::follow(client, session, options, sink)`): reads `GET /v1/events` frame by frame. `hello` must
say api 1 (`read_hello`; its other fields are optional and unknown ones ignored). AG8 (2026-11-17): a frame kind not
in `wire::STREAM_EVENTS` (a newer peer's addition) is dropped, not passed to the sink and not fatal, with one
"[peer] dropping <kind> events from <Mac> (build <build>)" line in the log per kind per stream (`DroppedEvents`, at
most 32 kinds remembered); never in the chat. A followed chat's `append` passes on only when its `ord` and `gen` match the `live` view held; otherwise,
and after every `turn` and reconnect, the chat's detail is fetched again. After such a refetch, `append` frames the
server queued before that snapshot would repeat text it holds, so appends are held until the stream is quiet for
500 ms (at most 2 s while text keeps coming), then the chat is fetched once more; a `live` frame (a whole view) ends
the wait. A list stream (no `session`) fetches the list again on
`sessions`, at most once every 2 seconds. No bytes for 40 seconds (the server pings every 15) drops the stream; it
reconnects after up to 1, 2, 5, 10, then 30 seconds (jittered; the waits start over only after a stream stayed up 30 seconds)
and sends the newest `id:` it read as `Last-Event-ID` (AG5, 2026-11-17; [events](../PHONE_CHAT_API.md)): when `hello` says
`resumed: true` the missed events are replayed and nothing is fetched; otherwise (`resumed: false` with a `resync` event,
an older peer with no ids, or a refetch still settling or a list fetch still due when the stream ended) it fetches the chat
or the list again as before. `resync` is read and not passed to the sink. A 401, an update needed, a chat gone or a refused route ends it (`FollowEvent::Ended`). The sink gets
`Connected`, `Event` (the frame), `Detail`, `Sessions`, `Disconnected` (message and wait) and `Ended`; dropping the
returned `Follow` stops it. The peer hub turns these into `peer-chat-event` through `Core::emit` ([peers](../reference/peers.md)).

**Reconnect rules** (PL1, 2026-11-17, `peer_link.rs`):

- **One retry owner per link.** Only `Tunnel::ensure` opens a link, under the tunnel's state lock; every caller that
  finds it down waits for that one opening. A stream's reconnect and a command's retry both dial through it.
- **Jittered exponential backoff.** The tunnel's and the stream's waits are ceilings; each wait is drawn between half
  the step and the whole step (`peer_link::Jitter`, equal jitter), so links that dropped together don't return in
  lockstep. Tests pass `Jitter::fixed`.
- **Liveness.** The stream watchdog (40 s against the server's 15 s ping) and ssh's `ServerAliveInterval`; the next
  `ensure` sees a dead ssh and opens it again.
- **Acknowledged after the intent is saved, idempotent by id.** The server saves a send's or card answer's reply
  under its `clientMessageId` before answering and gives a repeat that reply. So `send` and `answer` are sent once
  more with the same id when the link drops before the answer (`peer_link::retry_command`, at most 2 tries, only on
  `Offline`); a refusal is never retried. Before the retry the tunnel the try used is closed
  (`Tunnel::close_if_opened_before`, only a link that came up no later than the failure), since a quiet drop
  leaves ssh alive for up to a minute. The sender keeps the id too: `sendRemote` (`src/store/peerChat.ts`) reuses a
  send's id while it failed `offline`, so the person's retry of the same text in the same chat is the same command;
  any answer or refusal forgets it. A card answer still gets a new id per tap. `stop` carries no id and is not retried. Not covered: `POST
  /v1/machine/coo` checks `busy` before the repeat check, so its retry would answer `busy`, and the repeat memory is
  in memory for 10 minutes (a server restart between the try and the retry forgets it).

**Pairing** (`peer_client::pair_over_ssh_with`, the hub's `peer_pair`): runs `ssh <SSH_OPTS> -T -- <alias> 'hollow-serve/bin/hollow serve
pair --machine --json --data-dir="$HOME/hollow-serve/data"'` (the layout in [HEADLESS_SERVE.md](../HEADLESS_SERVE.md);
the hub gives another command, port and ssh in tests, and the tunnel through `TunnelFor`, its own one for that Mac), stdin closed, stdout capped at 4 KiB, 20
seconds. Exit 255 is ssh's failure (a command's own 255 is turned into 3, as `remote_session` does). The line must be a
machine window's (`machine: true` and a valid `machineId`), else the other Hollow needs an update. Then the tunnel
opens through that alias, `POST /v1/machines/pair` sends this Mac's `machine.json` id, slug, name and version, an
answer from a different `machineId` than the code's is refused (and its token given back), the token is saved, the
record written, and `GET /v1/machine` with the new token is the check. Any failure after the token was handed out
(saving it, the record or the 4-Mac cap, the check, another id at the check) gives it back with `DELETE
/v1/machines/me` and keeps neither record nor token here. The code is never in argv or a log, and `PairCode`'s Debug
and Display never show it.

