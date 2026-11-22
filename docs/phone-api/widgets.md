# Widgets (phone API v1.2)

2026-11-07, lane I4 of the overnight build. Part of the [phone chat API](../PHONE_CHAT_API.md), v1.2 additions. The
snapshot fixture is `docs/phone-api-fixtures/widgets/snapshot.json`.

Home Screen and Lock Screen widgets for the Hollow iPhone app. **There is no new Hollow route**: the widgets read what
the app already reads, `GET /v1/sessions` (v1) and `GET /v1/waiting` (v1.1), so this file describes the app's side
only. It lives in `hollow-companion`; Hollow's copy of the v1.2 docs links here.

## The widgets

| Widget | Families | Shows | A tap opens |
|---|---|---|---|
| **Waiting on you** | small, medium, Lock Screen circular, rectangular, inline | how many chats have a question card waiting, and (medium, rectangular) which | `hollow://waiting` (the chat list, whose first section is Waiting on you); a row opens `hollow://chat/<key>` |
| **Recent chats** | medium (3), large (6) | the newest chats with their workspace, and whether each is running, queued or waiting on you | a row opens `hollow://chat/<key>`; elsewhere `hollow://waiting` |
| **New chat** | small, Lock Screen circular, rectangular | a plus and "New chat" | `hollow://new` (the New chat sheet) |

Links use the app's URL scheme `hollow` (see [Live Activity](live-activity.md)): `hollow://chat/<sessionKey>`,
`hollow://new`, `hollow://waiting`. Any other `hollow://` link is ignored, and nothing opens until the iPhone is paired.

Settings → Widgets in the app draws the same views from what the app holds now, and says how to add them (touch and
hold the Home Screen, Edit, Add Widget, search Hollow).

## The snapshot

The widgets are drawn from one JSON document (fixture `snapshot.json`), version 1:

| Field | Meaning |
|---|---|
| `version` | `1`. A reader skips a snapshot of another version. |
| `updatedAt` | when it was made, seconds since 1970 |
| `address` | the Mac's address, so a widget can ask Hollow itself; absent when the iPhone isn't paired |
| `macName` | the Mac's name |
| `tokenFingerprint` | which pairing this was built for: the first 8 hex digits of the SHA-256 of the token the iPhone held. A widget uses the snapshot only while the token it reads has this fingerprint (never the token itself) |
| `waitingCount` | how many chats wait on the person (question cards only), which can exceed `waiting`; **absent when Hollow is older than v1.1** (no `GET /v1/waiting`: the count is unknown, which is not zero) |
| `waiting` | up to 5 `{key, title, workspace?, preview?, since?}`: the chat, its workspace, the card's first question (at most 100 characters), when it opened |
| `recent` | up to 6 `{key, title, workspace?, status, at?}`: `status` is `idle`, `running`, `queued` or `waiting` (a card is open) |
| `signedOut` | Hollow refused this iPhone's token: the widgets say "Open Hollow to sign in again." and `waiting`, `recent` and `waitingCount` are emptied (the titles don't stay in the Keychain for a pairing that is over) |

A title is the chat's title or "New chat", cut at 80 characters; line breaks become spaces. It is a few KB at most:
under 4 KB with ASCII titles at every limit, more with scripts that take more bytes per character.

**Where it lives.** One Keychain generic-password item beside the pairing token: service `com.hollow.app.companion`,
account `widget-snapshot`, readable after the first unlock (so a Lock Screen widget has it while the phone is locked),
this device only, never synced. The widget extension names the app's own access group in its entitlements
(`keychain-access-groups: $(AppIdentifierPrefix)com.hollow.app.companion`), the same way the Share extension reads the
pairing. **There is no App Group**: nothing new is registered with Apple.

**A dependency to keep.** The extensions read the app's items only because the app has **no** explicit
`keychain-access-groups` entitlement: its items go in the default group, `<team>.com.hollow.app.companion`, which is the
group the extensions name. If the app ever gets an explicit list, its first entry must stay
`$(AppIdentifierPrefix)com.hollow.app.companion`, or new items (the token, this snapshot) land in another group and the
widgets and the Share extension find nothing. Nothing tests the extension's read; a check on a real iPhone is the only one.

**Who writes it.**

- The app, while it runs: when the chat list, Waiting on you or the pairing changes (changes within a second are one
  write; a write is skipped when the stored item says the same, ignoring the time and each chat's last activity, except
  to renew one that is 5 minutes old), then it asks WidgetKit to reload the widgets. Unpairing, a 401 and pairing again
  delete the snapshot in the same call. Until both the chat list and Waiting on you have answered once since the
  pairing, the app leaves a stored snapshot alone: an empty list it hasn't asked for would read as "All clear".
- The widget itself, when the snapshot is 5 minutes old or older, the iPhone is paired and the token is not refused. It
  reads the token first: no token deletes the snapshot and the widget says to pair; a token whose fingerprint is not the
  snapshot's (paired again with another Mac before the app wrote a new snapshot) shows the same note and sends nothing,
  so the new Mac's token never goes to the old Mac's address; a Keychain error is not "unpaired" (the stored snapshot is
  drawn as it is). The address must pass the pairing form's check (https, or this device's own loopback). It then asks
  Hollow at `address` with the token (`GET /v1/sessions?limit=6` and `GET /v1/waiting`, each with an 8 second limit) and
  stores what came back, after reading the token once more: nothing is stored for a token that is gone or changed. A
  failure leaves the old snapshot. A 401 sets `signedOut` and empties the lists. A 404 on `GET /v1/waiting` (an older
  Hollow) leaves the waiting count unknown. Each widget asks to be drawn again 15 minutes after it last drew; a closed
  app's widgets refresh on iOS's schedule (WidgetKit decides, and may take longer), so a widget that was not drawn for
  a while shows the age of what it holds.

A snapshot older than an hour says so ("Updated 2 h ago" in small type; "2 h" on the Lock Screen's small widgets), so a
number that is out of date reads as one. The timelines carry entries an hour and two hours ahead, so the note appears
on time even when iOS does not reload the widget.

## Privacy

- A chat's title and a question's text on the Lock Screen are marked privacy sensitive: iOS hides them while the
  iPhone is locked when the person asked for that. The circular and inline Lock Screen widgets show only a count.
- The snapshot holds chat titles and question previews. The item is in the Keychain (encrypted, this device only), not
  in a file of the app's, and Unpair, a 401 and pairing again delete it at once. WidgetKit also keeps the timeline
  entries it was given (each holds the snapshot, so the same titles and previews) in the system's widget cache on this
  device until the widgets next load a timeline: the app asks for that (`reloadAllTimelines`) about a second after the
  pairing changes, which all three of those do while the app is open, and the widget then draws "Open Hollow to pair".

## Older Hollow, older app

- **Hollow v1.0** (no `GET /v1/waiting`): Recent chats works. Waiting on you says "Update Hollow on your Mac to see
  what's waiting." in place of a count.
- An older app (builds 1 to 3) has no widgets. A widget in a newer app on an unpaired iPhone says "Open Hollow to pair
  with your Mac."

## Fixtures

`docs/phone-api-fixtures/widgets/snapshot.json`: a snapshot with one waiting chat and three recent chats. The app's
tests decode it and check that a snapshot encodes back to the same JSON.
