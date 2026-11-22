# Peers: opening another Mac's chats

M2 (2026-11-15): this Mac (the MacBook) pairs with another Mac's Hollow (the mini) and opens and steers its chats.
The wire contract, the server side and the client's tunnel and HTTP layer are in
[docs/phone-api/machines.md](../phone-api/machines.md); this page is the commands and the webview's stores on top.
The link's reconnect rules (one retry owner, jittered backoff, liveness, a send or card answer retried once with
its id) are its Reconnect rules section (PL1, `peer_link.rs`).

**The rule:** the peer link is live control only. List chats, read one chat's transcript live without keeping it,
send a message, stop, answer a card at the chat's home. No files, tasks, Waiting on you, notes, search, settings
or secrets cross, and nothing about a remote chat is written on this Mac. Since A2 (2026-11-15) the one exception is
the account snapshot (`GET /v1/machine/usage`, no secrets), which `Hub::usage_snapshots` fetches in the background
for `peer_usage.rs` so spreading runs over accounts doesn't double-book one the other Mac just picked.
The other exception is M5's move of a whole chat, its history sent once
([Moving a chat here](../phone-api/machines.md#moving-a-chat-here-m5)).

**Different builds (AG8, 2026-11-17):** the MacBook and the mini often run different Hollow builds. A stream's
`hello` names the server's `build` and its event kinds; `peer_client::read_hello` turns an `api` other than 1 into
an `update-needed` notice that says which side is newer, and a frame kind this Hollow doesn't know is dropped with
one log line per kind per stream (`DroppedEvents`), never shown in the chat
([the calling Mac](../phone-api/machines.md#the-calling-mac-pr-2), Events). AG5 (same day): a reconnect sends `Last-Event-ID`
and skips the refetch when the peer replays (`hello.resumed`); an older peer or a `resync` falls back to the refetch.

## Commands (`src-tauri/src/commands/peer_api.rs`)

Every command answers Hollow's main window only and rejects with a `PeerFailure`
`{code, message, state, attemptsLeft}` (`code` one of `offline`, `pairing-needed`, `update-needed`, `forbidden`,
`pairing-closed`, `wrong-code`, `not-found`, `busy`, `question-closed`, `bad-request`, `too-large`, `misdirected`,
`unavailable`, `local`, `internal`; `state` the Mac's `PeerState` after it).

| Command | Tier | What it does |
|---|---|---|
| `peer_machines(check?)` | plumbing | `PeerMachinesView`: `thisMac` (local rows' home), each paired Mac as `PeerMachine` (record fields, `state`, `message`, `checkedAt`, the `alias` its tunnel is up through, and since the M5 UI `moves`: its last `GET /v1/machine` listed `moves`, kept in the record's `capabilities`), `suggestedAlias` (the runner in `~/.config/hollow/slot-remote`), and `moveTo` / `sleepTo` (the machine ids Move to runner and the sleep move would pick, `pick_for_move` / `pick_runner`, each only when that Mac lists `moves`). `check` asks each Mac `GET /v1/machine` first, skipping a tunnel's backoff |
| `peer_pair(alias)` | user-only | `alias` is what was typed in Pair a Mac: an ssh alias, `user@host`, a tailnet or dotted name or IPv4 address, each with an optional `:port` (the port Hollow answers on there, default 47680). `parse_pair_address` refuses URLs and paths, IPv6, loopback (`localhost`, `127.x`), `0.0.0.0`, broadcast, multicast, half-written IPv4 and ports outside 1 to 65535; the Mac is then reached only by the ssh login below (no new route, no fetch of a typed URL). One pairing at a time. `ssh <alias> hollow serve pair --machine --json`, the Mac's tunnel, `POST /v1/machines/pair`; the record and Keychain token |
| `peer_forget(machineId)` | user-only | `DELETE /v1/machines/me` when it answers within 15 s, then the record, token, streams and tunnel go here, and (M5 PR2) every chat still moving to that Mac is released to this one; in `commands/chat_move_api.rs` since M5 PR2 |
| `peer_reorder_hosts(machineId, hosts)` | user-only | the aliases and their order (Settings adds and removes them through it too: 1 to 8 SSH names, each once, checked by `peer_machines::check_hosts`); the next try skips the backoff |
| `peer_pair_cancel()` | user-only | Cancel on the Pairing step: stops the one pairing in flight (`PairCancel`: the ssh code read ends, the made tunnel stops opening; a trade that already went through is unpaired there and forgotten here, record and token). Nothing is kept. `true` when one was running |
| `peer_cancel_connect(machineId)` | user-only | Cancel while a Mac connects: `Tunnel::stop_opening` ends the opening ssh within one 100 ms poll, starts no new one and counts no failure; the Mac reads `offline` with "Connecting was cancelled. Retry connects again." until Retry or Check now (`peer_machines(check)`), an alias change or a new pairing (each calls `retry_now`, which clears it); a tunnel that is up stays up and is not marked (a Cancel that races its alive check is cleared by the next `ensure` that finds it up). Accepted race (review): a Retry and a Cancel within the same 100 ms poll are not ordered, so the one that lands last wins and the shown state may lag by one call; clicking again settles it |
| `peer_sessions(machineId)` | plumbing | `PeerSessionsPage`: pages of 200, at most 400 rows, each a `PeerSessionRow` (the wire row flattened beside `home: MachineRef`) |
| `peer_detail(machineId, key, before?, limit?)` | plumbing | `PeerChatDetail {home, detail}`; `detail.turns` are the wire's turns as sent |
| `peer_watch(machineId, key?)` / `peer_unwatch` | plumbing | follow the Mac's list (`key` null) or one chat; one of each per Mac, a new chat replaces the one followed |
| `peer_send(machineId, key, text, clientMessageId)` | user-only | text only, as the chat's composer at its home sends it |
| `peer_stop(machineId, key)` | user-only | Stop at its home |
| `peer_answer(machineId, key, questionId, clientMessageId, answers)` | user-only | a card answered at its home; `answers` maps item ids to text, a list of text or null |
| `peer_start_coo(machineId)` | user-only | M3a follow-up: Settings, Devices, COO on this Mac, Start its COO: `POST /v1/machine/coo` on a runner that lists `coo-start`; the record then names that conversation (the dispatcher is on at once) and the window opens it there. It has one already: that key, nothing sent |

M5 PR2 adds the person's own chat moves (`src-tauri/src/commands/chat_move_api.rs`, main window only, each answering a
`ChatMoveResult` `{key, outcome, to, message, continued}`, `outcome` one of `moved`, `kept`, `waiting`, `refused`, `busy`
(the move is being sent by another call right now; nothing was done); the controls are in
[Moving a chat](#moving-a-chat-m5-ui-2026-11-15)):

| Command | Tier | What it does |
|---|---|---|
| `chat_move_to_runner(key, expect?)` | user-only | Move to runner: the paired Mac `~/.config/hollow/slot-remote`'s alias reaches, else the only paired Mac (several with none matching is refused, naming the alias and the aliases compared); `expect` (the window's `moveTo`) refuses, moving nothing, when the pick is another Mac now; it must answer within 2 s and list `moves`; a running turn is asked to stop at its next safe point and stopped outright after 30 s; the chat is sent (`chat_move_send`) and, if a turn was running, gets one continuation there |
| `chat_move_keep_here(key)` | user-only | Keep it here on a `leaving` chat: the move's `DELETE` settles it at once when that Mac answers, else the move waits `aborting` for the next contact |
| `chat_move_release_here(key)` | user-only | Release here, after the window's confirm, on a chat stuck `leaving`: `chat_move::force_release`; that Mac may keep a copy |
| `chat_continue_offline(key)` | user-only | M5 PR3: Continue here offline on a chat that moved to another Mac (`away`): a new chat of this Mac's (`forkKind` `offline`) with this Mac's copy of its turns; it syncs there on the next contact (`chat_offline`, [Offline sync](../phone-api/machines.md#offline-sync-m5-pr3)). Answers the new chat's key, the same one while it hasn't synced |
| `chat_offline_rows()` | plumbing | the offline UI's read of this Mac's offline copies (`chat_offline::offline_rows`): `OfflineRow {offlineKey, key, peer, state, note, partly}`, `state` `open`, `appended`, `forked` or `kept`, `note` the kept reason, `partly` an open copy part of whose turns synced already (slices). Read again on the `chat-offline-changed` event (a sync folded in, a copy kept here) |
| `chat_move_homes()` | plumbing | the M5 UI's read of this Mac's home records (`chat_move::home_rows`): `HomeRow {key, state, peer, at, waiting}`, `state` `leaving`, `away` or `arrived`, `waiting` a `leaving` chat whose move is `aborting` after Keep it here |

When this Mac sleeps, running chats move without a command only when the person turned that on (`chat_move_trigger`,
setting `moveOnSleep`, absent or unreadable = off) and the runner is paired: see
[Moving a chat here](../phone-api/machines.md#moving-a-chat-here-m5). Only the runner takes those chats: the paired Mac
whose aliases include `~/.config/hollow/slot-remote`'s (the runner account's alias may differ from the one a Hollow pairing
used, so the person's own Move to runner also takes the only paired Mac). Forget this Mac releases the chats still moving to it (`leaving`,
`aborting`), even when unpairing fails; a chat that already moved there stays `away`, "moved to a Mac you forgot"
(it runs there; the chat's notice and its Quick chats badge say so).

None of these is an agent action. The reads are plumbing, not `read` actions: an action's answer would put a
remote chat's words into a transcript on this Mac. The writes are user-only with `COO_EXCEPTIONS` reasons. The one
agent path to another Mac is M3a's dispatcher COO: `sessions.route_to_runner` (Write with a card that asks as `sessions.start` does, this Mac's COO only) sends text
to the runner COO's chat or a lead's there through `Hub::runner_coo` and `Hub::send`, and returns no remote words
([COO.md](../COO.md#the-dispatcher-coo-m3a-2026-11-15)). Its row's chip (`RouteChip.tsx`) opens that chat there, and
`Hub::follow_routed` watches the Mac's list stream (states only) for one notification when the routed turn ends; the waiting turns are kept in `routed-turns.json` so a restart resumes them (`routed_store.rs`, [COO.md](../COO.md#the-dispatcher-coo-m3a-2026-11-15)).

## The hub

`Hub::for_app()` is process-wide: one `Machines` registry (`<support>/Peers/machines.json`) and, per paired Mac, one
`Peer` holding its one `Tunnel` (pairing borrows it through `peer_client::TunnelFor`, so a Mac never has two ssh
children), a client (its token read from the Keychain once, dropped on a 401, a new pairing or Forget), its state and
at most two `Follow`s. A peer is never replaced and its tunnel's port never moves: only a pairing changes its aliases
(a call holding an older record never puts old ones back), and a pairing at another port than the tunnel's is refused
("Forget that Mac in Settings first"), before ssh runs when the alias is one of that tunnel's, else once the code names
the Mac (an unused code there just expires). A pairing clears that Mac's backoff right before its own opening (it is
the person's own try); clearing never waits for an opening in progress, and neither does asking for the alias. A
pairing again tears the old streams and client down only once it succeeded, so a wrong code leaves a working pairing
alone; the streams that ran start again with the new token (one that can't is logged, and the window watches again on
the `peer-machines-changed` every pairing sends, done or failed). Each stream has its own id, and its frames and
errors count only while it is the one in its slot, so a stream unwatched, replaced or dropped by a pairing says
nothing late. Only Forget drops a peer, and a call that raced a Forget drops the peer it made
(the record is read again after). Starting a watch checks and fills its slot under one lock. Every call notes how it went: an answer is `online`, a tunnel or socket failure `offline` with
ssh's sentence, a 401 `pairing-needed` (and the record's `tokenRevoked`), a newer peer API `update-needed`; while a
tunnel opens the state is `connecting`. An error the Mac answered (busy, not found) leaves it `online`; one made on
this Mac (Keychain, a bad id) changes nothing. Before the first call a Mac is `offline` with `checkedAt` null. A sweep
every minute closes a tunnel nobody used for 10 minutes whose Mac has no stream open.

**Events,** through `Core::emit`:

- `peer-machines-changed`: the whole `PeerMachinesView`, whenever a Mac's state or sentence changes.
- `peer-chat-event` `{machineId, key, kind, data}` (`key` null for the list's stream): `kind` is the frame's name
  (`live`, `append`, `state`, `turn`, `sessions`) with its data as sent, or `detail` (`PeerChatDetail`, after a
  reconnect, a turn's end or an append that didn't follow the live view held), `list` (`PeerSessionsPage`, after a
  reconnect or a `sessions` frame, at most once per 2 s), `connected` `{reconnected}`, `disconnected`
  `{message, retryInMs}` (the Mac turns `offline`) or `ended` (`PeerFailure`; no reconnect helps).

## Stores (`src/store/`)

- `peerMerge.ts` (pure): `mergeChatRows(local, peers, {thisMac, knownWorkspaces, now})`. A row's id is
  `<home machineId>:<key>`, so the same key on two Macs is two rows. Newest `lastActivityAt` first across Macs. A
  remote row whose workspace id exists here has `placement: 'workspace'` (that project's list), else `'machine'`
  (under its Mac: the strip's popover, Library's "On <Mac>"). A Mac not `online` keeps its rows `stale`; 24 hours
  after they were fetched they are dropped; `pairing-needed` hides them. `rowsForWorkspace`, `rowsForMachine`,
  `runningOn` (the strip's count). M5: a moved chat shows one row, at its new home. A row whose `movedTo` (a local
  row's from this Mac's `away` home record; a remote row's from the wire) names a Mac that lists the same key is
  hidden; with no such row there (that Mac offline past a day, or forgotten) it stays, with `movedTo` set. The new
  home's row carries `movedFrom`. Quick chats builds its rows with it (local rows' `movedTo` from `chatHomes`).
- `peerSessions.ts`: the Macs and their rows in memory (`followPeerSessions()` while something shows them; it reads
  the Macs without `check`, so a mount never skips a tunnel's backoff, then fetches each list and follows each list
  stream; a load that ends after its last reader left starts nothing; one list fetch per Mac at a time, a request during it
  fetching once more after, and a failure from a fetch that started before the Mac was paired again dropped (its 401 is
  the old token's); every `peer-machines-changed` watches each paired Mac's list again, a no-op while it runs; each
  Mac's watch and unwatch go out in order, so an unwatch never overtakes its watch). Reducers: a fresh list (dropped for a Mac not in
  the last list of Macs), a failed fetch (offline keeps the rows stale, pairing needed drops them, an error the Mac
  answered marks it online and keeps the rows and the error's text), a new machines view (a forgotten Mac's rows go; a Mac that came back online keeps its old
  rows stale until its list is fetched again, which the store does at once).
- `peerChat.ts`: the open remote chat (`openRemoteChat`, `loadEarlierRemoteTurns`, `closeRemoteChat`, `sendRemote`,
  `stopRemote`, `answerRemote`). `foldPeerChat` applies the contract's client rules: `live` replaces the view (null
  clears it), `append` adds only to the view with its `ord` and `gen` (a mismatch is dropped; the Rust follower
  refetches and a `detail` follows), `turn` drops the view, `detail` replaces the newest turns and keeps older pages.
  `wireQuestionsToCard` renders the home's card with the existing `QuestionCard` (every card arrives as questions; a
  closed one is `stale`), `cardAnswerToWire` turns its answer into the route's `answers`. A `question-closed` answer
  reloads the chat. Closing the view drops its turns and stops its stream.

## The UI (PR 4)

Every piece is lazy; the shell pays Rail's one `lazy()` line and one launch call (`peer_machines`, counted in
`tests/smoke/launch-ipc.spec.ts`). Styles: `src/styles/peer-machines.css`, imported by those components only.

- **Machine strip** (`src/components/RailMachines.tsx`, in `Rail.tsx` above Shared tools): rendered only once a Mac is
  paired. This Mac first (not a button), then each Mac: status dot, name, short state, a count chip of its running
  chats. Words and clicks come from `src/components/machineStrip.ts` (pure, `machineStrip.test.ts`): `online`
  "Online" (the label says "N running"), `connecting` "Connecting…", `offline` "Offline · 4 min ago" or, while
  `checkedAt` is null, "Not checked yet", `pairing-needed` "Pair again", `update-needed` "Needs an update". A click
  opens the Mac's popover (its 8 newest chats, stale and dimmed while it can't be reached, ssh's sentence, Retry =
  `peer_machines(check)` then the list, Cancel while it is connecting, and **Open all in Library**, which opens
  Library, Conversations on "On <Mac>"; Retry and Cancel are `MachineActions.tsx`, shared with Library); Pair again opens Settings, Devices with Pair a Mac filled
  (`src/store/peerUi.ts` carries the alias). Collapsed sidebar: one glyph per Mac with its dot. At launch the strip reads only
  `peer_machines` (the local record, no ssh; one retry after 3 s if it fails); once a Mac is paired it starts `followPeerSessions()` 1.2 s later,
  which opens the tunnels and lists, and stops when none is left. A shared 30 s clock (`peerNow` in `peerUi.ts`)
  drives "4 min ago" and the 24 h drop. The right-click menu is `remoteRowMenu.ts`. Right-click on a remote chat (popover, Quick chats): Open, Stop while it runs,
  Copy conversation ID; Rename, Move to project… and Archive listed disabled, "Do this on <Mac>".
- **Badge** (`MachineBadge.tsx`): a laptop glyph (inline SVG, kept out of the shell's icon table) and the Mac name's
  last word ("mini"), named "On Mac mini" (", waiting for your answer" with a dot when `needsAnswer`). Used in the
  popover and Quick chats (another Mac's quick chats, workspace `quick-chats`, list under this Mac's own).
- **A chat on another Mac** (`RemoteChatView.tsx`): opened from the strip into the page pane through a `Portal` into
  `.canvas .page`, over the page, which is inert meanwhile, never through `chatRuntime.ts`; it closes when the person changes page,
  project or chat, when its Mac is forgotten, or on Close or Escape (not while the box holds words). Each open gets a
  token: its stream is watched only if that open is still the one shown once its turns load; watch and unwatch go out
  in order, each failing after 15 s so a hung one never blocks the next, and a failed watch shows the chat offline. A
  first Escape in a box with words clears them, the next closes; focus returns to the Mac's strip row. With no page
  pane there is no layer. An answer fetches the chat again at once, and the card unlocks after 10 s if it never
  closes here. Header: title, badge, "Live from <Mac>", the permission mode read-only, Copy
  conversation ID. Turns render with the transcript's classes: prose through `MarkdownView` (no media; a web link
  opens in the browser, a file link says the file is on the other Mac), code through `CodeBlock`, a canvas fence as
  "Canvas on <Mac>: open it there." Attachments by name. The live view shows its text and tool names. The open card is
  the existing `QuestionCard` (`wireQuestionsToCard`), keyed by card so a closed card is never read. Composer: text
  only, Enter sends, Stop while it runs; an answer holds the card until it closes there; busy keeps the draft and says so; offline or ended disables both with the
  reason (Retry reopens).
- **Library, Conversations** (`ConversationHistory.tsx` with the lazy `RemoteConversations.tsx`, M2 leftovers
  2026-11-15): the list switch reads Hollow, Archived and one "On <Mac>" per paired Mac, named from the registry
  (`libraryMacs`). "On <Mac>" lists every chat of that Mac held in memory (`libraryRows`: newest first, the search box
  filters title and preview, only the project's when the list is one project's), with the Mac's state sentence and
  Retry or Cancel while it is unreachable or connecting. A project's list (its ⋯, Open in Conversation history)
  also lists that project's chats on each paired Mac under the Mac's name, matched by workspace id. A row opens
  the chat over the page as from the strip; right-click is the remote row menu. The strip's request travels in
  `peerUi.ts` (`askLibraryMachine`); a refused navigation drops it.
- **Settings, Devices, Macs** (`src/settings/MachinesPanel.tsx`, lazy under Companion devices). At rest it is one
  row (Macs, the paired count, Check now once a Mac is paired, **Pair a Mac**), a one-line row per paired Mac (name,
  state, Forget with a confirm, and its failure message when it has one) and one closed fold for the server side,
  so Devices keeps to the Settings height rule. Each Mac's closed **Details and SSH aliases** fold (its summary
  counts the aliases and says "no home slug" when there is none) holds the home slug or "No home slug on <Mac>: set
  one there", paired, last seen, version, and the aliases in the order tried with "Try earlier" and Remove (not the last one),
  then an **Add alias** box (another address for the same Mac, tried after the others; hidden at 8). A Mac that is
  connecting has Cancel beside Forget. Pair a Mac opens
  its form under the row (an alias or address, prefilled from `suggestedAlias`: a name, `user@host`, a tailnet name
  or `host:port`, checked first by `store/pairAddress.ts` and again by `parse_pair_address`; one `peer_pair` call; its
  failure is an alert). **Cancel** stays clickable while a pairing runs: `peer_pair_cancel` stops it and the form
  closes with nothing kept (2026-11-18, two-Macs M-UI). The server fold (`phone_access_status`; group "Macs that open this Mac's chats") reads "Let another Mac
  connect" and its state (no Mac paired, N paired, or the open code's end time) and holds **Let another Mac
  connect** (`phone_access_machine_pairing_start` / `_cancel`, the code for 10 minutes) and the Macs paired to this
  one, each with Revoke (`phone_access_revoke`).
- **Mock** (`src/mockPeers.ts`): `localStorage.mockPeers` = `demo` (a Mac mini online: a running chat, one with an
  open card, one with a canvas) or a JSON seed; the live object `window.__mockPeers` stands in for the other Mac in
  `tests/smoke/peer-machines.spec.ts` and `tests/smoke/two-macs-ui.spec.ts` (pairing Cancel and address entry, route chip, Start COO, Library "On Mac mini" and its offline notice; `pairHold`, `pairCalls`, `cooStarted`) (strip states, popover, Retry, quick chats, read live with appends, send,
  busy, stop, offline, the card answered at its home, the canvas placeholder, Settings). Route ledger rows
  `conversation.remote` (seed `paired-mac`) and `settings.pair-mac`; right-click parity in `context-menus.spec.ts`.

## Moving a chat (M5 UI, 2026-11-15)

The controls on M5 PR2's commands. Words: `src/store/chatMoveWords.ts` (pure, `chatMoveWords.test.ts`); actions:
`src/store/chatMoves.ts` (lazy); the shell keeps two signals, `src/store/moveFlags.ts` (`moveRunner`, the name of
`moveTo`'s Mac, and `chatHomes`, filled by `peerSessions`' machines view and by `loadChatHomes`). The homes read
(`chat_move_homes`) runs only while a Mac is paired or the last read found records (`localStorage`
`hollow.chatHomes`), so a plain launch makes no extra call; while any chat is `leaving` it is read again every 30 s.

- **Move to runner** (one at a time per chat: `movingKeys`, the menu reads "(moving)" meanwhile): the conversation menu (`workspaceMenu.ts`: the ⋯ beside an open chat's name, a Quick chats
  row's right-click, Library, Conversations) lists it while `moveTo` is set, disabled "(moving)", "(moved)" or
  "(archived)". It asks first (where, that the runner's own logins pay, that a running turn stops at a safe point,
  that files stay here), shows "Moving … to <Mac>…" while it runs (up to about 30 s), then the result's sentence:
  moved (with Open, the chat at its new home), waiting, refused (red, with the reason) or busy.
- **The chat's notice** (`ChatMoveNotice.tsx`, mounted by `ChatBody` only for a chat with a home record, above the
  composer): moving ("runs on neither Mac until <Mac> answers") and waiting (after Keep it here) with **Keep it here**
  and **Release here…** (a confirm: it runs here again now, and if that Mac already took it, that Mac keeps its copy
  too); moved ("this copy is read-only", **Open on <Mac>**); moved to a Mac you forgot (no paired Mac by that id); moved
  here from <Mac>.
- **Forget this Mac** reads the homes first and its confirm names the chats still moving there that it releases (at
  most three titles, then "and N more") and how many that moved there stay there.
- **Settings, Devices, Macs**: **Move running chats to the runner when this Mac sleeps** (`MoveOnSleepRow.tsx`, a
  switch under the paired Macs), shown while `sleepTo` is set or the setting is on; its hint names the Mac and says
  its logins pay, the rest is behind More. It saves `moveOnSleep` true or false explicitly (a save without it keeps
  the old choice). The Settings height fixture pairs no Mac, so the row adds nothing there.
- **Badges**: a remote row with `movedFrom` reads "mini · moved" ("On Mac mini, moved there from another Mac"); this
  Mac's row of an `away` chat its new home doesn't list reads "moved" ("Moved to Mac mini", or "Moved to a Mac you
  forgot"). Quick chats hides this Mac's row while the new home lists the chat.
- **Mock**: `chat_move_*` in `src/mockPeers.ts` (`homes`, `nextMove`, `titles`, `moves`); the demo Mac lists `moves`.
  Route ledger rows `conversation.move-to-runner`, `conversation.move-keep` (seed `moving-chat`: the next move answers
  waiting) and `settings.move-on-sleep`.

**Offline copies (M5 offline UI, 2026-11-15).** Words: `src/store/chatOfflineWords.ts` (pure, tested); actions
`src/store/chatOffline.ts` (lazy: `continueOffline`, `loadOfflineRows`, read after a homes read that found an
`away` chat or once any copy is known, when a copy opens, and on `chat-offline-changed`); `src/store/offlineFlags.ts` holds the rows, so `ChatBody` also mounts the notice on a copy, which has no
home record. A moved chat's notice adds **Continue here offline…** (a confirm: a copy here for while that Mac is out
of reach, its new turns sync there into the chat or as a chat of their own), or **Open offline copy** while one is
open, and how its last copy went (synced into it, became its own chat there, or stayed here with the reason). The
copy's notice carries the **offline copy** badge and says it syncs once that Mac answers, that it became its own chat
there, or that it was kept here and why (`chat_offline.note`), with **Open the original**; a fork another Mac's sync
made here (`forkKind` `offline`, `arrived`) says so. Quick chats badge a copy (`forkKind` `offline`). Mock:
`offline` in `mockPeers.ts`. Route ledger row `conversation.continue-offline`. Not built: the badge in project lists
and Library (the shell chunk has no room). A copy whose sync goes in slices sends them one after another
(`chat_move_trigger::land_slices`) on Move to runner, at sleep and on the retry, and says moved only once all are there;
when the rest must wait for the next contact the result says so, and a continuation owed to a running copy is dropped
with its own notice.

Not built yet: Bring it back here; the moved badge in project lists and Library, Conversations (they show the local
copy, whose notice says where it went); the failed-move notice in the chat (the backend logs it and notifies).

Not built yet (M2): remote rows in project chat lists and Library, Conversations ("On <Mac>" filter); the popover's
Open all in Library; a Cancel for a tunnel that is connecting (no backend command); reordering aliases other than
moving one earlier, and adding one.
