# Spotlight, quick actions and workspace links (phone API v1.2)

2026-11-07, lane I4 of the overnight build. Part of the [phone chat API](../PHONE_CHAT_API.md), v1.2 additions. The
indexed items and the quick actions are fixed in `docs/phone-api-fixtures/spotlight/`.

Three small things that make the Hollow iPhone app reachable from iOS itself: its newest chats in **Spotlight search**, two
**Home Screen quick actions** (touch and hold the icon), and a link to open a **workspace**. **There is no new Hollow
route**: the index is built from the chat list the app already holds (`GET /v1/sessions`), so this file describes the
app's side only. It lives in `hollow-companion`; Hollow's copy of the v1.2 docs links here.

## Spotlight

**What goes in.** The newest 50 chats of the list that have a title, one item each: the title (cut at 80 characters, line
breaks as spaces), "Hollow chat in *workspace*" (or "Hollow chat" when it has none) as the description, and the keywords
"Hollow", "chat" and the workspace's name. **Never what was said in a chat**: the app holds no messages for most chats,
and a search index is not the place for them. A chat with no title yet ("New chat") says nothing a search could match, so
it is indexed once it is named. `items.json` is the result for `sessions.json`.

| Part | Value |
|---|---|
| Identifier | `hollow.chat.<session key>`; anything else (another prefix, a key that isn't letters, digits, `-`, `_`, `.`, or only dots) opens nothing |
| Domain | `com.hollow.app.companion.chats`: one delete removes every item (Unpair, the switch) |
| Lifetime | each item expires 30 days after it was last indexed, so a chat that fell off the list ages out by itself |
| Rank | `contentModificationDate` is the chat's last activity when it was indexed; a change of that alone (a turn that streams) does not index again |

**When it is indexed.** While the app runs, 2 seconds after the chat list, the pairing or the switch changes. The first pass
of each launch, of each pairing and of each week deletes the domain and indexes the current list, so a chat deleted or
renamed since leaves nothing behind and an unchanged chat is renewed before its 30 days run out; after that only items
that are new or whose title or workspace changed are indexed again, and chats that left the list are deleted. Before the
list has loaded nothing is changed. A call iOS refuses is not forgotten: the pass starts again from an empty domain 30
seconds later. An answer to a pass that a removal or a newer pass has overtaken indexes nothing.

**When it is removed.** The store empties the index in the same call that ends the pairing (Unpair, a 401 from Hollow,
also in a launch that only handled a notification's button, where no view and so no sync ever starts) or changes it
(pairing again, with another Mac), and in the same call that turns **Settings → Spotlight → Search chats from the Home
Screen** off (on until the person turns it off; the choice survives Unpair). The sync deletes again 2 seconds later, and
retries a refused delete. The titles of another Mac's chats are therefore never searchable beside this Mac's token.

**A tap.** iOS hands the app an `NSUserActivity` of type `CSSearchableItemActionType` whose identifier is the item's. When
the iPhone is paired, the app opens that chat (`ChatStore.openSpotlightItem`, which sets the same request a notification's
tap does); an identifier that isn't ours, or any tap while unpaired, does nothing (an unpaired app shows its Pair screen).

**Privacy.** Titles and workspace names are the only content, they stay on this iPhone (the index is not synced and no
Hollow route is involved), Unpair and the switch remove them, and iOS lets the person hide them without Hollow:
Settings → Siri & Search → Hollow → Show Content in Search. The widgets mark the same titles privacy sensitive on the Lock
Screen; the index has no such mark, and whether iOS offers search on the Lock Screen is the person's iOS setting (Face ID
& Passcode → Allow Access When Locked), which the app can neither see nor change. It is **on by default** (work chat
titles are work data, so the choice is the person's: Settings → Spotlight turns it off, and nothing else in Hollow
depends on it); a person who shares an unlocked iPhone may want it off.

## Home Screen quick actions

Touch and hold the Hollow icon (`docs/phone-api-fixtures/spotlight/quick-actions.json`; the same two items are
`UIApplicationShortcutItems` in `ios/App/Info.plist`, which a package test keeps in step):

| Item | Type | Does |
|---|---|---|
| **New chat** | `com.hollow.app.companion.new-chat` | the same as `hollow://new`: the New chat sheet |
| **Waiting on you** | `com.hollow.app.companion.waiting` | the same as `hollow://waiting`: the chat list, whose first section is Waiting on you |

Both are static (iOS shows them even before the first pairing). Unpaired they open the app on its Pair screen and the
request is dropped, as a widget's link is. A launch from a quick action hands its item to the app delegate before the
window exists (the chat list reads the request when it appears); one chosen while the app runs arrives at a scene
delegate (`PhoneSceneDelegate`), set only for that: it implements no `scene(_:willConnectTo:)`, so SwiftUI still builds
the window. Mac Catalyst has neither.

## The workspace link

`hollow://workspace/<workspace id>` joins `hollow://chat/<key>`, `hollow://new` and `hollow://waiting`. Once paired it
brings the Workspaces tab forward and opens that workspace's detail, whatever the tab showed (the detail loads by id; its
title shows once the list has it, "Workspace" before). The id has a chat key's shape (letters, digits, `-`, `_`, `.`;
never only dots, at most 120 characters): a link with a path, a query, a port or an id with any other character is
ignored. A link for a workspace Hollow doesn't have opens a detail that says "Workspace gone". Use it from a Shortcuts
"Open URL" action or a note.

## Older Hollow, older app

- Hollow v1.0 or v1.1: Spotlight and the quick actions need nothing from Hollow. The workspace detail is a v1.2 read
  (lane I3's workspaces route), so against an older Hollow `hollow://workspace/<id>` opens a detail that says "Needs a
  newer Hollow", as the Workspaces tab does.
- An older app (builds 1 to 3) has none of this.

## Not checked on a device

What iOS search shows (the ranking, "Show Content in Search"), a tap on a result and the icon's menu on a real Home Screen.
The index calls are `CSSearchableIndex`'s; the plan is a package test, the quick actions are pressed on the simulator's
Springboard by a UI test (`test9QuickActions`), and the Settings switch and the workspace link by `test9Spotlight`.

## Fixtures

`docs/phone-api-fixtures/spotlight/items.json`: the domain and the items for `sessions.json` (the chat with no title is
not indexed). `quick-actions.json`: the two quick actions, their titles, SF Symbols and links. The app's tests check each
against the code, and the quick actions against `ios/App/Info.plist`.
