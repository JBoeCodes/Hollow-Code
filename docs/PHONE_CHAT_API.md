# Hollow phone chat API (v1.1)

2026-11-04; v1.1 added 2026-11-05 (the [v1.1 section](#v11-2026-11-05): Waiting on you, answering question cards,
push notifications, Runs on). The contract between Hollow on the Mac (the server, `src-tauri`) and the Hollow iPhone
app (`hollow-companion/ios/`, the client). The same file lives in both repositories, with the same fixtures in
`docs/phone-api-fixtures/`. Change it only in both at once, fixtures included.

## v1.2 additions

- [Chat controls](phone-api/chat-controls.md): agent, model, effort and permission choices for a phone send.

## Deviations (server)

Where Hollow's side now differs from the text below (2026-11-04, after the review of `eed10ca3a`; deviations 1 and
2 updated 2026-11-05, after the third review and again after the review of its fixes). Every shape and fixture
stays as it is.

1. **A send runs in the mode the window keeps for the chat**, which can differ from the last run's: the person's
   choice for that chat as Hollow's window keeps it on disk, else Plan when the window keeps no complete record of
   choices (its storage filled up, so it shows Plan for every chat it has no choice for), else the mode its last
   turn recorded, else Edits only (`acceptEdits`). That is the mode the window shows for the chat, with one
   exception, in the safe direction: after a write to the window's storage fails, the window still shows that
   session's choices from memory but reports no choice and Plan for every chat (its disk keeps only the Plan
   fallback), so until a later write saves the whole record again, a phone's send runs in Plan, which only reads,
   even to a chat the window shows in another mode. The window reports each choice, and its whole record when the one
   it keeps on disk changes wholesale. After a launch, before a phone's chat read (`GET /v1/sessions/{key}`; the
   list and the other routes don't ask), send or new chat relies on that record, Hollow asks the window for it
   whole until the window answers: at once, then every 250 ms for 30 seconds. The request that starts those 30
   seconds waits up to 2 seconds for the answer, and so does any request during those 2 seconds; a later request
   goes on at once, and the first request after the 30 seconds starts them again, so a window that can't answer
   costs one wait every 30 seconds. A window that starts listening hears an ask within 250 ms, so the record Hollow
   kept, which may hold a choice the window has lost (cleared web storage, its Plan fallback), is used only while
   the window can't answer yet (still starting up, after that 2-second wait) and in the moment between the window
   starting to listen and its answer arriving: at most 250 ms and the answer's own time. When that mode and the
   last turn's are in different classes (Plan only reads; the other modes act; each of Grok's sandbox profiles is
   its own class), the send starts a fresh native session with Hollow's retained context, as the window does. A
   chat's `permissionMode` (`GET /v1/sessions/{key}`) is that mode. The model stays the one the chat's newest turn
   asked for: the window keeps no model per chat.
2. **Tailnet identity is required.** A request carrying `Tailscale-Funnel-Request` gets 403 `forbidden`, whatever
   else it carries. A request without `Tailscale-User-Login` (Tailscale leaves it out for a tagged node and for
   Funnel) gets 403 `forbidden` unless it came straight from this Mac: a loopback `Host` (`127.0.0.1:47680`,
   `localhost:47680`) and no `X-Forwarded-For` or `X-Forwarded-Host`, which `tailscale serve` sets on everything
   it passes on. This replaces "A request without the header (loopback tests) is judged by its token alone" under
   Who may call, and holds for `health` and `pair` too. The phone, which always comes through `tailscale serve` as
   its person, sees no difference. Never forward the port as raw TCP (`tailscale serve --tcp`): a raw forward
   passes on nothing of the caller (no `Tailscale-User-Login`, no `X-Forwarded-*`), so a request through it can
   name a loopback `Host` and pass as this Mac's own, judged by its token alone. Settings shows any such forward to
   port 47680 (from `tailscale serve status --json`) as a problem, wherever it is. Stop publishing removes each one
   in this Mac's saved serve config (`tailscale serve --tcp=<port> off`, every one tried even when Tailscale refuses
   another, none after a call that Tailscale doesn't answer within 15 seconds). That command reaches only the saved
   config, so for a forward a `tailscale serve` still running in a terminal holds, Settings says to stop that
   command with Control-C there, and for one in a Tailscale service's config it shows the command that removes it
   (`tailscale serve --service=<name> --tcp=<port> off`, with the CLI's full path when no folder of the person's
   PATH holds `tailscale`) for a service named `svc:` and lower-case letters, digits and hyphens, and the same steps
   in words for any other name, which Hollow never puts in a command; Hollow runs neither.
3. **A send to a busy chat is always 409 `busy`.** Hollow never holds a phone's send, so `"state": "held"` and
   `heldId` don't occur (Sending, under `POST /v1/sessions/{key}/messages`): a held send waits where only the window
   that sent it shows it, so a Stop pressed at the Mac would start the phone's message unseen. A chat whose window
   holds a message of its own for after the running turn is busy too, so the phone's never overtakes it. The app's
   busy handling covers it (it keeps the draft and says so).
4. **A live view only for a turn that started while a phone was paired.** Hollow copies the window's chat events
   only while the iPhone app is on and at least one phone is paired (none at all while it is off). Event streams
   opening and closing don't change that, so a running turn stays whole across a stream swap or a while in the
   background. A turn that was already running when the switch went on or the first phone paired has no live view
   until it ends: its `live` is null and no `turn` event comes for it. The app sees the chat `running` from
   `state`, and refetches it when its row changes (`sessions`), as for any chat; the turn after it is followed
   whole. Turning the switch off or revoking the last phone drops every live view.

Where v1.1 differs from the round 5 outline it was written from (2026-11-05; 8 corrected after the v1.1 review):

5. **Runs on is a workspace's setting, not a chat's.** Hollow keeps no Runs on per chat: a chat's turns run where
   its workspace's Runs on says. So `runsOn` on a new chat does what the Runs on control in a new chat's empty state
   does in Hollow's window: it sets the workspace's own Runs on when it differs, and later chats there follow it
   too. `canRunOnMini` is true for vault workspaces as well as build workspaces, because Hollow's Runs on moves a
   vault workspace's own chats too.
6. **Additions the outline didn't place.** The answer route names a card, so a Turn's `questions` gains the card's
   `id`, and Waiting items carry it as `questionId`; each question gains `required` and `allowsOther`, so the app can
   tell a question it may skip, and one that takes the person's own words, before it sends.
7. **"Watching live" is an open event stream for the chat.** Hollow can't tell a stream a suspended app left open
   from one on screen, so the app closes its event streams when it leaves the foreground (and opens them again when
   it comes back); until it does, Hollow sends no push for that chat to that phone.
8. **Which turns push, and a test kind.** `turnFinished` goes only for a turn the person waits on: one they sent,
   from Hollow's window or the phone (a message or a card answer; also a message held for after a turn, a chat they
   resumed after a restart, its continuation on another agent when one runs out, and the steps of a runbook run
   started by hand or from a chat). A turn Hollow or another agent queued pushes nothing when it ends: a night item,
   a COO check-in, wake or swarm wake, the morning review, the weekly update, a wake-up an agent asked for, a
   chat an agent started or messaged, an unattended runbook step. Neither does a turn that was already running when
   the iPhone app was turned on or the first phone paired (as its live view, Deviations 4). A `question` push goes
   for a new pending card whatever started the turn, a night item or a COO check-in included, because a card
   waits on the person (it is in Waiting on you) and the phone's Focus settings handle quiet hours. Settings' Send
   test notification sends `"kind": "test"` with no `sessionKey` and no `thread-id`; a tap on it just opens the app.
   The app treats any `kind` it doesn't know the same way.

## What it is for

The iPhone app lists Hollow's chats, reads a conversation, follows a running turn live, sends a message as a
chat's next turn, starts a chat in a workspace and stops a running turn. Nothing else in v1: no tasks, notes,
settings or approvals. A question card is shown on the phone and answered by typing (or tapping an option, which
fills the composer); the reply is an ordinary message, which Hollow already treats as superseding an open card.
v1.1 adds a Waiting on you list, answering a card as the card's own answer, push notifications and Runs on.

## Transport

- Hollow listens on `127.0.0.1:47680` (HTTP/1.1, plain) only while Settings → Devices → Companion devices → iPhone app is
  on. It never binds another address. If the port is taken, the switch shows that error; there is no fallback port,
  because the Tailscale mapping names this one.
- Tailscale publishes it to the tailnet: `tailscale serve --bg --https=8443 http://127.0.0.1:47680`. The app's base
  URL is `https://<the Mac's MagicDNS name>:8443`; on Alex's Mac that is
  `https://my-mac.tail0000.ts.net:8443`. Tailnet only, never Funnel. Tailscale provides the TLS
  certificate, so the app needs no App Transport Security exception.
- Bodies are UTF-8 JSON (`Content-Type: application/json`), field names camelCase. Times are RFC 3339 strings as
  Hollow stores them (fractional seconds and `+00:00` or `Z`); clients parse both.
- A request body over 256 KiB gets 413, except the two upload routes of v1.2, whose body is a file
  ([attachments](phone-api/attachments.md)). Every response carries `Cache-Control: no-store`.

## Who may call

1. **Host check (DNS rebinding).** Hollow answers only when `Host` is `127.0.0.1:47680`, `localhost:47680`, or
   the Mac's own MagicDNS name with or without `:8443` (from `tailscale status --json`, `Self.DNSName` without its
   trailing dot). Anything else gets 421 `misdirected`.
2. **Tailnet identity.** `tailscale serve` adds `Tailscale-User-Login` to requests from tailnet users. When that
   header is present and is not the Mac's own Tailscale login (`tailscale status --json`: the `LoginName` of the
   Self node's user, here `user@github`), Hollow answers 403 `forbidden`. A request without the header
   (loopback tests) is judged by its token alone.
3. **Device token.** Every route except `GET /v1/health` and `POST /v1/pair` needs `Authorization: Bearer <token>`.
   Missing, unknown or revoked gets 401 `unauthorized`. The token is checked when the headers arrive and again once
   the body has (within 30 s, an upload within 5 minutes), before the route runs: a phone revoked while its body was
   still coming gets the same 401, and its send or file is not taken. A send, new chat, answer or upload that then
   waits its turn (one at a time) checks the phone again once it has it, before anything is queued or saved.
   Every effect checks the phone once more at its commit point, right before it lands and after any wait that can be
   long (the held-send order, a chat check, the task writer's lock, the note writer, a new chat's placement, a card's
   resolve, an upload's save): a phone revoked meanwhile gets the same 401 and nothing lands. After a revoke,
   `DELETE /v1/push`, the Live Activity routes and every other call get 401, not a 204, and a revoked id is never
   live again (a new pairing never reuses it).

## Pairing

- In Hollow, Settings → Devices → Companion devices → iPhone app → **Pair an iPhone** opens a pairing window: a code of
  8 characters from `23456789ABCDEFGHJKMNPQRSTUVWXYZ`, shown as `XXXX-XXXX`, good for 10 minutes or one use, and a
  QR code of the pairing link. One window at a time (a new one closes the old). Five wrong codes close it.
- Pairing link: `hollow-companion://pair?url=<percent-encoded base URL>&code=<code, no dash>&mac=<percent-encoded
  Mac name>`. The iPhone's Camera opens the app with it. The app also takes the URL and code typed by hand.
- `POST /v1/pair` with `{"code", "deviceName", "deviceModel"}` (fixture `pair-request.json`). The code is compared
  ignoring case, spaces and dashes. `deviceName` 1 to 80 characters; `deviceModel` up to 80, may be absent.
  - 200: `{"token", "deviceId", "deviceName", "macName", "api": 1}` (fixture `pair-response.json`).
  - 403 `pairing_closed`: no open window, expired, used, or closed after five wrong codes.
  - 403 `wrong_code`, with `"attemptsLeft": n` inside `error` (fixture `error-wrong-code.json`).
  - 400 `bad_request`: missing or invalid fields.
- The token is 32 random bytes, base64url without padding (43 characters). Hollow keeps only its SHA-256 (hex),
  with the device id (`ph-` and 8 hex), name, model, paired time and last-seen time, in app data (never the vault),
  file mode 0600. Revoking the phone in Settings removes the record: its next request gets 401, and its open event
  streams close within a second.
- The app keeps the token in the Keychain (this device only, available after first unlock) and the base URL,
  device id and Mac name in its preferences.

## Errors

Every error is `{"error": {"code": "...", "message": "..."}}` with the status below. `message` is one plain
sentence the app may show as it is (fixture `error-busy.json`).

| code | status | when |
|---|---|---|
| `bad_request` | 400 | malformed JSON, a missing or invalid field |
| `unauthorized` | 401 | no token, an unknown one, or a revoked one |
| `forbidden` | 403 | another tailnet user (`Tailscale-User-Login`) |
| `pairing_closed` | 403 | pairing with no open window |
| `wrong_code` | 403 | pairing with the wrong code (`attemptsLeft`) |
| `not_found` | 404 | unknown route, chat or workspace |
| `busy` | 409 | the chat already has a queued or running turn (see Sending) |
| `question_closed` | 409 | v1.1: answering a card that was answered, dismissed, replaced or is gone |
| `too_large` | 413 | body over 256 KiB (a file over 25 MiB on an upload route), or text over its limit |
| `misdirected` | 421 | the Host check failed |
| `unavailable` | 503 | Hollow can't take turns now (another Hollow process owns the run queue, setup mode) |
| `internal` | 500 | anything else |

## Shapes

### Session

```json
{
  "key": "s-20261104-6e772c5f",
  "title": "Release notes for 5.2",
  "agent": "claude",
  "workspace": {"id": "hollow-companion-app-cdc399a4852fa14c", "title": "Hollow Companion App", "kind": "vault"},
  "createdAt": "2026-11-04T16:07:11.658361+00:00",
  "lastActivityAt": "2026-11-04T16:18:09.702896+00:00",
  "turnCount": 3,
  "state": "running",
  "needsAnswer": false,
  "preview": "Drafting the release notes now.",
  "runsOn": "this-mac"
}
```

- `title`: null until Hollow has one (the app shows "New chat").
- `workspace`: null for a chat in the vault root. `kind` is `vault`, `build`, `project`, `notion` (legacy), `quick` or
  `main` (Hollow's own workspace kinds).
- `state`: `idle`, `queued` (a turn truly can't start yet: a limit, memory, another run's folder or the vault,
  or another Hollow process runs the queue) or `running`. Since 2026-11-17 `running` also covers a turn Hollow is
  preparing (claimed, or queued with nothing holding it back), the window's Working; the wire has no word for it.
- `needsAnswer`: an open question card in this chat waits for the person.
- `preview`: the newest answer (else the newest prompt) as plain text, Hollow's fences removed, whitespace
  collapsed, at most 160 characters; null when there is nothing.
- `runsOn` (v1.1): where the chat's next turn runs, `this-mac` or `mac-mini` (the runner; see [Runs on](#runs-on)).

### Turn

```json
{
  "id": "s-20261104-6e772c5f-1-assistant",
  "ord": 1,
  "role": "assistant",
  "text": "Three things changed: ...",
  "agent": "claude",
  "startedAt": "2026-11-04T16:07:11.700000+00:00",
  "status": "success",
  "error": null,
  "truncated": false,
  "tools": [{"id": "toolu_01", "name": "Bash", "status": "done", "detail": "List merged PRs since 5.1"}],
  "questions": null
}
```

- A prompt and its answer are two turns with the same `ord`: `role: "user"` first, then `role: "assistant"`.
  The ids are Hollow's own (`<key>-<ord>-user`, `<key>-<ord>-assistant`).
- `text`: the stored text with Hollow's fences (`hollow-canvas`, `hollow-next`, `hollow-questions`) left in; the app
  handles them. Over 200,000 characters it is cut at a character boundary and `truncated` is true.
- `status`: as Hollow stores it: `running`, `success`, `error`, `cancelled`, or null. The app treats any other value
  as finished.
- `error`: a short message when the turn failed, else null.
- `tools`: assistant turns only (user turns `[]`), in order, at most 200. `status` is `running`, `done` or `error`.
  `detail`: at most 120 characters saying what the call did, taken from the input's `description`, then
  `file_path` or `path`, then the first line of `command`, then `query`, then `url`; else null. No other input
  and no results: they can be large and sensitive.
- `questions`: the question card this turn posted, else null:
  `{"id", "state": "pending" | "closed", "items": [{"id", "header", "prompt", "mode", "required", "allowsOther",
  "options": [{"id", "label", "description", "recommended"}]}]}`. `mode` is Hollow's answer mode for the question:
  exactly `single`, `multi` or `text` (the app treats any other value like `single`); the app shows options as
  buttons only while `pending`. In v1 a tap fills the composer; with v1.1 the app answers the card itself
  ([Answering a question card](#post-v1sessionskeyquestionsquestionidanswer)). v1.1 adds the card's `id` (`qi-` and
  16 hex, the route's `questionId`), `required` (a null answer to it is refused) and `allowsOther` (a `single` or
  `multi` question also takes the person's own words).

### Live

The part of a running turn the transcript doesn't hold yet. Hollow stores an answer's prose only when its turn
ends, so it keeps, in memory, per running turn, the answer text its own window shows so far and the tool calls
seen (built from the same `chat-event`s the window draws from, the same way).

```json
{
  "key": "s-20261104-6e772c5f",
  "ord": 3,
  "gen": 1,
  "startedAt": "2026-11-04T16:18:09.702896+00:00",
  "text": "Drafting the release notes now.",
  "truncated": false,
  "tools": [{"id": "toolu_04", "name": "Write", "status": "running", "detail": "outputs/release-notes-5.2.md"}]
}
```

- Only while a turn runs. `gen` starts at 1 and goes up whenever the text is replaced rather than appended to.
- At most 256 KiB of text; past that Hollow stops adding and sets `truncated`.
- For the running `ord`, the app shows `live.text` and `live.tools` in place of that ord's assistant turn.

## Routes

### `GET /v1/health`

No auth. 200 `{"ok": true, "app": "hollow", "api": 1}` (fixture `health.json`). Nothing else: no Mac name, no
version.

### `GET /v1/me`

200 `{"deviceId", "deviceName", "macName", "api": 1, "runner"}` (fixture `me.json`). Also how the app checks its
token. `runner` is v1.1's (see [Runs on](#runs-on)).

### `GET /v1/sessions?limit=50&cursor=…`

200 `{"sessions": [Session], "nextCursor": "…" | null}` (fixture `sessions.json`).

- Chats that aren't archived, newest `lastActivityAt` first: every chat Hollow's own lists show (workspace chats,
  Quick chats, the COO, chats agents started).
- **One COO conversation (2026-11-07):** the COO is listed once, by its current conversation (its
  newest, `chat_chain::list_filter`); its earlier conversations aren't listed. `GET /v1/sessions/{key}` still opens
  one by key, and `/v1/search` still finds text in them. The shape is unchanged.
- `limit` 1 to 200, default 50. `cursor` is opaque: the app passes back `nextCursor`.

### `GET /v1/sessions/{key}?limit=20&before=<ord>`

200 `{"session": Session, "permissionMode": "…" | null, "turns": [Turn], "hasMore": bool, "live": Live | null}`
(fixture `session-detail.json`).

- `turns`: the newest `limit` ords (each a user and an assistant turn) below `before` when it is given, oldest
  first. `limit` 1 to 100, default 20. `hasMore`: older ords exist.
- `permissionMode`: the chat's last permission mode as Hollow names it, shown read-only.
- 404 `not_found` for an unknown or archived key.

### `POST /v1/sessions/{key}/messages`

Body `{"text", "clientMessageId"}` (fixture `send-request.json`). 202 `{"accepted": true, "state", "queueId",
"waitingFor"}` (fixture `send-accepted.json`).

- The message becomes the chat's next turn exactly as if typed into Hollow's composer for that chat: its agent and
  model, its last permission mode, its folder, resuming its native session, at depth 0, as the person's own words.
  Hollow's window shows it as it shows a composer send.
- `text`: 1 to 100,000 characters after trimming (400 when empty, 413 when longer).
- `clientMessageId`: a UUID from the app. The same id from the same device within 10 minutes returns the first
  answer and queues nothing, so a retry after a lost reply never sends twice.
- `state`: `queued` or `running` (as for a session's `state`: a turn being prepared is `running`); `waitingFor`:
  Hollow's plain sentence for what a `queued` turn waits on, else null.
- **A chat that continued (2026-11-07):** when `{key}` has a handover successor, or is a COO conversation
  that isn't the newest, the message runs in the chain's current chat instead, and the answer adds
  `"forwardedTo": "<that chat's key>"` (absent otherwise). The app should follow it: open that chat and read its
  events. A busy current chat is 409 `busy`, as for any chat. The same holds for a question card's answer.
- **Sending:** if the chat already has a queued or running turn, Hollow does what its own composer's send does at
  that moment only when it can do so without the window (its backend hold, answering 202 with `"state": "held"`
  and `"heldId"`); otherwise 409 `busy`. The app handles both: `held` shows "Sends when the current turn ends";
  `busy` keeps the draft and says so.

### `POST /v1/sessions`

Body `{"workspaceId", "text", "agent", "clientMessageId", "runsOn"}` (fixture `new-chat-request.json`; `runsOn` is
v1.1's and optional, see [Runs on](#runs-on)). 202 `{"accepted": true, "key", "state", "queueId", "waitingFor"}`
(fixture `new-chat-accepted.json`).

- A new chat in that workspace as Hollow's own New chat there starts it: the workspace's folder, `agent` (null for
  the default agent), that agent's default model, Hollow's default permission mode for a new chat there.
- `workspaceId`: an id from `GET /v1/workspaces`. Unknown: 404.
- `clientMessageId`: as above.

### `POST /v1/sessions/{key}/stop`

No body. 200 `{"stopped": true}`, or `false` when nothing was queued or running (fixture `stop.json`). Does what
the Stop button in Hollow's window does.

### `GET /v1/workspaces`

200 `{"workspaces": [{"id", "title", "kind", "status", "canRunOnMini"}]}` (fixture `workspaces.json`;
`canRunOnMini` is v1.1's, see [Runs on](#runs-on)): what the app's New chat picker offers. Quick chats first (`quick-chats`), then the Main agent (`main`), then the workspaces the Work page
lists as current, most recently changed first, at most 100.

### `GET /v1/events?session=<key>`

A server-sent event stream (fixture `events.txt`). Headers `Content-Type: text/event-stream`,
`Cache-Control: no-store`, `X-Accel-Buffering: no`. Each `data:` is one line of JSON.

- First `event: hello`: `{"api": 1, "build": "<version> <commit>", "events": [...]}`. `api` is the version to check;
  `build` and `events` (the event kinds this stream may carry: `hello`, `live`, `state`, `append`, `turn`,
  `sessions`, `ping`, `resync`, and from B4 `approvals`) were added 2026-11-17 (AG8) and a Hollow before that sends `{"api": 1}` alone. Later fields
  may be added; ignore what you don't read. A client should skip an event kind it doesn't know (a newer Hollow may add
  one) rather than end the stream, and treat an `api` other than its own as an update notice. With `session`, then `event: live` (the Live object, or `null` when
  nothing runs) and `event: state`.
- Then, as things happen:
  - `sessions` `{"keys": [...]}`: list rows changed (a new chat, a new turn, a state or title change). Sent on
    every stream, at most one a second. The app refetches what it shows.
  - For the stream's `session` only:
    - `state` `{"key", "state", "needsAnswer"}`.
    - `live` (a Live object): when a turn starts, when `gen` changes, when its tools change.
    - `append` `{"key", "ord", "gen", "text"}`: text to add to the end of the live text with that `ord` and
      `gen`. At most 8 a second (coalesced). When `ord` or `gen` doesn't match what the app holds, it refetches
      the chat.
    - `turn` `{"key", "ord", "status"}`: the running turn ended. The app refetches the chat (the transcript has
      the answer now) and drops its live view.
  - `ping` `{}` every 15 seconds.
- **Ids and resume (AG5, 2026-11-17).** Every event but `hello` and `resync` has an `id:` line,
  `<epoch>-<chat>-<list>-<bind>` (hex epoch, decimal cursors, hex chat tag), for example `5e2a91c4-14-12-3f2c`. Treat
  it as opaque and keep the newest one per stream. It is a position in Hollow's history of what happened, not a frame
  counter: it never goes down, and two events may share one. A client that reconnects to the same stream (same
  `session`, or none) sends it back as the `Last-Event-ID` header (or `lastEventId` in the query; the header wins).
  - Hollow keeps a short ring in memory, at most 1024 events and 512 KiB in all chats together (oldest dropped
    first; a run of one chat's appends counts as one event). Inside it, `hello` carries `"resumed": true` and the
    stream replays what the client missed and nothing it has: the missed text as one `append`, or a whole `live`
    when the tools changed (it already holds the text after that change, so no `append` repeats it), a `turn` for a
    turn that ended, one `sessions` hint, then the current `state`. No refetch is needed.
  - When the ring can't cover the id, `hello` carries `"resumed": false` and `event: resync`
    `{"reason": "epoch" | "expired" | "chat" | "ahead" | "malformed"}` follows, then the usual `live` and `state`:
    the client refetches the chat (or the list), exactly as it did before ids existed. `epoch` is a restart or a new
    watching period (the iPhone app switched off and on, the last phone revoked), `expired` is an id older than the
    ring, `chat` an id from another stream, `ahead` and `malformed` an id Hollow never gave out. Never keep an old
    id across a `resync` and never treat a stream that ended without a `turn` as a finished turn: refetch.
  - Additive: a client that ignores `id:` lines and never sends `Last-Event-ID` sees exactly the old stream (hello,
    live, state, then events) plus the `resumed` field only when it asks. An older Hollow ignores the header, so the
    client gets no `resumed` and must refetch (the iPhone app compatibility note, below).
- At most 4 open streams per device (a fifth closes the oldest), 16 in all. Revoking the device closes its streams.

## v1.1 (2026-11-05)

Four additions: the Waiting on you list, answering a question card, push notifications and Runs on. Every new route
needs the bearer token and passes the same Host and Tailscale checks as v1 (Who may call, and Deviations 2). No v1
shape changes: v1.1 only adds routes and fields, and `api` stays 1. A Hollow without v1.1 answers its routes with
404 `not_found`; the app then hides those features (and may say Hollow on the Mac needs an update).

Fields v1.1 adds to v1's bodies, each described above where its body is:

- Session (`GET /v1/sessions`, and `session` in `GET /v1/sessions/{key}`): `runsOn`.
- A Turn's `questions`: the card's `id`; each item's `required` and `allowsOther`.
- `GET /v1/workspaces` items: `canRunOnMini`. `GET /v1/me`: `runner`. `POST /v1/sessions`: optional `runsOn`.

### `GET /v1/waiting`

200 `{"items": [WaitingItem]}` (fixture `waiting.json`; from B4 also `kind: "approval"` items, see [approvals.md](phone-api/approvals.md)), newest `since` first: one item for every chat with an open
question card, the same chats whose `needsAnswer` is true and Hollow's own Waiting on you shows. At most 100. The app
refetches it whenever a `sessions` event arrives.

```json
{
  "sessionKey": "s-20261104-e2582579",
  "questionId": "qi-8c4e2b7a1d9f3605",
  "title": null,
  "workspaceTitle": "Quick chats",
  "kind": "question",
  "since": "2026-11-04T15:02:41Z",
  "preview": "Which branch should I start from?"
}
```

- `title`: the chat's title, null until it has one (the app shows "New chat"). `workspaceTitle`: its workspace's
  title, as the Session's `workspace.title`; null for a chat in the vault root.
- `kind`: `question`. Approvals join later as another kind (behind Face ID); the app skips a kind it doesn't know.
- `since`: when the card opened. `preview`: the card's first question as plain text, whitespace collapsed, at most
  160 characters.
- `questionId`: the open card's id, the Turn `questions.id` the answer route names. A chat with more than one open
  card lists its newest. Only cards that can still be answered count: one whose file changed since it was asked
  shows `closed` on its Turn, its chat's `needsAnswer` is false, and the person replies by typing.
- Archived chats never show (archiving closes their cards).

### `POST /v1/sessions/{key}/questions/{questionId}/answer`

Body `{"clientMessageId", "answers": {"<item id>": "<string>" | ["<string>", ...] | null}}` (fixture
`answer-request.json`). 202 `{"accepted": true, "state", "queueId", "waitingFor"}`, a send's acceptance (fixture
`answer-accepted.json`).

- Hollow applies it exactly as the card's Send does in its own window: it checks the card is still open and the
  file it asks about unchanged, checks the answers fit the card, marks the card answered and sends Hollow's own
  answer text (`Answered questions`, then each question on its own line and the answer on the next) as the chat's next
  turn, into the native session that asked when the chat still resumes it, else as an ordinary message. The window
  shows that turn and drops the card, as after its own Send.
- `answers` maps each item's `id` (`questions.items[].id`) to its answer; an item left out counts as null:
  - `single`: an option's `id`, or the person's own words when `allowsOther`; null for none.
  - `multi`: a list of option ids, plus at most one string of the person's own words when `allowsOther`; null or
    `[]` for none. A lone string counts as a list of one.
  - `text`: the text; null for none.
  - A confirmation card's one item (`confirmation`) takes `accept` or `reject`; a checkbox card's (`checkbox`) a list
    of its option ids; each item of a verdicts card one of its options (`approve`, `reject`, `defer`).
- `clientMessageId`: as for a send. The same id from the same device within 10 minutes returns the first answer and
  sends nothing.
- 404 `not_found`: an unknown or archived chat. 409 `question_closed`: the card was already answered, dismissed,
  replaced by a newer card or a later message, its file changed, or it isn't this chat's (fixture
  `error-question-closed.json`). 409 `busy`: the chat has a queued or running turn, as for a send; the card stays
  open. 400 `bad_request`: the answers don't fit the card, with Hollow's own sentence (for example "`Audience` needs
  an answer."). 503 `unavailable` as for a send.
- When the answer turn can't be queued after all, the card is put back open, as the window puts it back, and the
  error is the send's.

### Push notifications

`POST /v1/push` with `{"token", "environment", "events"}` (fixture `push-request.json`) → 204, no body.
`DELETE /v1/push`, no body → 204, whether or not a token was registered.

- `token`: the APNs device token as hex (32 to 400 characters, even length, either case). `environment`:
  `development` for a build from Xcode, `production` for TestFlight and the App Store. `events`: the pushes this
  phone wants, any of `turnFinished` and `question`; unknown names are ignored (a later event never breaks an older
  app), and an empty list keeps the token but sends nothing. 400 `bad_request` for a missing or invalid token or
  environment.
- One token per phone: a new `POST` replaces it. Hollow keeps it with the phone's record (app data, mode 0600, never
  the vault). `DELETE /v1/push` drops it (Unpair calls it); revoking the phone in Settings drops it with the record.
  When Apple answers that a token is no longer valid (410, or 400 `BadDeviceToken` or `DeviceTokenNotForTopic`),
  Hollow drops it too; the app registers again at its next launch.
- Hollow sends straight to Apple (`api.push.apple.com`, or `api.sandbox.push.apple.com` for `development`) over
  HTTP/2 with token auth: an ES256 JWT signed with a `.p8` key Alex creates (team `TEAM000000`), topic
  `com.hollow.app.companion`, `apns-push-type: alert`, `apns-priority: 10`. The key lives in the Mac's Keychain;
  Settings → Devices → Companion devices → iPhone app imports it, shows Push as ready, no key, or its last error, and sends
  a test notification. With no key Hollow sends nothing and the routes still answer 204.
- The payload (fixture `push-payload.json`):

```json
{
  "aps": {
    "alert": {"title": "Release notes for 5.2", "body": "Drafting the release notes now."},
    "sound": "default",
    "thread-id": "s-20261104-6e772c5f",
    "category": "hollow.turn"
  },
  "sessionKey": "s-20261104-6e772c5f",
  "kind": "turnFinished"
}
```

  `category` is v1.2's (see [Notification actions](phone-api/notification-actions.md)); an app that doesn't know it
  ignores it, and a v1.1 Hollow doesn't send it.

  - `title`: the chat's title, else "New chat"; at most 80 characters.
  - `body`: for `turnFinished`, the chat's `preview` after the turn (else "Finished."), or "Stopped with an error:"
    and its error for a failed turn; for `question`, the card's first question. At most 160 characters.
  - `kind`: `turnFinished` or `question`; a tap opens the chat `sessionKey`. Settings' test notification is `test`,
    with no `sessionKey` or `thread-id` (Deviations 8).
- When: `turnFinished` when a turn the person sent ends as `success` or `error` (not one someone stopped, and never
  a turn Hollow or another agent queued, Deviations 8); `question` when a question card opens, whatever started its
  turn. Only for chats `GET /v1/sessions` lists. A phone that asked for both gets a turn that opened a card as its
  `question` push alone.
- Never to a phone watching that chat live: one with an event stream open for it (`GET /v1/events?session=<key>`).
  The app closes its streams when it leaves the foreground (Deviations 7).
- A push that fails is logged on the Mac (never with its token) and never holds up a turn. Hollow sends none again,
  but one Apple refused because its JWT expired (403 `ExpiredProviderToken`) goes once more with a fresh JWT. A test
  notification goes to every phone with a token.

### Runs on

Where a chat's agent runs: this Mac or the remote runner (Hollow's Runs on; on Alex's Mac the Mac mini). In
Hollow it is a workspace's setting, not a chat's (Deviations 5): a build workspace's own Runs on, whose Default
follows Settings → Devices → Runs on, or a vault workspace's own (This Mac unless it is set). Only Claude and Codex
turns move; the COO, Chats, Antigravity, Grok and self-hosted models always run on this Mac.

- `POST /v1/sessions` takes an optional `"runsOn": "this-mac" | "runner" | "mac-mini"` (`runner` and `mac-mini`
  are the same choice, M6 2026-11-15); absent or null keeps Hollow's own
  choice for that workspace. Hollow does what the Runs on control in a new chat's empty state does in its window:
  it sets the workspace's Runs on when the chat would otherwise run elsewhere, then starts the chat, so later chats
  in that workspace run there too until it is changed. 400 `bad_request` for the runner where the workspace's
  `canRunOnMini` is false or the agent isn't Claude or Codex.
- Session `runsOn`: where the chat's next turn from the person runs (a send from the phone or the window):
  `mac-mini` or `this-mac`; null when Hollow can't tell. A send to an existing chat follows it. Answers keep
  saying `mac-mini` for the runner in v1.1, though Hollow itself now saves it as `runner`, so the shipped iPhone
  app reads them unchanged; a client should treat `runner` the same if a later version sends it.
- `canRunOnMini` on `GET /v1/workspaces` items: true for a build or vault workspace while a runner is set up on this
  Mac; false for Chats, the COO and every workspace when none is.
- `runner` on `GET /v1/me`: null when no runner is set up, else `{"name", "available"}`. `name`: the runner
  machine's name from Hollow's last check of it, else its SSH host. `available`: its last check said it takes work
  (Settings → Devices → Remote runner shows the same). Hollow checks again in the background when that answer is over
  five minutes old, and the route never waits for a check; false until one has answered.

## v1.2 additions

2026-11-07: each feature has its own contract file in `docs/phone-api/` and its fixtures in
`docs/phone-api-fixtures/<feature>/`, the same in both repositories. v1 and v1.1 keep working unchanged, and an older
Hollow answers a v1.2 route 404 `not_found`.

- [Notification actions](phone-api/notification-actions.md): Reply, Open and one-tap answers on Hollow's push notifications.
- [Attachments](phone-api/attachments.md): upload photos and files, send them with a message, show them in the transcript.
- [Canvases](phone-api/canvases.md): the canvas kit, a chat's canvases, one canvas, answering an ask-canvas, and a
  workspace's saved canvases; error `canvas_closed`.
- [Tasks](phone-api/tasks.md): list the tasks as the Tasks page groups them, add one, finish or reopen one, and
  reschedule one; errors `task_changed` and `busy`.
- [Notes](phone-api/notes.md): save a quick note as a new file under `notes/`, never over an existing one; a retry with
  the same key is answered `duplicate` and makes no second file; error `busy`.
- [Workspaces and file preview](phone-api/workspaces.md): a read-only list of workspaces (pinned first), one
  workspace's outcome, plan, chats and Results, and a preview of one file.
- [Search](phone-api/search.md): chats (title or the person's own messages) and workspaces, one read-only route.
- [Calendar](phone-api/calendar.md): today's meetings for the Today tab, one read-only route, off unless work data is on.
- [Live Activity](phone-api/live-activity.md): a running turn on the lock screen and in the Dynamic Island, kept current by the app and, with the app closed, by Hollow's Live Activity pushes (`POST` and `DELETE /v1/live-activity`).
- [Widgets](phone-api/widgets.md): Waiting on you, Recent chats and New chat on the Home Screen and Lock Screen, drawn
  from a snapshot the app keeps in the Keychain; a widget refreshes itself with `GET /v1/sessions` and `GET /v1/waiting`
  (no new route; the iPhone app's side only).
- [Siri, Shortcuts and the New chat control](phone-api/app-intents.md): Ask Hollow, What's waiting on me and Open a chat
  for Siri and Shortcuts (all need the iPhone unlocked), and a Control Center control for New chat; no new route.
- [Spotlight, quick actions and workspace links](phone-api/spotlight.md): the newest chats' titles in iOS search, New chat
  and Waiting on you on the Home Screen icon, and `hollow://workspace/<id>`; no new route.
- [Another Mac](phone-api/machines.md) (2026-11-15, peer API 1): Mac-to-Mac pairing and routes; the iPhone app never calls them and no phone shape changes.
- **iPhone app compatibility (AG5, 2026-11-17):** the `id:` lines, the `resumed` field in `hello`, the `resync` event kind and
  `Last-Event-ID` resume are additive and `api` stays 1. An app that ignores `id:` and never sends the header behaves as
  before (refetch after every reconnect); it must already skip an event kind it doesn't know. The app adopts resume by
  keeping the newest `id:` per stream, sending it on reconnect to the same stream, skipping the refetch only when `hello`
  says `resumed: true`, and refetching on anything else (`resumed: false`, `resync`, no `resumed` from an older Hollow).
  The companion repository carries the same change (this file and `docs/phone-api-fixtures/events.txt` are shared).

[Approvals from the phone behind Face ID](phone-api/approvals-design.md) (design) and
[approvals.md](phone-api/approvals.md) (contract): answering the Mac's Allow/Deny cards from the iPhone with Secure
Enclave keys (Allow signs only after Face ID), enrolled with the Mac's code and App Attest. Built so far: enrollment,
`GET /v1/approvals`, the `approval` waiting kind, the `approvals` event and the `approval` push (B4 PR 1), and the
challenge and decision routes with the longer wait (PR 2).

## Fixtures

`docs/phone-api-fixtures/` holds one example of every body above. Hollow's tests serialize its response types for
these values and compare the JSON (`serde_json::Value` equality); the app's tests decode every fixture. When a
shape changes, change the fixture in both repositories in the same change.

v1.1 added `waiting.json`, `answer-request.json`, `answer-accepted.json`, `error-question-closed.json`,
`push-request.json` and `push-payload.json`, and added its fields to `sessions.json` and `session-detail.json`
(`runsOn`; the card's `id`, `required`, `allowsOther`), `workspaces.json` (`canRunOnMini`), `me.json` (`runner`)
and `new-chat-request.json` (`runsOn`).
