# Phone API v1.2: workspaces and file preview

2026-11-07 (overnight lane I3). Read-only routes for the iPhone app's Workspaces tab: a list with the pinned
workspaces first, one workspace's outcome, plan, chats and Results, and a preview of one file. The same file lives in
both repositories (`docs/phone-api/workspaces.md`), with the same fixtures in `docs/phone-api-fixtures/workspaces/`.
Change it only in both at once. v1 and v1.1 are unchanged.

Every route needs the bearer token and passes the same Host and Tailscale checks as v1 (Who may call, and Deviations
2 in [../PHONE_CHAT_API.md](../PHONE_CHAT_API.md)). **Nothing here writes**: no route changes a workspace, a brief, a
plan, a file or a Result's review state (the phone never accepts a Result). A Hollow without these routes answers 404
`not_found`; the app then shows "Update Hollow on your Mac to use this".

## Card

The row the Workspaces tab shows:

```json
{
  "id": "hollow-companion-app-cdc399a4852fa14c",
  "title": "Hollow Companion App",
  "kind": "vault",
  "status": "active",
  "pinned": 1,
  "modified": "2026-11-04T16:07:11.658361+00:00",
  "lastActivityAt": "2026-11-06T21:10:02.758538+00:00",
  "chatCount": 14,
  "runningCount": 1,
  "waitingCount": 0,
  "nextStep": "Ship round 6 to TestFlight."
}
```

- `kind`: `vault`, `build`, `project`, `notion`, `quick` or `main` (as Session's `workspace.kind`).
- `status`: the brief's status as written (`active`, `planning`, `in progress`, `blocked`, `paused`, `done`,
  `archived`, or any other word).
- `pinned`: the pin's rank (lower first) or null. Never set for an archived workspace.
- `modified`: when the brief last changed. `lastActivityAt`: the newest activity of a chat in this workspace, null
  when it has none.
- `chatCount`: unarchived chats here (counted among Hollow's newest 1,000). `runningCount`: those with a turn queued
  or running. `waitingCount`: those with a question card waiting for the person.
- `nextStep`: the brief's Next step section as one plain line, at most 240 characters, else null.

## `GET /v1/workspaces/overview?scope=current|all`

200 `{"workspaces": [Card], "total": 12}` (fixture `workspaces/overview.json`).

- `scope=current` (the default when `scope` is absent) lists the workspaces Hollow's Work page lists as current
  (status `active`, `planning`, `in progress` or `blocked`); `all` lists every workspace that is not in the trash,
  archived ones included. Anything else, an empty `scope=` too, is 400 `bad_request`.
- Chats and the Main agent are not listed: their chats are in the chat list.
- Order: pinned first by rank, then by the newest conversation (else when the brief changed), then by title.
- At most 200 cards; `total` counts every match.

## `GET /v1/workspaces/{id}`

200 (fixture `workspaces/detail.json`), for any id `GET /v1/workspaces` or the overview gave, and for `quick-chats`
and `main`:

```json
{
  "workspace": Card,
  "outcome": "A Hollow iPhone app on TestFlight ...",
  "plan": {"text": "# Plan\n...", "truncated": false, "openItems": 3, "modified": "2026-11-06T20:00:00+00:00"},
  "documents": [{"name": "WORKSPACE.md", "size": 2412, "modified": "2026-11-04T16:07:11+00:00"}],
  "chats": [Session],
  "chatsHasMore": false,
  "results": [{"path": "outputs/README.md", "name": "README.md", "title": "Hollow Companion App",
               "kind": "document", "size": 4120, "modified": "2026-11-05T09:00:00+00:00"}],
  "resultsHasMore": false
}
```

- `outcome`: the brief's `## Outcome` section as Markdown, at most 4,000 characters, else null.
- `plan`: `PLAN.md` as Markdown (`text`, at most 60,000 characters, `truncated` when cut), `openItems` the number of
  unchecked `- [ ]` or `* [ ]` items in the first 512 KiB of the file (which can be more than `text` holds), `modified`.
  Null when the workspace has no plan.
- `documents`: the context documents that exist, in the order `WORKSPACE.md`, `PLAN.md`, `SOURCES.md`, `AGENTS.md`.
  The app opens one through the file route below. A document that is a link to a file outside the workspace's own
  folder (another workspace's plan, anything outside the vault) counts as missing here, in `plan`, `outcome` and
  `nextStep`, as it does in the file route.
- `chats`: the newest 50 chats in this workspace, each a v1 Session (`GET /v1/sessions`), newest activity first;
  `chatsHasMore` when there are more. Like `chatCount`, they come from Hollow's newest 1,000 unarchived chats (the
  window the Mac's workspace page reads), so a chat older than that is in neither.
- `results`: the newest 100 files under `outputs/`, as Hollow's Results lists them (hidden folders, a page review's
  captures, a canvas's sidecars and archived brief entries are left out): `path` is relative to the workspace folder
  and is what the file route takes, `title` the document's first heading (else the file name as words), `kind` one
  of `document`, `table`, `image`, `video`, `page`, `review`, `canvas`, `pdf`, `data`, `file`. No review state is
  sent. `resultsHasMore` when there are more. Only files the file route can open are listed (no backslash or control
  character in the path, at most 400 characters); a link is never listed or followed, and a link in place of
  `outputs/` lists nothing. The walk looks at 5,000 entries at most, files and folders alike: in a larger tree the
  newest of the files it reached first are listed and `resultsHasMore` is true.
- 404 `not_found` for an unknown or trashed id.

## `GET /v1/workspaces/{id}/file?path=<relative path>`

One file of the workspace's folder for the app's preview. `path` is relative to the workspace folder
(`outputs/notes.md`, `PLAN.md`), `/`-separated, percent-encoded once. On this route a `+` is a plus sign, not a
space (a file may be named `C++ notes.md`): a space travels as `%20`.

- 200 (fixtures `workspaces/file-text.json`, `workspaces/file-binary.json`):

```json
{"path": "outputs/notes.md", "name": "notes.md", "kind": "document", "mime": "text/markdown", "size": 1204,
 "modified": "2026-11-05T09:00:00+00:00", "encoding": "utf8", "content": "# Notes\n...", "truncated": false}
```

- `encoding` is `utf8` for a text file (Markdown, plain text, CSV, TSV, JSON, YAML, TOML, XML, HTML, source code):
  `content` is its text, cut at 1 MiB on a character boundary with `truncated` true. Every other file is `base64`
  (standard alphabet, padded): `content` is the whole file, `truncated` false. A file whose extension says text but
  whose bytes are not UTF-8 travels as `base64` with `application/octet-stream`.
- `mime` comes from the file's extension. `size` is the file's size on the Mac in bytes. `kind` is as in Results.
- 400 `bad_request`: `path` is empty, longer than 400 characters, absolute, has a backslash or control character
  (the path is never trimmed, so a leading line break is refused), or a segment that is empty, `.`, `..` or starts
  with a dot (Hollow keeps its own state in `.hollow/`).
- 404 `not_found`: an unknown or trashed workspace, no such file, a directory, a pipe or device, or a path that
  resolves outside the workspace's own folder (a symbolic link out) or into a hidden folder of it (a link to
  `.hollow/`). Hollow opens the resolved path with `O_NOFOLLOW_ANY` on macOS (as attachment downloads do; the
  context documents above too), so a folder on the path swapped for a link after the check is a 404 as well.
- 413 `too_large`: a file over 10 MiB that is not text (fixture `workspaces/error-file-too-large.json`). The app
  says the file is too large to open on the phone.

## Notes for the client

- Chats listed in a detail open in the Chats tab's conversation by `key`, as any chat does.
- A Markdown file or the plan is rendered read-only; the app never writes back. Canvas files (`kind` `canvas`)
  arrive as HTML text: a later phone API (canvases) renders them, until then they show as a file.
- Every response carries `Cache-Control: no-store`, as in v1. Nothing here is a stream: the app refetches a screen
  when it opens and when a `sessions` event arrives for a chat it shows.
