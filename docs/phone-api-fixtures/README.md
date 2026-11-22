# Phone chat API fixtures

One example of every request and response body in [../PHONE_CHAT_API.md](../PHONE_CHAT_API.md), plus an event
stream transcript (`events.txt`) and the push notification payload Apple delivers (`push-payload.json`, v1.1). The
same files live in the Hollow repository and in `hollow-companion`; change them in both in the same change.

- Hollow (server): serializing its response types for these values gives exactly this JSON (`serde_json::Value`
  equality); parsing the request fixtures gives the expected requests.
- iPhone app (client): every response fixture decodes, the request fixtures match what it encodes, and its event
  parser reads `events.txt` into the listed events in order.

v1.2 features keep their fixtures in a folder of their own, named like the contract file in
[../phone-api/](../phone-api/) (`notification-actions/` for `notification-actions.md`, `attachments/` for
`attachments.md`, `canvases/` for `canvases.md`, `live-activity/` for `live-activity.md`, `notes/` for `notes.md`,
`widgets/` for `widgets.md`, `app-intents/` for `app-intents.md`, `spotlight/` for `spotlight.md`), beside the contract in `docs/phone-api/<feature>.md`. The tests that list this folder's files look only at the files directly in
it, so a feature's folder is covered by that feature's own tests.

`live-activity/` holds the registration request (`register-request.json`), the three pushes Hollow sends to Apple
(`push-update.json`, `push-waiting.json`, `push-end.json`, exact: Hollow's tests build them at the fixture's
timestamps, the app decodes their `content-state`) and `step-labels.json`, the tool-name-to-step table both sides test.

`canvases/` holds the kit, a chat's and a workspace's canvas lists, one canvas and an ask-canvas, the answer
request, the message Hollow sends for it (`answer-message.txt`), and the `canvas_closed` error. A fixture with a long
string (the kit) is cut short and holds the shape; the rest are exact.

`tasks/` holds the open and finished lists, a new task's request and answer, the status and update requests and
their answers (a repeating task's status answer carries `rolledForwardTo`), and the `task_changed` error. Every body
is exact; the hashes are 64 hex characters of no particular task.

`notes/` holds a quick note's request and its two answers: a new note (`applied`) and the same note again
(`duplicate`, naming the same file). Every body is exact; the time is an example.

`machines/` (2026-11-15, [../phone-api/machines.md](../phone-api/machines.md)) holds the peer API's bodies: another Mac's
pairing request and answer, `GET /v1/machine`'s answer, `GET /v1/machine/usage`'s account snapshot (A2, `usage.json`)
and the 403 a machine token gets off its routes. Every body is exact; the iPhone app never reads them.

`widgets/`, `app-intents/` and `spotlight/` belong to the iPhone app alone (no Hollow route): `widgets/snapshot.json`
is the JSON the app keeps in the Keychain for its widgets, `app-intents/sentences.json` the requests Siri's Ask Hollow
makes and every sentence the intents say, and `spotlight/` the chats the app puts in iOS search (`items.json`) and its
two Home Screen quick actions (`quick-actions.json`). Hollow's tests do not read them; the app's tests check each against
its code. They are here so both repositories hold the same v1.2 contract.
