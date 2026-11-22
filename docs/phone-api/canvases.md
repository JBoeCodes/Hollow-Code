# Phone API v1.2: canvases

2026-11-07 (overnight lane I1). Part of the phone chat API ([PHONE_CHAT_API.md](../PHONE_CHAT_API.md)); the same
file lives in both repositories, with the same fixtures in `docs/phone-api-fixtures/canvases/`. Transport, auth,
errors and shapes are v1's unless this file says otherwise. Every route here needs the device token. An older Hollow
answers each of them 404 `not_found` ("Hollow has no such route."): the app then keeps v1's "open it in Hollow on the
Mac" row.

What it adds: the iPhone app draws Hollow's canvases (`hollow-canvas` fences, [CANVAS.md](../CANVAS.md)) itself, in a
`WKWebView` with Hollow's own canvas kit and host runtime, and answers an ask-canvas as the Mac's Send under it does.
Canvases are read from Hollow's canvas store (`workspace_canvases`) and the conversation's own transcript; only the
answer route writes, and it writes nothing but the person's message.

## The document the app builds

The app draws every canvas as one HTML document:

```
<!doctype html><html><head>{kit.head}{ask ? kit.askHead : ""}</head><body>{body}</body></html>
```

- `kit` is `GET /v1/canvas-kit`'s answer, cached by its `sha256`.
- `body` is the canvas's frame body: `<div data-hc-texture-root texture="T" style="display:contents">INNER</div>`,
  where `INNER` is everything between the root's start tag and its `</hc-canvas>` and `T` is `ascii` when the root's
  `texture` attribute (entities decoded, trimmed, any case) is `ascii`, else `dither` (`canvasFrameBody` in
  `src/canvas/fence.ts`). A read route gives it as `body`; for a fence in a reply the app builds it from the fence
  (a fixture holds the pair, so the two agree).
- `kit.head` carries the Canvas mode's policy as a `<meta>` (scripts inline only, no network: `img-src` and
  `media-src` only `data:` and `blob:`, `font-src` only `data:`, `connect-src 'none'`), the theme tokens for light and dark,
  Hollow's artifact styles, the kit's stylesheet, Hollow's host runtime (`src/components/sandbox-doc.js`) and the
  kit's script, byte for byte as the Mac serves its own frames. `askHead` is the ask kit's stylesheet and script.

The app's web view (the client's half, not Hollow's): JavaScript on; a `WKContentRuleList` that blocks every load
whose URL is not `data:` or `blob:`; no navigation away from the document (a link opens only after a confirm sheet
naming the full address, and only `https:` and `mailto:`); no file access; no persistent website data. The Canvas
mode's `<meta>` policy holds as well.

### The bridge on the phone

The page is the top window, so the host runtime's host (`window.parent`) is the page itself. The app speaks the
canvas bridge v1 ([CANVAS.md](../CANVAS.md#the-bridge-v1)) through the page's own `message` events:

- **Host to canvas:** the app runs `window.postMessage(<message>, "*")` in the page. After the document loads it
  sends `theme` (`{"__hc": 1, "kind": "theme", "vars": kit.themes[scheme], "scheme": "light" | "dark"}`), then
  `init` (`canvasId`, `kit`, `locale`, `reducedMotion`, and `state` from the read route, else null), then `motion`
  (`reducedMotion`, `textureAllowed: !reducedMotion`, `textureLevel: "full"`). It sends `theme` again whenever
  the phone's appearance changes, so a canvas that redraws on `hollow-theme` follows it.
- **Canvas to host:** a script the app adds at document start (in its own content world) forwards every
  `{"__hc": 1}` message whose `kind` is a child kind (`size`, `settled`, `error`, `navigate`, `open`, `blocked`,
  `state`, `draft`, `ask-invalid`) to the app. The app treats each as untrusted and checks it as
  `src/canvasBridge.ts` does (exact fields, limits; `state` and `draft` only for this canvas's id; `draft` only
  with the nonce of the app's own `collect`).
- The app uses `size` and `settled` for the card's height, shows `error` as a line under the canvas, opens
  `open` links through the confirm sheet, and ignores `navigate` (no vault file view in v1.2) and `blocked`.
  `state` stays in the web view: the phone saves no canvas state in v1.2.

## Routes

### `GET /v1/canvas-kit`

What every canvas document shares (fixture `canvases/canvas-kit.json`, its long strings cut short):

```json
{
  "kit": "kit@1",
  "sha256": "2f0c…64 hex…",
  "head": "<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; …\">…",
  "askHead": "<style>…</style>\n<script>…</script>",
  "themes": {
    "light": {"--app-ink": "#1a1a1a", "--app-canvas": "#ffffff", "…": "…"},
    "dark": {"--app-ink": "#f9f9f9", "--app-canvas": "#1a1a1a", "…": "…"}
  }
}
```

- `kit`: the kit version (`KIT_VERSION`, `kit@1`), which the app passes on in `init`.
- `sha256`: hex SHA-256 over `kit`, `head`, `askHead` and the themes, each followed by a NUL. It changes when any of
  them does; every canvas route also gives it as `kitSha256`, so the app fetches the kit only when its cached copy's
  `sha256` differs.
- `themes`: the `--app-*` tokens the Mac's frames see (`THEME_TOKENS` in `src/components/htmlSandbox.ts`), with
  Hollow's stock light and dark values from `src/styles/tokens.css`. `head` holds the same values as CSS, light in
  `:root` and dark in `@media (prefers-color-scheme: dark)`.
- About 130 KB. It holds no data: no chat, canvas or workspace.

### `GET /v1/sessions/{key}/canvases`

Every canvas the chat's replies drew, in transcript order (fixture `canvases/session-canvases.json`):

```json
{
  "kitSha256": "2f0c…",
  "canvases": [
    {
      "turnId": "s-20261104-6e772c5f-3-assistant",
      "index": 0,
      "id": "release-status",
      "title": "Release status",
      "ask": false,
      "height": 480,
      "revision": 2,
      "latestRevision": 3,
      "updatedAt": "2026-11-04T16:20:02.104000+00:00",
      "answered": null
    }
  ]
}
```

- One entry per closed `hollow-canvas` fence in an assistant turn that validates (`canvas_fence::validate`), in
  order; `turnId` is the turn's id as `GET /v1/sessions/{key}` gives it and `index` the fence's place among that
  turn's valid fences with the same canvas `id` (0 first; almost always 0). The app matches a reply's fence to its
  entry by `turnId`, the root's `id` and that place. At most the newest 200 entries.
- `revision`: the saved revision whose body is exactly this fence's (`fenceSha256` in the sidecar), else null (not
  saved, or aged out of the 30 kept). `latestRevision` and `updatedAt`: the store's current revision and its time,
  null when the canvas isn't saved. The store is the chat's own: the workspace whose folder holds the chat's folder,
  else the chat's app-data store, as the Mac's chat picks it.
- `answered`: for an ask-canvas fence whose revision this chat answered, `{"revision", "summary"}`: the revision
  and the lines the person sent (the answer message without its header and hidden block, at most 2,000
  characters), read from the chat's messages as the Mac's lock reads them (`canvas_answers::answered`); else null.
- The app offers an ask-canvas's Send only when `ask`, `revision` equals `latestRevision`, and `answered` is null.
  An earlier revision is shown read-only ("A newer version of this canvas replaced it.").
- 404 `not_found`: the chat isn't in Hollow or was archived.

### `GET /v1/sessions/{key}/canvases/{id}?revision=<n>`

One saved canvas from the chat's store, the current revision unless `revision` names a kept one (fixture
`canvases/canvas.json`; an ask-canvas: `canvases/canvas-ask.json`):

```json
{
  "id": "release-status",
  "title": "Release status",
  "revision": 3,
  "latestRevision": 3,
  "ask": false,
  "height": 480,
  "kit": "kit@1",
  "updatedAt": "2026-11-04T16:20:02.104000+00:00",
  "source": "<hc-canvas id=\"release-status\" title=\"Release status\" height=\"480\">\n…\n</hc-canvas>",
  "body": "<div data-hc-texture-root texture=\"dither\" style=\"display:contents\">\n…\n</div>",
  "state": {"tab": "Week"},
  "answered": null,
  "kitSha256": "2f0c…"
}
```

- `source`: the revision's fence body as the agent wrote it. `body`: its frame body (above).
- `state`: the canvas's saved view state (at most 16 KB), else null. It belongs to the canvas, not a revision.
- `answered`: as in the list, for this revision in this chat.
- 404 `not_found`: the chat, the canvas (never saved in the chat's store) or the revision (no longer kept).
- 400 `bad_request`: an `id` that isn't 1 to 64 lowercase letters, digits or hyphens, or a `revision` that isn't a
  whole number from 1.

### `POST /v1/sessions/{key}/canvases/{id}/answer`

Sends an ask-canvas's answers as the person's next message in the chat, the way the Mac's Send under the canvas
does (fixture `canvases/answer-request.json`):

```json
{
  "revision": 3,
  "answers": {"order": ["Docs", "Beta", "Pricing page"], "budget": 20, "day": "This Friday"},
  "summary": "What goes out first?: Docs, Beta, Pricing page\nLaunch budget: 20 k$\nWhen?: This Friday",
  "clientMessageId": "6f1c2a9e-0b7d-4c55-9a51-2f3e8d7c4b10"
}
```

- `answers` and `summary` are the kit's `draft` (the app's `collect` asked for them): `answers` a map of field
  names (`^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`) to text, a number, a list of texts, a `{row: text}` map or null, at
  most 16 KiB as compact JSON; `summary` 1 to 500 characters, no control characters but newlines and tabs.
- Hollow checks that `revision` is the canvas's current saved revision, that it is an ask-canvas, and that this
  chat hasn't answered it, then writes the message as the Mac's composer gets it (`canvasAnswerText` in
  `src/canvas/askAnswer.ts`; only the hidden block's key order differs, see below):

  ````
  Answered canvas "launch-order":
  - What goes out first?: Docs, Beta, Pricing page
  - Launch budget: 20 k$
  - When?: This Friday

  ```hollow-canvas-answer
  {"canvasId":"launch-order","revision":3,"answers":{"budget":20,"day":"This Friday","order":["Docs","Beta","Pricing page"]}}
  ```
  ````

  (the hidden block's `answers` keys come out sorted; the Mac's own Send keeps the form's order, and the lock and
  the agent read the JSON, so the order carries nothing) and sends it as `POST /v1/sessions/{key}/messages` sends a message (the same checks, mode, `clientMessageId`
  repeat rule and answers). The transcript then holds the answer block, so the Mac's canvas locks too.
- 202: the send's answer (`send-accepted.json`). 409 `busy`: the chat has a turn queued or running (send again when
  it ends). 409 `canvas_closed` (new in v1.2, fixture `canvases/error-canvas-closed.json`): the canvas isn't an
  ask-canvas, `revision` isn't its current one, or this chat already answered it. 404 `not_found`: the chat or the
  canvas. 400 `bad_request`: a malformed body. After a 202 the app sends the frame
  `{"__hc": 1, "kind": "answered", "summary": <the summary>}` and shows the canvas as answered.

### `GET /v1/workspaces/{id}/canvases` and `GET /v1/workspaces/{id}/canvases/{canvasId}?revision=<n>`

A workspace's saved canvases (its `outputs/canvases/`), for the app's Results: the list is
`{"kitSha256", "canvases": [{"id", "title", "revision", "updatedAt"}]}`, newest first (fixture
`canvases/workspace-canvases.json`); one canvas is the read route's shape with `answered` always null (answers
belong to a chat). 404 `not_found`: no such workspace, or no such canvas or revision. Read-only: Send works only
from the chat that showed the canvas.

## Errors added

| code | status | when |
|---|---|---|
| `canvas_closed` | 409 | answering a canvas that isn't an ask-canvas, isn't the current revision, or was answered |

The app shows any error's `message` as it is and treats an unknown `code` like `internal`.
