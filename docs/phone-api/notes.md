# Phone API v1.2: notes (quick capture)

2026-11-07 (overnight lane I3). One write route for the iPhone app's quick note: a few words typed on the phone become
a new Markdown file under `notes/` in the vault. The same file lives in both repositories
(`docs/phone-api/notes.md`), with the same fixtures in `docs/phone-api-fixtures/notes/`. Change it only in both at
once. v1 and v1.1 are unchanged.

The route needs the bearer token and passes the same Host and Tailscale checks as v1 (Who may call, and Deviations 2
in [../PHONE_CHAT_API.md](../PHONE_CHAT_API.md)). A Hollow without this route answers 404 `not_found`; the app then
shows "Update Hollow on your Mac to use this". **It only ever creates a file:** nothing here edits, appends to,
renames or deletes a note, and an existing file is never overwritten.

## `POST /v1/notes`

Request (fixture `notes/create-request.json`; an unknown field is a 400):

```json
{
  "idempotencyKey": "7f3a9c2e-5b1d-4e8a-9c6f-2d4b8a1e0c35",
  "text": "Ask the vendor about the second invoice\nThe PO number was 4471.",
  "title": "Vendor invoice question"
}
```

- `idempotencyKey` (required): 8 to 64 letters, digits, `-`, `_` and `.` (spaces around it are ignored), made by the
  app when the person starts the note and kept for every retry of it. Hollow scopes it to the iPhone (`<deviceId>.<key>`), so two phones never share a
  key. 400 `bad_request` otherwise.
- `text` (required): what to keep, at most 48 KiB of UTF-8 after trimming, and not empty. It may hold line breaks and
  tabs; any other control character is a 400.
- `title` (optional): one line of plain text, at most 200 characters. Without it the title is the first non-empty line
  of `text`, cut to 80 characters on a character boundary. A title that is empty after that is a 400.

The note is the file `notes/<name>.md`, the same file a paired companion device's `note.create` makes
(`docs/COMPANION_PROTOCOL.md`): front matter with `title`, `created`, `source: companion: <this iPhone's name>` and
`companion-op: <deviceId>.<key>`, then a blank line and `text` with the whitespace at both ends trimmed. The name is the
title with every character other than letters, digits, spaces and `-_.,()&'` dropped, at most 100 bytes ("note" when
nothing is left); a name already taken gets ` 2`, ` 3` and so on (at most 50), never an overwrite. A name an agent
would load as instructions when it reads that folder (`Claude`, `Claude.local`, `Agents`, `Agents.override` and
`Gemini`, in any letter case) gets ` note` after it, so a note can never be `CLAUDE.md` or `AGENTS.md`.

200 (fixtures `notes/created.json`, `notes/duplicate.json`):

```json
{
  "note": {
    "path": "notes/Vendor invoice question.md",
    "title": "Vendor invoice question",
    "created": "2026-11-07T09:12:44-05:00"
  },
  "disposition": "applied"
}
```

- `note.path`: the vault-relative path of the file, always under `notes/`.
- `note.created`: when Hollow made it (the front matter's `created`), RFC 3339.
- `disposition`: `applied` for a new file, `duplicate` when the same key had already made one (a retry after an answer
  was lost): the answer names that file and nothing is written. A repeat is found while the file still has its name
  (`<name>.md` or `<name> 2..50.md` under `notes/`) and its `companion-op` line; one the person renamed or moved on
  the Mac before the retry is made again. The name comes from the title: the same key under a title that gives the
  same name (the same title with other text, or titles that differ only in characters a name drops, such as
  `Plan: A` and `Plan A`, or in letter case on the Mac's usual case-insensitive volume) answers `duplicate` with the
  first note and the new words are not saved; under a title that gives another name it makes a new file. The app never
  reuses a key for changed words.

Errors, all in the v1 error shape (`{"error": {"code": ..., "message": ...}}`):

- 400 `bad_request`: a missing, short or long key, a missing or empty `text`, a `text` over 48 KiB, a control
  character, an over-long or empty title, an unknown field, a body that is not JSON, and the day's limit (below). Also
  a vault that cannot take the file (a `notes` folder that is a link, every name from `<name>.md` to `<name> 50.md`
  taken, a read-only folder): the message says why and nothing was written, so the app keeps the words.
- 401 without a valid bearer token.
- 413 `too_large`: a body over 256 KiB, the limit every v1 route has.
- 409 `busy`: Hollow is in the middle of an automated run or a write, so the file waits. The app says so and the
  person tries again (the same key is safe).
- Nothing else: no 404 other than an older Hollow's. A note that was written is never answered with a 500: if
  something fails after the file landed, the retry answers `duplicate`.

An iPhone may make at most 200 new notes in 24 hours (counted in memory by the phone API, so a restart of Hollow starts
the count again); a repeat that only finds its earlier file does not count.

## For the app

- Send the key made when the note was started, again on every retry of the same words; a changed `text` or `title`
  is a different note and gets a key of its own (under the old key Hollow would answer the first note again, or
  make a second file).
- Keep the unanswered note, with its key, until Hollow answers (a timeout, a dropped connection or a killed app may
  have left a note on the Mac). `duplicate` is as good as `applied`.
- The app shows `note.path` ("Saved to notes/Vendor invoice question.md"); it does not open the file.
