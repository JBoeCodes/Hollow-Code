# Siri, Shortcuts and the New chat control (phone API v1.2)

2026-11-07, lane I4 of the overnight build. Part of the [phone chat API](../PHONE_CHAT_API.md), v1.2 additions. The
sentences and requests are fixed in `docs/phone-api-fixtures/app-intents/sentences.json`.

Three App Intents and one Control Center control for the Hollow iPhone app. **There is no new Hollow route**: the
intents use `POST /v1/sessions` (v1), `GET /v1/waiting` (v1.1), `GET /v1/sessions` and `GET /v1/workspaces` (v1), so
this file describes the app's side only. It lives in `hollow-companion`; Hollow's copy of the v1.2 docs links here.

## What is offered

| Name | Siri says | Does | Result |
|---|---|---|---|
| **Ask Hollow** | "Ask Hollow", "Ask Hollow something", "Start a chat in Hollow" | Starts a new chat with a message (Siri asks "What do you want to ask Hollow?"), in a workspace if one is named, else Quick Chats, on the default agent | "Started a chat in Hollow." or "Started a chat in *workspace*." The app is not opened |
| **What's waiting on me** | "What's waiting on me in Hollow", "What is Hollow waiting on", "Check Hollow" | Asks Hollow which chats have a question waiting | A sentence (below) and, for Shortcuts, the count as a number |
| **Open a chat** | "Open a chat in Hollow" | Opens Hollow on one chat (Siri asks which; the newest chats are offered) | The app comes forward on that chat |
| **New chat** (control) | Control Center, the Lock Screen, the Action button | Opens Hollow on the New chat sheet | `hollow://new`, the link the New chat widget uses |

All of them also show in the Shortcuts app (Ask Hollow with a Workspace parameter), so they can go on the Action button
or in an automation. Settings → Siri and Shortcuts in the app lists the phrases and links to Shortcuts.

## How Ask Hollow sends

The message is trimmed; a message of only white space is refused ("There was nothing to send to Hollow.") and nothing is
sent. The request is `POST /v1/sessions` with the workspace's id (`quick-chats` when none is named or the id is empty),
the trimmed text, no `agent` (Hollow's default, as the New chat sheet's "Default") and a fresh `clientMessageId` per run.
The chat then runs on the Mac like any chat started from the phone: the widgets and the push notifications see it, and
it is in the chat list. (A Live Activity starts only for a chat the person has open in the app, so none starts for a
chat started by voice until it is opened.)

**Locked.** All three intents require the iPhone to be unlocked (`authenticationPolicy = .requiresAuthentication`; Siri
asks for the passcode or Face ID first). Ask Hollow sends the person's own words to an agent on their Mac under the
pairing token, which is readable on a locked iPhone, and the notification Reply button already refuses a locked iPhone
for the same reason; What's waiting on me and Open a chat read chat titles aloud or list them. The New chat control
only opens the app, which iOS unlocks first.

**Unsure whether it was sent.** When Hollow doesn't answer (the connection fails or times out, the answer can't be
read, or a 5xx), the message may or may not have arrived. Ask Hollow then says "Hollow didn't answer, so I can't tell
whether it got your message. Check Hollow before asking again." and not "can't reach your Mac", because every run has a
new `clientMessageId` (nothing repeats a run), so a second ask would start a second chat. A refusal (a workspace that
is gone, Hollow busy, a bad request) is certain and is said in Hollow's own words.

## What it says

`sentences.json` holds each sentence. "What's waiting on me" counts question cards only (a kind this app doesn't know is
skipped):

| Waiting | Says |
|---|---|
| none | "Nothing is waiting on you." |
| one | "One chat is waiting on you: *title*." |
| two or three | "3 chats are waiting on you: A, B and C." |
| more | "5 chats are waiting on you, including A, B and C." (the first three) |
| Hollow older than v1.1 | "Update Hollow on your Mac to ask what's waiting." |

A chat without a title reads "New chat". For a shortcut that reads the count, a Hollow older than v1.1 is an error with
that sentence, never the number 0 (which would read as "nothing is waiting"). Failures are said in a sentence too: not
paired ("Open Hollow and pair with your Mac first."), turned away by Hollow (401: "Hollow on your Mac turned this iPhone
away. Open Hollow and pair again."), the Mac not answering a read ("Hollow can't reach your Mac. Check that Hollow is
open there, the Mac is awake and on Tailscale."), not knowing whether Ask Hollow's message arrived (above), or Hollow's
own words.

## Where they run

The intents live in the app target (`ios/App/Intents`) and run in the app's own process, so they read the pairing as the
app does (the base URL in the app's preferences, the token in the Keychain) and make no use of the widget snapshot.
`IntentService` (HollowChatKit, Foundation only) holds every request and sentence, with package tests against a stub
transport. The New chat control (`ios/Widgets/NewChatControl.swift`) is a `ControlWidgetButton` over `OpenURLIntent`
for `hollow://new`: the app opens on its Pair screen when there is no pairing, and drops the link (the New chat sheet
does not open after pairing). The entity lookups (the chats and workspaces Siri offers, a saved shortcut's names) wait
4 seconds for the Mac, not the 20 a send gets, and a saved chat or workspace that can't be looked up still works by its
id (Hollow says if it is gone). **Open a chat** says "Open Hollow and pair with your Mac first." when unpaired.

## Privacy

What is said to Siri is handled by Apple's Siri and Shortcuts as for any app. Hollow receives the message (and nothing
else) over the tailnet, as it does from the app's composer. "What's waiting on me" speaks chat titles aloud, and Open a
chat lists them: both need the iPhone unlocked (see Locked above), so a stranger or a colleague can't have Siri read
them. AirPods and CarPlay speak them once it is unlocked, as for any Siri answer.

## Older Hollow, older app

- Hollow v1.0 or v1.1 works: nothing here needs a v1.2 route. v1.0 (no `GET /v1/waiting`) answers "What's waiting on me"
  with "Update Hollow on your Mac to ask what's waiting."
- An older app (builds 1 to 3) has no intents; an unpaired iPhone answers "Open Hollow and pair with your Mac first."

## Not checked on a device

Siri recognition, the Shortcuts app and Control Center can't be driven from a simulator test. The build is checked for the
intents (the app's `Metadata.appintents/extract.actionsdata` lists `AskHollowIntent`, `OpenChatIntent`,
`WaitingOnMeIntent` and the two entities, and three App Shortcuts), the logic by package tests, and Settings → Siri and
Shortcuts by a UI test.

## Fixtures

`docs/phone-api-fixtures/app-intents/sentences.json`: the Ask Hollow requests (trimming, Quick Chats by default, a blank
message sends nothing), the sentences after it, the waiting sentences and the failure sentences. The app's tests check
each against the code.
