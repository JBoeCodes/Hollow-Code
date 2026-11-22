# Live Activity (phone API v1.2)

2026-11-07, lane I4 of the overnight build. Part of the [phone chat API](../PHONE_CHAT_API.md), v1.2 additions. The
same file and the same fixtures (`docs/phone-api-fixtures/live-activity/`) live in Hollow and in `hollow-companion`;
change them in both at once.

A running turn shows on the iPhone's lock screen and in the Dynamic Island: the chat, its workspace, a running timer
and what the turn is doing now. It ends with the turn.

Two ways keep it current, and the first needs nothing from Hollow:

1. **The app, while it is open.** The app starts the activity itself (ActivityKit, local) and updates it from the
   chat list and the event stream it already follows (v1 and v1.1). It works against any Hollow.
2. **Hollow, while the app is closed.** The app gives Hollow the activity's push token (the routes below); Hollow
   updates and ends the activity with Apple's Live Activity pushes, when a `.p8` key is imported (Settings →
   Companion devices → iPhone app, as for notifications). With no key Hollow accepts the token and sends nothing; the
   lock screen then shows the last state the app saw, and says "Open Hollow to refresh" once it is stale.

## Routes (new in v1.2)

Both need the bearer token and pass the same Host and Tailscale checks as every route. A Hollow without v1.2 answers
them with 404 `not_found`; the app then keeps the activity local (see Older Hollow below).

### `POST /v1/live-activity`

`{"sessionKey", "token", "environment"}` (fixture `register-request.json`) → 204, no body.

- `sessionKey`: the chat the activity shows. A key that is not a chat `GET /v1/sessions` lists is ignored (still 204),
  so a 404 always means a Hollow without v1.2.
- `token`: the activity's push token as hex (32 to 400 characters, even length, either case), as for `POST /v1/push`.
  `environment`: `development` or `production`, as for `POST /v1/push`. 400 `bad_request` for a missing or invalid
  field.
- One token per phone and chat: a new `POST` for the same chat replaces it. At most 8 chats per phone (a ninth drops
  the oldest).
- Hollow keeps it **in memory only** (never on disk). It is dropped when Hollow sends the `end` push for that chat,
  when `DELETE` says so, when the phone is revoked or unpaired (`DELETE /v1/push`, which Unpair calls, drops them
  too), when Apple answers that the token is no longer valid
  (410, or 400 `BadDeviceToken` or `DeviceTokenNotForTopic`), after 12 hours (an activity's longest life), and when
  Hollow quits. The app registers again at its next launch while an activity still shows. A 410 is the routine answer
  for an activity that already ended (the app ended it first, or the person swiped it away): Hollow drops the token
  without showing an error in Settings; any other refusal shows there as "Last error".

### `DELETE /v1/live-activity?session=<key>`

→ 204 whether or not a token was registered. Without `session`, drops every token this phone registered.

## Hollow (server): the pushes

Straight to Apple over HTTP/2 with the same JWT and key as notifications (`POST /v1/push`'s Hollow section), with
`apns-push-type: liveactivity`, topic `com.hollow.app.companion.push-type.liveactivity`, and `apns-priority: 5` for
a step update, `10` for `waiting` and `end`. Sent only for a chat with a registered token, whoever started its turn.

| When | `aps.event` | `content-state.phase` | `step` |
|---|---|---|---|
| a running turn's step changed (at most one push per chat every 10 seconds; the newest step wins) | `update` | `running` | the step label |
| a question card opened in the chat | `update` | `waiting` | `Waiting for your answer` |
| the card was answered or dismissed and the turn goes on, or the next step began | `update` | `running` | the step label |
| nothing changed for 5 minutes while the turn runs or waits | `update` | the same | the same, with a new `stale-date` (the keep-alive) |
| the turn ended | `end` | `finished`, `failed` (ended with an error) or `stopped` (someone stopped it) | none |

- **How it ended is what Hollow stored for the turn**: `success` is `finished`, `error` is `failed`, `cancelled` and
  `interrupted` are `stopped`. The chat events only decide when the store can't say (the turn still reads as
  running): a refusal ends a turn as an error with no `error` event, and an `error` event can come in a turn that
  went on to succeed, so the events alone would be wrong both ways.
- **`waiting` usually lasts seconds before `end`.** Hollow tells an agent to end its turn right after it posts a
  question card, so the card's turn finishes a moment later and the activity ends as `finished`; the answer then
  starts a new turn, which gets a new activity when the app is open. While a card is still pending, a step update
  does not replace `waiting`.
- **The keep-alive.** `stale-date` is 10 minutes after a push. A step can last longer (a long command), and Hollow
  sends only on a change, so it also sends the unchanged state again 5 minutes after its last push for the chat. The
  open app does the same for any activity it has not updated for 5 minutes, so neither a long step nor a quiet
  one reads "Open Hollow to refresh" while the turn is healthy.
- **An end Apple didn't take is sent again.** When Apple doesn't answer, answers 429 or a 5xx, or refuses Hollow's
  JWT, Hollow keeps the activity's token and sends the end again 20 seconds later, then 40 seconds after that (three
  tries in all). A token Apple calls gone (410, `BadDeviceToken`) is dropped without a retry. A `waiting` or step push
  that fails is not retried by itself; the keep-alive sends the state again.
- **An end is for the activities registered when the turn ended.** Its job can wait behind the turn's notification
  push, and the open app can end the old activity and register the chat's next turn's meanwhile; the end (and each
  re-send) goes only to the registrations that existed when the turn ended, so the next turn's activity is never told
  the previous turn is over and keeps its token.

Payloads (fixtures `push-update.json`, `push-waiting.json`, `push-end.json`):

```json
{
  "aps": {
    "timestamp": 1794027000,
    "event": "update",
    "stale-date": 1794027600,
    "relevance-score": 100,
    "content-state": {"phase": "running", "step": "Editing files"}
  }
}
```

- `timestamp`: seconds since 1970 when Hollow made the push, or one second after the chat's previous push when that
  was in the same second (ActivityKit drops a push that isn't newer than the last one it applied). `stale-date`: `timestamp` plus 600 seconds, on every
  `update`. `end` carries `dismissal-date`: `timestamp` plus 300 seconds instead, so the finished state stays on the
  lock screen for five minutes.
- `content-state` is exactly the app's `TurnActivityState`: `phase` and an optional `step`. Nothing else is in it: no
  chat text, no tool detail.
- **The step label** says what the turn is doing without naming a file, path or command: tool call details (which can
  hold a shell command with a secret in it) never reach Apple. From the newest tool call still running, by its name,
  the first row that matches (fixture `step-labels.json` is the table both sides test):

| Tool name | Step |
|---|---|
| `Read`, `NotebookRead` | `Reading files` |
| `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `apply_patch` | `Editing files` |
| `Bash`, `BashOutput`, `shell`, `exec_command`, `local_shell` | `Running a command` |
| `Grep`, `Glob`, `LS` | `Searching files` |
| `WebFetch`, `WebSearch` | `Searching the web` |
| `Task`, `Agent` | `Running a helper` |
| `TodoWrite`, `update_plan` | `Planning` |
| a name starting `mcp__` | `Using a tool` |
| any other name | `Working` |

  With no tool running: `Writing the answer` when the turn's live text is not empty, else `Working` when a tool call
  was already seen, else `Starting`. A queued turn (the app only; Hollow pushes nothing for a queue): `Waiting in
  line`. A label never exceeds 80 characters.

  **Known limit of the app's step: after 200 tool calls it stops changing.** The step the app shows comes from
  `live.tools` in the phone API's `live` view, which holds at most 200 calls (`live.truncated` is then true). Past
  the 200th call the app's step stays at what the first 200 said (`Writing the answer` and the final phases still
  follow). Hollow's own pushes have no such limit: Hollow follows the turn's tool calls by id itself.

## The app (client)

- **Starts** when the person sends a message from this iPhone and the chat's turn runs or queues, and when the person
  opens a chat whose turn is running. Never for a turn the person did not start or open (a night item, a check-in).
  At most 3 activities show at once (a fourth turn gets none). Settings → Live Activities (on by default) and iOS's
  own switch for the app both must allow it.
- **Attributes** (set at the start, never pushed): `sessionKey`, `title` (the chat's title, else "New chat"),
  `workspace` (its name, none for the vault root or Chats), `startedAt` (the turn's `startedAt` when the app has it,
  else the moment it started). The timer on screen counts from `startedAt` by itself, with no updates.
- **Content state:** `phase` (`running`, `waiting`, `finished`, `failed`, `stopped`) and `step` (at most 80
  characters), as above. The app updates it only when it changed, from the chat list's `state` and `needsAnswer`
  and the live view's tools.
- **Ends** when the chat's list row says the turn is over (the app is open) or when Hollow's `end` push arrives, with
  the final phase and a dismissal five minutes later. Unpairing ends every activity at once. When the chat is not the
  one open, what the app holds of it is from before the turn, so it loads the chat first and ends the activity with
  how the newest answer ended. When the app ends an activity itself it also `DELETE`s that chat's token, so Hollow
  does not push to an activity that is gone.
- **Swiped away.** An activity the person swipes away during a turn is not started again for that turn (the chat is
  still open and busy, which would bring it straight back); the next turn gets one.
- **Push token.** The app asks ActivityKit for the token (`pushType: .token`) and `POST`s each token it receives.
  ActivityKit hands a token out once, so one Hollow did not take (a failed request, the Mac asleep) is kept and
  offered again, no sooner than 30 seconds apart, until Hollow takes it or the activity is gone. 404 means an older
  Hollow: it stops asking until the next pairing, and the activity stays local.
- **Tap** opens the chat: `hollow://chat/<sessionKey>` (the app's second URL scheme, `hollow`, beside the pairing
  link's `hollow-companion`). A key holds only letters, digits, `-`, `_` and `.`; any other link is ignored.
- **Stale.** A local update sets the activity stale in 10 minutes (an update from Hollow too, as above). A stale
  activity says "Open Hollow to refresh" in place of the step.

## Older Hollow, older app

- A Hollow without v1.2 answers the routes with 404: the activity works as in way 1 and the app says nothing is wrong.
  Settings → Live Activities explains: "Updates while the app is open. Update Hollow on your Mac to keep it current
  while the app is closed."
- An older app (builds 1 to 3) never calls the routes.

## Fixtures

`docs/phone-api-fixtures/live-activity/`:

- `register-request.json`: the body of `POST /v1/live-activity`.
- `push-update.json`, `push-waiting.json`, `push-end.json`: the three payloads.
- `step-labels.json`: `[{"tool", "step"}]`, the step table's rows and the expected label for each.
