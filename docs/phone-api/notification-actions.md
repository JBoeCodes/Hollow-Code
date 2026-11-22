# Notification actions (phone API v1.2)

2026-11-07, lane I4 of the overnight build. Part of the [phone chat API](../PHONE_CHAT_API.md), v1.2 additions. The
same file and the same fixtures (`docs/phone-api-fixtures/notification-actions/`) live in Hollow and in
`hollow-companion`; change them in both at once.

A push notification from Hollow gets buttons: **Reply** (type a message into the chat, from the lock screen or a
banner), **Open**, and, for a question card the person can answer with one tap, **one button per option**.

No new route. Replying uses `POST /v1/sessions/{key}/messages` and answering uses
`POST /v1/sessions/{key}/questions/{questionId}/answer`, both v1.1 and unchanged. What is new is what Hollow puts in
the push payload and what the app registers with iOS.

## Hollow (server): the payload

v1.1's payload (`push-payload.json`) gains fields; none is removed or changed. An app that doesn't know them shows the
notification as before (iOS ignores a category it has not registered).

### `turnFinished`

`aps.category` is `hollow.turn` (fixture `push-payload.json`, which now carries it).

### `question`

When the card can be answered with one tap (see below), the payload is (fixture `notification-actions/push-question.json`):

- `aps.category`: `hollow.question`.
- `aps.mutable-content`: `1`, so the app's notification service extension can turn the options into buttons.
- `card`: `{"questionId", "itemId", "options": [{"id", "label"}]}`:
  - `questionId`: the card's id, the answer route's `questionId` (as `GET /v1/waiting` names it).
  - `itemId`: the card's one question's `id`, the key of the answer in the request body.
  - `options`: the question's options in the card's order, each `id` as the card gives it (at most 40 characters)
    and `label` cut to at most 40 characters with a trailing `…` when cut (an empty label is replaced by the id).

A card **can be answered with one tap** when it asks exactly one question (a confirmation card is one, a verdicts card
with one item is one), that question's `mode` is `single`, and it has 2 to 4 options whose ids are usable as they are
(not empty, at most 40 characters, none twice) and whose buttons read differently once the labels are cut as above
(an answer can't be taken back, so two buttons never look alike). A verdicts card's verdict that needs a reason
(`requireReasonOn`) gets no button, because a button sends no reason and Hollow would refuse the answer; with fewer
than two buttons left the card gets none. Any other card gets a `question` push as in v1.1:
no `category`, no `mutable-content`, no `card` (fixture `notification-actions/push-question-plain.json`); a tap opens
the chat.

A `test` notification is unchanged. The payload stays far under APNs's 4096-byte limit (title 80, body 160, four
options of 40 and 40 characters).

**What reaches Apple.** The option labels are the card's own words, and they travel through Apple's push service
like the notification's title and body always have. Nothing else of the card does (no prompt beyond the body's first
question, no paths, no tokens). The only way to stop it is to turn notifications off (Settings in the app, or Hollow's
Settings → Devices → Companion devices → iPhone app).

## The app (client): categories and actions

The app registers the static categories with iOS at every launch (before a button press that started the app in the
background is delivered), and keeps the dynamic ones below that a notification still on the screen uses.

| Category | Action identifier | Title | Kind | Options |
|---|---|---|---|---|
| `hollow.turn` | `hollow.action.reply` | Reply | text input; button "Send", placeholder "Message" | authentication required |
| `hollow.turn` | `hollow.action.open` | Open | opens the app on the chat | foreground |
| `hollow.question` (fallback) | `hollow.action.open` | Open | opens the app on the chat | foreground |
| `hollow.question.<questionId>` | `hollow.action.answer.0` to `.3` | the option's label | answers the card | authentication required |

- A tap on the notification itself (not a button) opens the chat, as in v1.1.
- **Dynamic category.** A `question` push with `mutable-content` reaches the app's notification service extension,
  which reads `card`, adds the category `hollow.question.<questionId>` (one answer action per option, in the card's
  order) to the registered ones and sets it as the notification's category before iOS shows it. The extension never
  removes a category, so two questions arriving together can't drop each other's buttons; it reads the set back,
  writes again (up to three times) when the push's own category isn't there, gives up after 8 seconds and calls iOS's
  handler once. The app drops the categories of notifications no longer on the screen when it starts. If the
  extension fails, can't confirm the category or is out of time, iOS shows the notification with the fallback
  category `hollow.question`, whose only button opens the chat.
- **One try.** A press is sent once: iOS gives an action about 30 seconds and a request waits up to 20, so there is
  no second attempt. The app may have been started in the background just for the press, and uses the saved pairing.
- **Reply.** Sends the typed text to the notification's `sessionKey` as a send (`clientMessageId` a fresh UUID).
  202: the delivered notification is removed. 409 `busy` (a turn is running or queued): nothing is sent, the text is
  kept as the chat's draft (after any draft already there) and the app posts a local notice that says so. Any other
  error: the same, with Hollow's message; when no answer came back, the notice adds that the reply may not have been
  sent. An empty reply sends nothing.
- **Answer.** Sends `{"clientMessageId", "answers": {"<itemId>": "<option id>"}}` to
  `POST /v1/sessions/{sessionKey}/questions/{questionId}/answer`. 202: the delivered notification is removed.
  409 `question_closed` (answered at the Mac or elsewhere, replaced or gone): the app posts a local notice that says
  the question was already answered. 409 `busy` and other errors: a local notice with the reason; the card stays open. No answer came back
  (timeout, 5xx): the notice says Hollow may have taken it ("open the chat to check").
- **Local notice.** A local notification in the chat's thread (`thread-id` the session key) with `kind` `notice`
  and the `sessionKey` in its payload; a tap opens the chat. Its title is the original notification's title.
- **Not paired.** When the app has no pairing (unpaired since), an action posts the notice "Open Hollow to pair this
  iPhone again." and sends nothing. A 401 (Hollow revoked this iPhone) posts "This iPhone was unpaired in Hollow." and
  the app drops its pairing as it does anywhere else. In both cases a typed reply is kept as the chat's draft.
- **The chat's key.** A `sessionKey` that is not a plain key (letters, digits, `-`, `_` and `.`, at most 120
  characters) names no chat: a button press on it does nothing.

## Older Hollow, older app

- An older Hollow sends no `category`: notifications show no buttons and a tap opens the chat. Nothing in the app
  hides or breaks; notification buttons simply don't appear.
- An older app (build 2) ignores `category`, `mutable-content` and `card`.

## Fixtures

`docs/phone-api-fixtures/notification-actions/`:

- `push-question.json`: a `question` push for a card that can be answered with one tap.
- `push-question-plain.json`: a `question` push for a card that can't (two questions).

And `push-payload.json` (v1.1's turn-finished payload) gained `"category": "hollow.turn"`.
