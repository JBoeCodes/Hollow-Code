# Phone chat API v1.2: attachments

2026-11-07. Photos, camera pictures, scanned documents and files go from the iPhone into a chat as attachments, and the
files a chat's turns carried (from the phone or from the Mac's composer) show in its transcript. Additive: v1 and v1.1
behave as before. A Hollow without this answers the new routes 404 `not_found` ("Hollow has no such route.") and
ignores `attachmentIds`. The same file and `docs/phone-api-fixtures/attachments/` live in the Hollow repository and in
`hollow-companion`; change them in both at once.

## Limits

| What | Limit | Over it |
|---|---|---|
| One file | 25 MiB (26,214,400 bytes) | 413 `too_large` |
| Attachments on one message | 10 | 400 `bad_request` |
| One message's attachments together | 50 MiB (52,428,800 bytes) | 413 `too_large` |
| Uploads Hollow takes at once | 3 | a fourth waits up to 20 seconds for a place, then 503 `unavailable` |
| Downloads Hollow serves at once | 4 | a fifth waits up to 20 seconds for a place, then 503 `unavailable` |
| An upload's body arrives within | 5 minutes, with no more than 60 seconds between two chunks | 400 `bad_request` |

An upload's body is the file itself, not JSON, so the 256 KiB body limit of the other routes does not apply to the two
upload routes (and only to them). An empty file is 400 `bad_request`. A refusal that comes before the body is read
(over the limit by `Content-Length`, or no place in 20 seconds) first reads and drops the body, up to 26 MiB and 15
seconds, so a phone that is still sending hears the answer instead of a reset. A place is held for as long as the
file's bytes are in Hollow's memory: an upload's until its save has ended (a phone that hangs up halfway through the
save does not free it early), a download's until its response has been written.

Hollow keeps no quota on what a chat's folder holds. An upload sent without `clientUploadId` is saved every time it is
sent, so a client that retries without an id saves the file again each time; the app always sends an id. Clearing a
chat's `.hollow-attachments/` folder is the person's (or an agent's) to do on the Mac.

## Shape

### Attachment

```json
{"id": "at-1f0e3c5a9b7d2468", "name": "IMG_0412.jpg", "kind": "image", "mime": "image/jpeg", "size": 482113, "available": true}
```

- `id`: `at-` and 16 lowercase hex characters. Opaque to the app. The same file has the same id in the upload's answer
  and in the transcript.
- `name`: the saved file's readable name, without the generated part Hollow adds: ASCII letters, digits, dot, dash
  and underscore, at most 48 characters of stem and a lower-case extension (`Quarterly-report.pdf`, `IMG_0412.jpg`).
  The Mac composer's attachments have the same kind of name.
- `kind`: `image` when the bytes are a PNG, JPEG, GIF or WebP (the types the Mac composer shows as a thumbnail and
  hands the agent as an image), else `file`. Hollow reads the bytes; the name and the request's `Content-Type` never
  decide it. The app treats any other `kind` like `file`.
- `mime`: from the bytes when Hollow knows them (`image/png`, `image/jpeg`, `image/gif`, `image/webp`, `image/heic`,
  `application/pdf`, `application/zip`, `video/mp4`, `video/quicktime`, `audio/mp4`, `text/plain` for text with no NUL
  byte), else from the name's extension for a few common text and Office types, else `application/octet-stream`.
- `size`: bytes on disk.
- `available`: `false` in a transcript when the saved file is no longer on the Mac (deleted, or the folder moved) or
  is over 50 MiB, which the download route does not serve (an agent can leave a bigger file in the folder).
  An upload's answer always says `true`.

## Routes

### `POST /v1/sessions/{key}/attachments?name=<file name>&clientUploadId=<uuid>`

Saves one file into the chat's folder, in the hidden `.hollow-attachments/` folder where the Mac composer keeps its own
(`.gitignore` inside it keeps the files out of git). The body is the file's bytes. `Content-Type` is advisory and
ignored. The app sends `Content-Length`; a body without one is read up to the same limit. The file is written under a
hidden name and renamed when whole, so a save that is cut short (a crash, a full disk) leaves no file that looks
finished.

- `name`: the file's original name, percent-encoded UTF-8, 1 to 255 characters, no control characters. Hollow keeps a
  cleaned stem and extension of it. For an image Hollow's own reading of the bytes decides the extension (a PNG sent
  as `shot.jpg` is saved as `shot.png`).
- `clientUploadId` (optional): a UUID from the app. The same id from the same device for the same chat (or the same
  workspace) within 10 minutes returns the first answer and saves nothing again, so a retry after a lost reply never
  saves twice; the same id sent to another chat or workspace is another upload.
- 201 `{"attachment": Attachment}` (fixture `attachments/upload-response.json`).
- 400 `bad_request`: no or invalid `name`, an empty body, a body that stopped early.
- 404 `not_found`: an unknown or archived chat.
- 413 `too_large` (fixture `attachments/error-too-large.json`): over 25 MiB.
- 503 `unavailable`: three uploads are already running and none ended within 20 seconds, or the chat's folder is gone
  or one Hollow refuses to write in (`/`, `/tmp`, a home folder; the same rule as the Mac composer).

### `POST /v1/workspaces/{id}/attachments?name=<file name>&clientUploadId=<uuid>`

The same for a new chat that does not exist yet: the file goes into the folder a new chat in that workspace would run
in (`quick-chats` and `main` included; an id from `GET /v1/workspaces`). Unknown workspace: 404 `not_found`. Then
`POST /v1/sessions` with the returned ids. An attachment uploaded this way belongs to that folder: a chat in another
folder refuses it (see Sending).

### Sending

`POST /v1/sessions/{key}/messages` and `POST /v1/sessions` take one more optional field, `attachmentIds`: up to 10
ids from the upload answers (fixtures `attachments/send-request.json`, `attachments/send-request-attachment-only.json`,
`attachments/new-chat-request.json`).

- `text` may be empty or left out when `attachmentIds` has at least one id. With neither text nor an attachment the
  request is 400 `bad_request`, as before.
- Each id must name a regular file in the chat's own `.hollow-attachments/` folder (for a new chat, the folder a
  chat in that workspace would run in). Otherwise 404 `attachment_gone`
  (fixture `attachments/error-attachment-gone.json`): the file was deleted, or it was uploaded for another folder.
  The app uploads the file again from its own copy and sends again. No part of the message is sent.
- A repeated id, a malformed id or more than 10 ids is 400 `bad_request`; the files together over 50 MiB is 413
  `too_large`. Both are checked before anything is queued.
- Hollow composes the prompt as the Mac's composer does (`composePayload` in
  `src/components/chat/chatSendController.ts`): its intent block, the words, then an `<attached-images>` block for
  the `image` files and an `<attached-files>` block for the others, each listing the saved files' absolute paths
  (a file's line also gives its `name`, the cleaned name above, where the Mac composer gives the original one).
  The agent opens them with its own file tools. A turn that runs on the Mac mini carries the folder with it.
- `clientMessageId` repeats behave as they do without attachments: the first answer comes back and nothing is queued.

### `GET /v1/sessions/{key}/attachments/{id}`

The file's bytes, for a thumbnail or Quick Look. Only files a turn of this chat carried are served, and only from the
chat's own `.hollow-attachments/` folder (the path is resolved and must stay inside it; a link, a folder or a file
outside is 404). The file is opened without following a link and judged on the opened file, so a link swapped in after
the check is 404 too.

- 200 with the bytes. `Content-Type` is the `mime` Hollow worked out (`application/octet-stream` when it knows no
  type), `Content-Length`, `Content-Disposition` (`inline; filename="<ASCII name>"` for a file of `kind` `image`,
  `attachment; filename="<ASCII name>"` for every other file, so a browser saves an `.xml` that is really an SVG instead
  of showing it), `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox` and, like
  every response, `Cache-Control: no-store`. The app keeps its own copy by `id`; a file never changes.
- 404 `not_found`: an unknown chat or id, a file that is gone, a file outside the folder, or a file over 50 MiB.
- 503 `unavailable`: four downloads are already running and none ended within 20 seconds.

### Turns carry their attachments

A user `Turn` (see Shapes → Turn) gains `attachments`: an array of `Attachment`, in the order of the prompt's blocks
(the images first, then the other files, each in the order they were attached), present only when the turn carried
files (fixture `attachments/turn-with-attachments.json`). It is filled for
turns started from the phone, from the Mac's composer and by an agent's message, from the `<attached-images>` and
`<attached-files>` blocks the turn's prompt carried, recorded when the turn starts (only files inside the chat's
`.hollow-attachments/` folder are recorded; a turn that started before this version has none). `text` stays the
person's own words with the blocks left out, as in v1. The app shows a thumbnail for each `image` and a chip for each
`file`, and asks for the bytes with the route above.

## What the app does (not part of the wire contract)

- Images are sent as JPEG or PNG: HEIC and other formats are converted on the phone, and a picture over 2048 pixels
  on its long side is scaled down first. Scans are PDFs.
- Uploads run one file at a time per message with progress, and a file stays on the phone (in the app's own folder)
  until its message is accepted, so a draft with attachments survives the app being closed and a lost id is uploaded
  again.
- An upload that answers 404 `not_found` with the message "Hollow has no such route." means Hollow on the Mac is
  older than this version: the app says to update Hollow.
