# Hollow Canvas — the reference

## Copy canvas items

The host dialog groups editable items and Plan previews on solid shared
materials; its body owns scrolling while Close and Confirm remain fixed.
Pinned hosts use the same reading surface with a separate wrapping header.
These visual roles do not change frame markup, palette bridges or copy rules.

Canvas actions → **Add items to Plan or Tasks…** opens a host dialog shared by
inline, Results and gallery views. It reads the saved current version, labels
that version (including when the view showed an older one), and uses the inert
Read as text parser. List and table rows become selectable, editable plain text;
at most 50 are shown and omitted items are counted. Unsaved, conversation-only,
Chats and Main-agent canvases need a project workspace first.

Plan copies require a heading and **Preview Plan**, then **Confirm**. The preview
contains the complete prospective document. The backend inserts only absent
row identities under the uniquely named heading (or a new level-two heading),
using shared-doc compare-and-swap. A changed Plan refuses the write and asks for
another preview; concurrent edits survive. Markdown metacharacters in row text
are escaped, and every copied row names the canvas and version.

Task copies always call local createTask. The linked project is resolved from
local notes; without a linked project, tasks go to Inbox. Stable keys bind
workspace, canvas id, version and original row position. Successful receipts
stay disabled during partial failure; retry attempts only missing items.
Reopening returns existing tasks, even if a title was edited; the receipt says
that those edits were not applied to existing tasks. These copies are
independent of the canvas and each other. The source is never changed, no paid
turn runs and no bridge message kind or agent action reaches this dialog.

The dialog/parser load on demand behind a cancellable host loading/error surface.
A newer saved revision refuses confirmation;
close and reopen to review it. Missing reads have Retry. Escape and Close return
focus to the menu trigger; both wait while a confirmation is in flight.

## State recovery and source navigation

A focused read-only frame's first state edit shows “Changes here aren’t kept”
and retags its header. Open vN reaches the current editable version when a
source route is recorded; unavailable sources say so. Current chat revisions
save state even when opened from an older reply. Failed saves offer Try again,
retaining the edit's original time and revision through CAS conflict refreshes.
Results and gallery menus offer Open source chat for the displayed revision.

Pin success is a four-second notice with Show. Existing pins offer Replace
and Unpin. A stale pin offers Update to vN, which verifies that revision and
its hash and refuses if the original pin changed or was removed during the read.
The canvas look sample has no artifact toolbar or source shortcut; its swatches
expose the background, text and accent colours as one named image.

The shared Present player shows a placeholder for canvas/HTML/SVG fences in
Markdown Results and release notes. Open the original Result for the live canvas;
Present never mounts another canvas frame. See [Present](reference/surfaces.md#welcome-tour-and-whats-new-overnight-10-11-s1).

Before installing a build containing canvas changes, Changes → Hollow app
lists that finished build's PRs and hand-check caveats. The receipt comparison
and dropped-commit warning are described in [Build & ship](reference/build-and-ship.md).

The one current description of canvases in Hollow: what they are, how they are
drawn, sandboxed, saved, answered and shown in chat, the Files picker and the
Preview pane, where the code lives, what is known not to work and which risks are open. It replaces reading
the slice reports in order; those stay as history (table below) and this file
links into them for detail. Written 2026-10-06 at `origin/main` `5a14838b`
(all of C0–C6 merged). Every claim here was checked against that code; files
are cited, line numbers are not.

**Installed:** the everyday app includes all of C0–C6 and the `hc-file` fix (#374); the current
install is `a1c80d02` (whiteboards removed, `hc-flow` diagrams, [record](history/WHITEBOARDS_REMOVED_INSTALL.md)), verified 2026-10-07. The programme is complete;
what remains is the by-hand checks and open risks listed below.

| Slice | What it added | History |
| --- | --- | --- |
| C0 | Exfiltration probe of the old modes, two P0 repairs (forged open, WebRTC), kit token sample, 30-frame sample | [CANVAS_C0.md](history/CANVAS_C0.md) |
| C1 | Canvas sandbox mode, served CSP per mode, bridge v1, live theme | [CANVAS_C1.md](history/CANVAS_C1.md) |
| C2 | kit@1 (display components), the gallery, the SRI repair (nonces, not hashes) | [CANVAS_C2.md](history/CANVAS_C2.md) |
| C3 | The fence, the store, revisions and state, chat rendering, frame budget, the prompt, the setting, receipts | [CANVAS_C3.md](history/CANVAS_C3.md) |
| C4 | Ask-canvases: the ask kit, draft → composer, the lock, Waiting on you | [CANVAS_C4.md](history/CANVAS_C4.md) |
| C5 | The six side-panel sections as native views of the Markdown | [CANVAS_C5.md](history/CANVAS_C5.md) |
| C6 | Curated section canvases; refused-message log, C4/C5 follow-ups, accessibility pass, performance | [CANVAS_C6.md](history/CANVAS_C6.md) |

Install records: [C0](history/CANVAS_C0_INSTALL.md), [C1](history/CANVAS_C1_INSTALL.md),
[wave 1](history/CANVAS_W1_INSTALL.md), [wave 2](history/CANVAS_W2_INSTALL.md),
[wave 3](history/CANVAS_W3_INSTALL.md). Bed details: [TEST_BED.md](history/TEST_BED.md#canvas-c0-evidence-limits)
onwards.

## What a canvas is

A **canvas** is a rendered HTML answer an agent writes in a `hollow-canvas`
fence: layout, numbers, charts, timelines, choices. It may use `<canvas>` and
SVG inside but is not limited to them. Four rules hold everywhere:

1. **Markdown is the truth.** A canvas is a view. Agents keep working in
   WORKSPACE.md, PLAN.md, SOURCES.md and their notes, and are told never to
   write a canvas into them. Nothing renders a canvas back into Markdown.
2. **Agent HTML runs only in the sandbox.** Every agent-written canvas,
   runs in an opaque-origin frame with scripts on and no network.
3. **A canvas never sends on its own.** An ask-canvas's answers go out only
   when the person presses Hollow's own Send under it (question cards,
   2026-10-14; before that, Send only filled the message box), in every
   permission mode including Bypass.
4. **Prose first.** A reply states its answer in prose; the canvas
   illustrates it.

**When agents draw one** ("Visual answers in Hollow", below): plain text for
short or conversational replies, a canvas for status, numbers, comparisons,
plans, timelines and choices, a canvas with an `hc-flow` diagram for
relationships and flows (whiteboards were removed on 2026-10-07), an ask-canvas when the agent needs a visual answer to go on.
How often is the person's Canvases choice. Plain ```html / ```svg fences
still work; they render through the same frame but are never saved.

## The fence

````
```hollow-canvas
<hc-canvas id="release-status" title="Release status" [height="480"] [ask]>
  …HTML, kit elements, scripts…
</hc-canvas>
```
````

Parsed by `src/canvas/fence.ts` (`parseCanvasFence`) while it streams and
re-validated by `src-tauri/src/canvas_fence.rs` before anything is saved.
Both run every case in `src/canvas/fence.golden.json`, which is the spec;
the summary:

| Part | Rule |
| --- | --- |
| Body | At most 128 KB of UTF-8, measured after the streaming strip of a half-typed closing fence. One `<hc-canvas>` root with only whitespace around it; a nested root is refused. |
| Start tag | Closes within 4,096 bytes, at most 32 attributes, names case-insensitive, no repeats. Unknown attributes are ignored (an older Hollow still shows a newer prompt's canvas). |
| `id` | `[a-z0-9-]{1,64}`. Re-emitting the same id makes the next revision. |
| `title` | Required, 1–180 code points, no control characters but tab and newline. |
| `height` | Optional reserved height, 80–1600 px. |
| `ask` | Boolean (bare, `""` or `"ask"`); makes an ask-canvas. `ask="false"` is refused. |
| `section` | Ignored like any unknown attribute: the curated panel view it named is gone with the workspace panel (COO rebuild L4, 2026-11-10). |

States while streaming: `pending` (start tag still arriving), `drawing` (a
skeleton at the root's height), `ready`, or `invalid` with a code and a short
reason. A well-formed canvas never reads `invalid` on any prefix. The inner
HTML is not sanitized: the sandbox is the boundary. Details and the
tokenizer's known disagreements with HTML: [CANVAS_C3.md](history/CANVAS_C3.md#rules).

## Sandbox modes and their CSP

Every artifact frame (canvas, html fence, brief, Result) is
`<iframe sandbox="allow-scripts">` (`SANDBOX` in
`src/components/htmlSandbox.ts`): no `allow-same-origin`, popups or forms. The
document is served from `artifact://` by `src-tauri/src/artifacts.rs`, which
owns the policy: the frontend names a mode, never a CSP string. The policies
are pinned in `src/components/artifactCsp.golden.json`, tested from both
sides; read them there, not here.

| Mode | Scripts | Network | Opens by default for |
| --- | --- | --- | --- |
| Offline | host runtime and kit only, by this document's nonce | none | HTML from outside: html fences outside quick and workspace chats, `.html` files not written by an agent in Hollow |
| Images | same | images | only by the person's grant (locked + a refused image) |
| HTTPS | same | HTTPS images, styles, fonts, media | only by the person's grant (locked + a refused fetch/font/style) |
| **Canvas** | `'unsafe-inline'`, no nonce or hash (either would disable it) | **none** | every `hollow-canvas` fence; html fences in quick and workspace chats; `.html` the agent wrote this conversation or under a workspace's `outputs/` |
| Interactive | `'unsafe-inline' https:` | HTTPS | only by the person's grant (scripted + any refused remote load) |

**No mode buttons (10-11 B1).** No surface shows the modes. The default comes
from the surface (`htmlFenceMode`, `CANVAS_FENCE_MODE`, `fileMode` in
`src/canvas/canvasGrants.ts`). The host runtime listens for the browser's own
`securitypolicyviolation` and posts the first refusal of each kind as bridge
`blocked` (`images`, `network` or `scripts`, plus the host name only). The
host then shows one permission line ("This canvas tried to load images from
x.com. Allowing lets it load from and send to any website." **Allow** · Not
now). Allow takes the smallest policy that covers it (`escalate`): anything
scripted plus any network is Interactive, because a scripted page with remote
images can already send data out. The grant is remembered in localStorage
(`hollow.canvasGrants.v1`, newest 300) for that canvas **and its exact
content** (FNV hash of the body): a new revision or an edited file asks again.
Keys: a saved canvas is `canvas:<workspace or session>:<id>`, shared by its
reply and Results (none before a new chat has a store); a file
is `file:<path>`; anything else the content key. A grant never lowers a
scripted default, and a view whose key, default or content changes (the
preview pane switching files) starts from the new one's own. The head always
shows the boundary as a small tag, before anything is refused (plan L3 X7,
2026-10-14): **Scripts on · no internet** (Canvas), **No scripts · no
internet** (Offline, and the browser fallback), **Internet on (you allowed
it)** while a grant is on, **Read-only local data** on a pin (`permissionTag`,
`src/canvas/hostContract.ts`, read from the same mode the grants set). The named host is the frame's own report
(normalised: lowercase, no trailing dot), so it is only ever shown as text.
`fileMode` refuses `.`/`..` path segments; it does not anchor to the vault
root, so a folder outside the vault laid out as `workspaces/<x>/outputs/`
also opens in Canvas (still no network).

A canvas's **⋯** menu holds View source (⌥⌘U: the frame forwards the chord as
bridge `source`), Copy source, Read as text, Export canvas… (workspace
canvases, pins excluded), Compare
versions in the gallery (where the host offers it), Allow internet / Turn
internet off, and on a locked fence Run scripts. `HtmlDoc` (the Preview pane, briefs) has no
menu: its source view belongs to its host pane. It shows the line, plus a
"Turn … off" line while a grant is on.

- **Nonce, never hash** in the script-free modes: a hash-source also admits
  an external `<script integrity>` carrying that hash (the SRI repair,
  [CANVAS_C2.md](history/CANVAS_C2.md#sri-repair-2026-10-05)). The document arrives
  as host-built parts and Rust places a fresh 144-bit nonce between them, so
  agent text can neither carry nor receive it.
- Every response sends `X-DNS-Prefetch-Control: off`, `Referrer-Policy:
  no-referrer` and `nosniff`. The host runtime
  (`src/components/sandbox-doc.js`, first script in every document) removes
  the WebRTC peer constructors and captures the host window before any
  canvas script runs.
- Links: a frame's `open` is taken only while that frame has focus, at most
  three a minute (`FrameOpenGate`, `src/linkGuard.ts`); from a scripted
  frame (Canvas, Interactive) it also needs the host's confirmation naming
  the full address (`openFrameLink` in `src/store/browser.ts`). Navigating
  any frame of the main window to a foreign URL is blocked
  (`src-tauri/src/main_nav.rs`).
- `artifacts.rs` keeps at most 64 documents in memory (oldest out); a frame
  that remounts stores its document again.
- Outside Tauri (the browser harness) there is no `artifact://`, the frame
  falls back to srcdoc, which inherits the app CSP, and Canvas renders
  Offline and says so in the permission line's slot. The fallback seals its own nonce too.
- `'unsafe-eval'` is blocked in every mode.

## The bridge (v1)

`postMessage` is a frame's only channel. `src/canvasBridge.ts` is the typed
contract: `{__hc: 1, kind, …}`, accepted only from that frame's own window
and the `null` origin, at most 64 KB as UTF-8 JSON, exact fields per kind.
The limits are `LIMITS` in that file.

| Direction | Kind | What |
| --- | --- | --- |
| child → host | `size` | reported height (the inline frame shows at most 640 px; Expand and the panel show it whole) |
| child → host | `settled` | height after fonts, mutation quiet and animation frames; capture uses it, and a settled reply's current canvas then asks for its design check (`design-check`); other hosts ignore it |
| child → host | `error` | a runtime error (≤500 chars): the canvas shows it with **Ask to fix** |
| child → host | `navigate` | a vault path (`hc-file`). `HtmlDoc` (briefs, preview) matches it by note name. `HtmlArtifact` (chat, Results) opens it in the Preview pane (`openFrameFile`): only while the person is using the frame (focused, user activation, ≤3 a minute), resolved against the conversation's folder (a curated view: its workspace) like a reply's file link (a Result: its workspace), and only inside that folder or the vault (`canvasFilePath`) |
| child → host | `open` | a web or mail link, confirmed as above |
| child → host | `blocked` | the frame's first CSP refusal of a kind (`images`, `network`, `scripts`) and the refused host name only (10-11 B1): shown as the permission line |
| child → host | `source` | ⌥⌘U typed inside the frame: toggle the source view (10-11 B1) |
| child → host | `state` | the person's view state, ≤16 KB, own canvas only, only while the frame has focus |
| child → host | `draft` | an ask-canvas's answers (≤16 KB, summary ≤500, the `collect` nonce), own canvas only, gated by `draftRefusal` |
| child → host | `ask-invalid` | the reply to `collect` when `count` answers need attention (the frame marks them) |
| child → host | `design` | the design check's one measured report (repo ideas 2026-10-27 #1): at most 24 items (`contrast` with its ratio and the 4.5 or 3 it needs, `small-text` with its px size, `texture-text` with the ratio it needs and, since repo ideas 2026-10-27 #4, `box`: its own text lines as `[x, y, w, h]` in whole document CSS px within 100,000, or null), each with ≤80 characters of the element's words, plus `scanned` (≤1,500), `more` (≤100,000), `complete` (a boolean: every text element looked at), and since #4 the frame's `width` (≤8,000 CSS px) and whether `texture` was allowed, so Measure on the picture can draw it again; only as the reply to this load's `design-check` ticket. Untrusted: a canvas script can post one too, so it is bounded and only ever shown as advisory text |
| host → child | `theme` | live tokens (`THEME_TOKENS`, 46 of 48), including text scale, and colour scheme on any theme change |
| host → child | `init` | once per load: canvas id, kit version, locale, reduced motion, saved state |
| host → child | `collect` | Hollow's Send was pressed: hand over the answers (a fresh 32-hex `nonce`) |
| host → child | `answered` | the ask-canvas is locked; the sent lines (≤2,000) |
| host → child | `design-check` | measure once (a fresh 32-hex `ticket`, never called a nonce: that word must not appear in the runtime): sent once per load after `settled`, only for a settled reply's current canvas |

**No local video in a canvas (2026-10-17).** A canvas still can't embed a local video: every artifact mode's
`media-src` is `data: blob:` (plus `https:` in Https and Interactive), never the app's `hollow-media:` scheme, and
that stays so. Name the file with `<hc-file path="outputs/clip.mp4">`: `navigate` opens it in the Preview pane,
which plays it (`MediaPreview`).

`motion` (host → child) carries `{reducedMotion: boolean}` for the combined live
app/system preference, matching `init`; host system changes repost it to mounted frames.
Only parent messages are accepted. System reduced-motion is also observed directly by the kit. **Refused messages are logged**
(C6 P1): each refusal of a frame's own message becomes one line in
`{app data}/canvas-bridge-refusals.jsonl` (kind, size, reason, canvas id,
conversation; never the payload or any canvas text), at most 30 a minute,
rotated at 64 KB into one `.1.jsonl` (`src/canvas/bridgeRefusals.ts` →
`record_canvas_bridge_refusal`, `canvas_bridge_log.rs`). There is no UI
count; read the file to diagnose a misbehaving canvas.

Inside the frame, scripts use one frozen global, `hollowCanvas`: `init`,
`get(key)`, `set(key, value)` (posts `state` after 120 ms), and the
`hollow-init` / `hollow-theme` events
([CANVAS_C2.md](history/CANVAS_C2.md#canvas-state-and-init-canvas-c3-wave-2)).

## Workspace canvas look

A workspace row's right-click or ⋯ menu → **Canvas look…** selects a custom
theme for its live frames and optional layout-correction pictures. The lazy picker previews stats, a chart and a
callout, shows palette swatches and contrast adjustments, and saves or removes
the preference. Host chrome keeps the app theme. Built-in Chats and Main
agent menus do not offer this setting. Export offers that workspace look in its Look selector.

`WORKSPACE.md` frontmatter stores optional `canvas_theme`; the local write
`set_workspace_canvas_theme(id, theme?)` preserves other metadata, body and
mtime, including for legacy `notion-*` and connected workspaces. A named theme must exist
on this Mac; removing it needs no theme file. A missing theme uses the app
palette, with a notice in the workspace menu and picker. External theme edits
are refreshed when the custom-theme list is refreshed (including opening the
picker or Settings); there is no filesystem theme watcher.

`customThemeVars.ts` shares the app palette derivation. Host-only `canvasLook.ts`
maps every frame token and moves failing foreground colours toward black or
white in 5% steps: ink 4.5:1 on canvas and surface, body/muted 4.5:1 on surface,
primary 3:1 on surface, on-primary 4.5:1 on primary. Tone and chart series colours
stay unchanged. Short hex colours are normalized; invalid or uncheckable values
and unattainable targets are reported in the picker. App font and text-scale
tokens stay live, even with a named Look. Default canvas and HTML-document
bodies scale from 14 and 15 px; scale changes update mounted frames without
reloading their state. Contrast/transparency media changes also refresh tokens.
`store/canvasLook.ts` caches by name, refreshes on theme-list changes while
retaining the current look, and rejects stale reads. The math loads on demand;
the eager menu signal in `store/canvasLookTarget.ts` does not load the cache.

Reply, Results, gallery, pins and Preview pane frames receive
their workspace via `lookWorkspace`. `watchFrameTheme` posts the current palette
on load and reactive changes, without reloading the frame or its saved state.
Identical palette/motion messages are deduplicated per frame. HTML and Mermaid
file previews use their conversation workspace's look too.
An app-theme switch leaves a workspace look intact. Source, kit, runtime and
always-on prompts are unchanged. Native WKWebView, glass and VoiceOver checks
remain separate from mock browser checks.

## A canvas on a tracker issue (A19)

Mermaid flowcharts also come back into Hollow through reply fences and `.mmd`
file previews ([Chat rendering](#chat-rendering)); that is a local read-only
conversion.

A saved canvas's **⋯ → Attach to issue…** (Results, the gallery) puts a copy
of the version shown on a tracker issue (`tracker.attach_canvas`, no card since
T1, 2026-11-15); the canvas stays in Hollow. The contract is in
[Tracker sync](TRACKER_SYNC.md#status-and-canvas-copies). It replaced
**Add to Notion page…** (10-11 B3: `toNotion.ts`, `notionSends.ts`,
`notion_append_canvas`), removed with Notion sync on 2026-11-10; the plain
canvas model it walked lives on as `src/canvas/canvasModel.ts`.

## Export a canvas as image or PDF

A workspace or Chats canvas's **⋯ → Export canvas…** opens one
size/look picker. PNG and PDF save under `outputs/exports/` and appear in Results;
PNG can also go to the clipboard. **Reveal in Finder** opens the saved location.
**Show the file** opens that exact exported file in the Preview pane, with its review bar (`openWorkspaceResult`).
When the saved markup contains an `hc-flow`, Format also offers **SVG flow**.
Choose a flow; each save writes one `-flow-<n>.svg` file. This is a static drawing
of that flow's saved text, with resolved colours from the selected look. It omits
the rest of the canvas, custom CSS, script-generated changes and saved pan/zoom.
Choose another flow to save it separately. The SVG title and description retain
labels and connections; kit truncation and undrawable connections produce a warning.
Failures use Hollow's shared plain-language error display with technical Details.
The palette accelerates the last workspace canvas used. Inline, expanded,
Results, gallery comparisons and older revisions share the dialog.
A pin with live values withholds export; conversation-only canvases explain
that they need a workspace first.
Export preserves an explicit empty frame state; historical comparison revisions
never fall back to the current revision's saved controls.

Sizes: 600 (Email), 800 (Doc), 1280 (Slide, at least 720 high) CSS pixels.
The filename preserves id, displayed revision, size, dark look and an optional
sanitized theme-name suffix (ASCII letters/digits/hyphens, at most 48 characters;
names with no ASCII letters/digits use `theme`). Collisions
get suffixes `-2` through `-50`, never overwrite. PNG stops at 8,000 CSS pixels
high; PDF at 14,400. PNG reports its actual pixel/CSS scale. Light/Dark use the
stock palette; App theme uses the current app palette. **This workspace's look**
uses its saved custom theme, or App theme when no preference exists. Named custom
themes can be chosen for just this export. Full derived pairs and contrast
adjustments are shared with live frames; the dialog reports adjustments and
unmeasured contrast. An unavailable theme blocks saving with a visible explanation;
choose App theme explicitly to fall back. Named themes are read again at save time,
so removing a file after preview cannot silently export a stale cached palette.
The choice never edits the workspace preference, canvas source or saved controls.
Export always uses Canvas
mode with no network, ignoring internet grants. Ask-canvases cannot send,
navigate or persist state from the capture host.

`canvas/CaptureHost.tsx` is storeless and loads its sandbox through `artifactDocUrl`,
never srcdoc. `canvas_export.rs` validates the full fence, state and destination.
The request's optional `themeName` labels its full `theme` palette (only with
`look: match`); the backend bounds it and sanitizes the filename. Capture filters
pairs through the bridge validator, bakes them before canvas scripts run and posts
the same pairs at init. PNG, PDF and flow SVG share the palette selection.
Each job has a unique child-webview label; only that label obtains or completes
the job, and only `main` starts exports. The capability has `core:default` only,
with guarded navigation. Wry passes every frame URL without a frame identity,
so the guard admits the exact host URL plus only this job's registered
Canvas-mode artifact URL. The iframe cannot navigate the top frame or open popups.
One export at a time, a 15-second capture timeout and
cleanup on every exit. Files are written only after capture. Clipboard failure
returns the saved file plus a warning rather than hiding a successful write.

SVG uses the lazy host module `canvas/flowSvg.ts`: inert-template extraction,
`parseFlow`/`layoutFlow`, and `kit/flowPath.ts` shared with the live renderer.
It never serializes agent HTML. Stock colours come from `styles/tokens.css`;
App/workspace/custom looks resolve colours to numeric sRGB. Rust uses the existing
quick-xml dependency graph to validate a bounded 2 MiB static SVG subset:
no scripts, foreign content, event handlers, external references, DTDs or CSS URLs.
The existing `canvas_export` request adds optional `svg` and one-based `flowIndex`,
valid only for SVG file exports; PNG/PDF capture and PNG attachments are unchanged.
Native SVG opening in Safari/Keynote remains a hand check after installation.
`flowSvg.golden.svg` is shared by the host test and Rust validation; update it
alongside intentional changes to the static drawing contract. It is loaded as
raw data so frontend tests need no Node filesystem types.

Bridge `settled {h}` uses the size-message bounds. After init, the runtime waits
for fonts, 250 ms without mutations and two animation frames (a 4-second fallback
still waits for frames). Ordinary hosts ignore it. `webkit_capture.rs` shares PNG
encoding with browser Annotate and adds PDF and clipboard support. The offscreen
child feature-detects WebKit's private `_setWindowOcclusionDetectionEnabled:`
selector; if unavailable, export fails with words. A standalone Swift spike on
2026-10-18 painted chart pixels in an opaque sandbox frame and advanced rAF with
that setter. Without it PNG painted but rAF stayed at zero. The literal KVC key
raised NSUnknownKeyException, so use the checked selector, not KVC.

**Not checked by hand:** in-app capture, tall canvases, PDF text selection,
clipboard paste, Retina/non-Retina scale, and whether anything flashes with
AeroSpace on/off. Mock smoke checks exercise requests, not native capture.
Layout **Ask to fix** re-reads the workspace look for its optional PNG, then
rechecks the existing conversation/provider/draft guards before starting capture.
If that palette cannot be read, the complete text draft stays and the picture
failure is disclosed. No picture is queued for a later composer.

## kit@1 and kit-ask

The 3.0 kit uses flat 6px evidence planes, compact ruled tables, square step
markers, recommendation rails and continuous ask-option/rank rows. Native
input semantics, host-owned Send and saved answers are unchanged. ASCII flow
requests produce static dashed routes and square box corners without changing
the graph layout, connection list, hit paths, labels, focus or Fit/zoom.
Dither progress fills stay inside the exact completed fraction; geometry is
clamped, while authored values outside the range remain in the written and
ARIA text. Stripes are clipped, never scaled; only supplemental spoken
percentages are rounded. Start/end flow nodes retain their pill shapes.
Timeline dates, callout tones and table sort state remain exact.

**Texture contract (2026-10-31).** Every canvas uses whole-canvas static dither
by default, including a legacy root `texture="plain"`. `ascii` remains the
alternative. A single graphic's `texture="plain"` still opts out; an unknown
local value fails to plain. No kit@1 element or attribute is removed.
The body field is strongest at edges and the headline, with solid reading
plates. Stats and tone callouts use edge bands; recommended comparisons and
sorted table columns use denser accents. Steps, tabs, details, badges, code
headers, file links and checklists have quiet textured gutters or bands.
Ask choices, multi, rank, matrix and pick use the same tile; the range track
is textured while its native thumb, number box, text and date inputs stay crisp.
Dither fills bar/area/donut charts, the area under lines and sparklines,
progress/bars/gauges, timeline spans, heatmap levels and flow outlines/groups.
Exact chart boundaries use solid ink; source tables and keyboard routes remain.
These graphics keep ASCII's existing alternative or plain rendering.
**Chart fills (2026-11-14).** Bar, area, line and donut series are
opacity-only ordered dither: each series is one theme colour and only its alpha
changes, so light and dark need no second shade. Off cells keep 40% of the on
alpha (no holes, no white specks). Fills are dense at the baseline and thin
toward the value line, under a soft edge (one row at 72%, one at 36%). Series
slots take four looks automatically, in order: gradient fade, hatched, dotted,
solid (slot 5 and up repeat); all four thin toward the value, and solid has no
off cells. A donut sector fades radially: dense at the inner radius, thin toward
the outer edge, which carries the soft edge rows (`ditherRing`). There is no override attribute. Quiet scales chart alpha by 0.6. Bars keep the 1px inner ink outline and lines keep their
ink-ringed strokes and markers. `hc-bars` fades its fill by opacity along the
bar with a soft value end, and `hc-sparkline` puts an opacity gradient (dense
at the floor) under its stipple; both are gradient fades, not Bayer cells, and
both follow Quiet through `--hc-gain` (0.6 under `data-hc-density=quiet`).
Legend and tooltip swatches show each series' look: a 14 by 10 px box with a 1px ink
outline (`data-look` 0 to 3: gradient, hatched, dotted, solid), alone on fill charts
(`data-solo` hides the glyph) and beside the marker glyph on a line chart; plain,
ASCII, Texture Off and forced colours keep the glyph. The dithered data layer is cached in an
offscreen canvas and redrawn only when size, dpr, colours, Quiet, texture or data
change, so a hover costs about 0.2 ms on a 1368 by 400 px chart with 4 series and 60 points
(10 ms without the cache).
Plain, ASCII, Texture Off and every host veto keep the old solid or 10% fills.
Written original; Boring-Software-Inc/dither-kit at `1e7faee` was the
inspiration only and no code was copied.
Gauges add stipple inside the exact solid arc; donut sectors take the chart fill
above and keep surface gaps. ASCII heatmaps use `. : + #` density bins with a legend,
retaining original cells and missing/zero distinctions. Stat
values stay plain. Donuts show positive values
only, explicitly naming missing-only and nonpositive-only data; all source
values remain in Table. Text, controls and saved answers remain plain.
One frame-owned Bayer tile is shared by CSS, SVG image patterns and Canvas2D
pattern handles. Full selects six of sixteen threshold cells; Quiet selects
two. It regenerates only when theme ink, density or DPR changes. Resize/reflow
updates SVG pattern scale to screen pixels. No scroll listener, animation loop,
ResizeObserver or per-tile canvas is added. Live policy changes repaint without
rebuilding controls or answers. The frame shares one texture observer/media subscription.
The host runtime exposes a frozen, read-only `hollowCanvas.textureAllowed`
getter, false until a parent `motion` message explicitly grants it via boolean
`textureAllowed`. Its read-only `textureLevel` is Full unless the parent sends
Quiet. Reduce motion (app or OS), Reduce transparency, Increase
contrast and forced colors veto decoration. `hollow-texture` announces policy
changes; renderers must redraw without replaying entrances or changing state.
Existing accessibility hatching remains independent. `textureOf` also checks
frame media preferences. No animation loop or new dependency is introduced.
The shared host `data-texture-active="false"` and `data-texture-level="off"`
also veto frame decoration. The level is bootstrapped before every window
renders, including storeless windows, and observed by mounted frames; Off
therefore applies before the main policy controller mounts. Interface owns
the preference; ambient motion remains separate.

The fence source is stored unchanged. `canvasFrameBody` preserves recognized
root variants on a `display:contents` div marked `data-hc-texture-root` inside the frame after the fence
wrapper is removed (author CSS should target components, not `body >`); ordinary
bodies and the wrapper's first/last content share edge margin resets. Ordinary
bodies gain only this presentation wrapper. Revision, gallery and pinned views share
that path. Export preserves the request but its reduced-motion capture policy
renders plain. HTML sanitization still owns active-content restrictions; texture
is never a grant, CSP exception or bridge request from an agent.

Host-owned components, inlined into every artifact document so a canvas is
short and looks like Hollow in every theme. Light DOM, attribute values
written as text only, readable before upgrade and with the script blocked, no
network, no events of their own. Charts draw on a DPR-sized `<canvas>` with a
hidden data table, never use ResizeObserver, and redraw on `hollow-theme`,
`resize` and the kit's `hc-reflow`.

**Look (10-26 B9).** One rhythm (`--hc-gap`, 16px between blocks), cards
with a soft edge (`--hc-card`: inset edge, 1px hairline ring and faint shadow),
radius 12px matching host cards. Existing bridged colors keep frames opaque;
host-only material aliases are never read. Stats use optical/tabular type;
callouts have solid reading planes; chart and comparison heads use dividers.
Completed steps keep an outlined check and the current step is filled.
This is CSS only: kit@1 markup, data, keyboard controls, state,
script hash/CSP and all motion gates remain unchanged.
A toned `hc-stat` shows its tone by a top edge and,
with no delta, its icon and word by the label; a value over 10 characters
steps down a size (`data-long`). Kit tables reset the host base's full cell
borders to row lines with a tinted header. `hc-flow` normally keeps at least 80% of
its size (`--hc-flow-min`; the box scrolls past that); large/interactive flows instead offer Fit and zoom, and a `right` flow
that would shrink past 80% in its width runs down when that is narrower. The
host base (`ARTIFACT_STYLES`) gives author headings and paragraphs a type
scale. Agents get `src-tauri/src/canvas_design_prompt.md` (composition rules
and two exemplars) in the visual guide after the kit reference;
`src/canvas/designGuide.test.ts` checks the exemplars draw. Since repo ideas
2026-10-27 #10 its evidence rule picks the chart from the data's shape: one
value a stat (a gauge only with real bounds: it defaults to 0–100), a few
categories bars, time a line, parts of a whole a donut, else a table of at
most 8 rows; the status exemplar's daily series is a line to match (hallmark's rule written for
this kit, which keeps donut and gauge). It is capped at 2,400 bytes
(`visual_prompt.rs`), so the line replaced most of the texture paragraph,
the guide's least-used text: no `texture=` in the 39 canvases of the busiest
workspace; its contract stays in the Texture contract section below.

- **kit@1** (display): `hc-badge`, `hc-callout`, `hc-details`, `hc-stats` /
  `hc-stat`, `hc-progress`, `hc-steps`, `hc-bars`, `hc-chart` (line, bar,
  area, donut), `hc-table`, `hc-compare`, `hc-tabs`, `hc-file`, `hc-code`,
  `hc-flow` (2026-10-07), `hc-gauge`, `hc-sparkline`, `hc-timeline`, `hc-heatmap` (10-11 V1), `hc-checklist` (10-11 Y4).
  **Frozen API:** [CANVAS_C2.md](history/CANVAS_C2.md#kit1-api-frozen). New elements
  and optional attributes may be added; changing one needs `kit@2`. Budget
  99 KiB (`KIT_BUDGET_BYTES`; 88 KiB before the whole-canvas material, 96 KiB until 2026-11-14, 97 KiB until the
  hover cache), raised for opacity-only chart fills and the four gaps they left (donut fill, Quiet fades, legend looks, hover cache).
  It remains lazy and outside the launch chunk; the ask kit keeps its separate budget.
- **Checklist (10-11 Y4):** `hc-checklist label` takes one text/list row per check: `[x] Done`, `[ ] Pending`,
  optional Markdown bullet; unmarked rows are not done. It is a static report, not a task mutation.
  Inside `hc-ask`, add `name` and optional `required`: every row asks Pass/Fail/Skip, with no choice set by authored marks.
  It composes `hc-matrix` to share validation, draft JSON (`{row: "Pass"|"Fail"|"Skip"}` or null), keyboard radio
  behavior and sent-answer locking. The nested matrix stays connected on wrapper changes; rows refresh, choices/focus/errors remain.
  Literal-star labels stay literal. Duplicate labels keep their first row with a notice; empty lists show a notice.
  Without the ask kit a checklist remains a static report. Source: `kit/checklist.ts` and
  `kit/components/checklist.ts`. Plain checklists use theme tokens and no animation; ask controls use the ask stylesheet.
- **Instruments (10-11 V1):** `hc-gauge label value min max unit tone` (0/100 default range;
  geometry clamps, exact value stays visible); `hc-sparkline label unit` (standard chart rows,
  first series/366 samples plotted, missing values break the line, all values in a table,
  may nest in an `hc-stat`); `hc-timeline label` (`YYYY-MM-DD | Label | optional end`, 60
  valid events/spans, invalid/reversed ranges counted); `hc-heatmap label unit start end`
  (`YYYY-MM-DD: value`, at most 366 days, bounds optional). Heatmaps outline missing days,
  distinguish explicit zero, disclose duplicate dates (last wins), invalid dates and truncation.
  Timeline and heatmap each have one keyboard stop; arrows/Home/End inspect values. All
  instruments have exact text or a data table, and use theme tokens with no layout polling.
- **Chart entrances:** cached data marks reveal once (line/area trace, bars grow from zero,
  donut sweeps clockwise), using only transforms on a temporary SVG image/clip. Axes and
  exact labels remain still on the existing DPR canvas. Hover and keyboard interaction
  settle immediately; theme/size changes retain the final chart. IntersectionObserver and
  visibility pause motion; both reduced-motion gates settle immediately. No rAF paint loop.
  Observer/listener/animation cleanup runs on removal and completion. The bitmap overlay
  is local data: only, and never enters saved source or the state bridge.
- **Flow 2 (10-11 V2):** unrelated group boxes and intermediate nodes are obstacles for grouped-flow routes;
  an orthogonal visibility grid provides eight-unit clearance and six-unit rounded corners. Source/target groups
  permit entry/exit. Twelve-thousand expansions per link and 100,000 per layout bound search work; resize
  measurements skip routing and cache geometry by source text. A route that cannot be found is disclosed rather than drawn through a box. Labels and the full
  text description preserve the connection. Trailing `{active}` on a link (or chain) marks flow motion; hover/focus
  also runs one travelling point (at most eight visible particles; focus/hover takes priority). Points use sampled geometry and transform keyframes, never a JS frame loop.
  Each node/link has an IntersectionObserver gate; hidden documents pause; both motion gates cancel animations;
  disconnect/re-render removes observers, listeners and animation objects. Active strokes remain when motion is off.
  Big flows (>700 wide or >560 high) and `interactive` flows have a bounded viewport, 1–16× Fit-relative zoom,
  drag pan, arrow pan, +/− zoom and Home/Fit. Links use one tab stop with arrow navigation; focusing a link
  outside the current view restores Fit so it is visible. Wheel zoom requires Control/Command. Zoom is view-only and resets
  when markup re-renders. SVG is a labelled group so focusable link descriptions remain accessible. Native
  WKWebView, VoiceOver and trackpad behavior still need hand checks.
- **`hc-flow`** (diagrams; replaces whiteboards): one line per node
  (`id: Label {shape, tone}`, shapes `start`/`end`/`decision`/`note`), link
  (`a -> b: label`, chains `a -> b -> c`, ends may carry braces) or group
  heading (`[Name]` … `[]`); `direction="right"` runs across. A line is a
  link when its first `->` comes before its first `:`; a link's label is
  everything after the first `:` past the first `->`. Braces stay in the
  label unless every word is a shape or tone. Repeated links merge (labels
  joined with ` / `); a node line naming a link-made node by its label takes
  it over. `src/canvas/kit/flow.ts` (pure: parse; layered layout with
  cycle-breaking and barycenter ordering; with groups, each group and each
  ungrouped node takes the first lane free over its layers, so a box holds
  only its members, boxes never overlap, and groups that follow each other
  share a row; labels and loops count toward the size) and `components/flow.ts`
  (one SVG, `role="group"` named by `label`, roving focus across links, plus a visually hidden list of
  every node and its links). Bounds: 60 nodes, 120 links, node labels
  wrapped to 3 lines, link labels cut at 40 characters (whole in the text
  list); what's left out is counted ("2 nodes and 3 links not shown").
  A forward link that skips layers gets a slot in each layer it crosses and
  is drawn through them, so it goes round the nodes there. Link labels draw
  above nodes, each at the first of a few spots along its link that covers
  no node; in `right` mode the gap after a layer widens to fit its labels.
  Group names cut at their box (240 px at most, widened down the page).
  Limits: ungrouped kit@1 flows keep their existing curves; a back link in an ungrouped flow can cross a node; ungrouped sibling link labels can overlap. Grouped labels avoid nodes, group headers and earlier labels;
  labels that cannot fit stay in the named link and a disclosed, expandable connection list; groups that share layers sit in separate lanes, which widens the diagram. Tests: `kitFlow.test.ts`, `kitFlowRouting.test.ts`, the
  gallery smoke spec's `hc-flow` case.
- **kit-ask** (inputs, only in `<hc-canvas ask>`): `hc-ask` with Send, and
  `hc-choice`, `hc-multi`, `hc-range`, `hc-rank`, `hc-text`, `hc-date`,
  `hc-matrix`, `hc-pick`. API and answer shapes:
  [CANVAS_C2.md](history/CANVAS_C2.md#ask-inputs-the-kit-ask-bundle-canvas-c4-a1-additive);
  bridge contract: [CANVAS_C2.md](history/CANVAS_C2.md#bridge-draft-and-answered-canvas-c4-a2).
  Budget 25 KiB of its own (`KIT_ASK_BUDGET_BYTES`), including static field material.

Source `src/canvas/kit/` (and `ask/`); `npm run kit:build`
(`scripts/build-canvas-kit.mjs`, then `scripts/sync-sandbox-csp.mjs`) writes
the committed `generated/` files and re-syncs their hashes into
`htmlSandbox.ts` and the app CSP (for the srcdoc fallback only; no served
policy lists a hash). `scripts/canvas-kit.test.mjs` enforces the budgets,
staleness, determinism and the no-network / no-`eval` / no-`innerHTML`
policy. Manifests: `src/canvas/kit/manifest.ts` (`KIT_VERSION`,
`KIT_COMPONENTS`) and `src/canvas/kit/ask/manifest.ts`; the prompt tests
read both.

## The store

Every canvas a live turn shows is saved (`src-tauri/src/workspace_canvases.rs`;
commands in `commands/canvases_api.rs`). Viewing never
writes, and nothing here writes WORKSPACE.md, PLAN.md or SOURCES.md.

| Where | Files |
| --- | --- |
| A workspace (Chats included) | `outputs/canvases/<id>.canvas.html` (the current fence body as written), `<id>.canvas.json` (the sidecar and commit point), `outputs/canvases/.revisions/<id>/<sha256>.canvas.html` (every kept body) |
| Any other conversation | the same layout in `<app data>/canvases/<sessionKey>/` (owner-only), newest 500 conversations; pruned bodies remembered in `.pruned` (newest 8,192) |

- **Sidecar** (`hollowCanvas: 1`): `id`, `title`, `kit`, `session`,
  `fenceSha256`, `createdAt`, `updatedAt`, `revision`, `state`,
  `stateSavedAt`, `revisions` (≤30: revision, hash, title, saved at, `ask`,
  `height`, session, turn id), `retired` (aged-out numbers and hashes, newest
  500). An older sidecar's `curated` field (from the retired curated views) is kept as unknown data. Unknown fields survive a save; a
  newer format or a damaged sidecar is refused, never overwritten.
- **Revisions.** A new body is the next revision; a body the store has seen
  (kept or retired, or from a pruned conversation) writes nothing, so
  scrolling through an old chat never rolls a canvas back. Ids are unique
  per workspace: two conversations using one id share a canvas.
- **State** belongs to the canvas, not a revision, so a re-emitted canvas
  opens as the person left it (≤16 KB, `save_canvas_state`, user-only).
- **Compare-and-swap** on the sidecar's SHA-256 for every write
  (`CANVAS_CHANGED:` on a race), atomic writes, no symlinked folders or
  files, `run_guard::is_protected_target` on workspace writes. An outside
  edit of the visible copy is kept as `outside-<sha>.canvas.html` before the
  next revision.
- **Results.** A saved workspace canvas is one Result of kind `canvas`
  ("Canvas v3"); the sidecar and `.revisions/` are never listed or
  reviewable; acceptance is person-only and a new revision reads "Changed
  since accepted".
- **The ask lock is not in the store.** It is read from the transcript (the
  person's sent answer message); see Ask-canvases.
- **Retired curated views:** the reserved ids `_overview`, `_brief`, `_plan` and `_sources` (`RETIRED_CURATED`) may remain in older workspaces; they are not listed, not Results and nothing reads or writes them (COO rebuild L4, 2026-11-10).

Commands: `list_canvases`, `read_canvas`, `read_canvas_revision` (read),
`record_canvas` (plumbing: the renderer records the agent's own fence),
`save_canvas_state` (user-only). Errors start
`CANVAS_INVALID:`, `CANVAS_CHANGED:` or `CANVAS_RETIRED:`. Full table and
the sidecar rules: [CANVAS_C3.md](history/CANVAS_C3.md#the-store-s2).

## Chat rendering

- Reply local images resolve against the conversation folder (vault fallback)
  through `chat_preview_file`, with a 10 MiB byte-read cap. The thumbnail is a
  keyboard-operable Preview pane opener; missing images name the file, while
  unsupported, oversized or unreadable files say unavailable. Two local reads
  run at a time across reply segments; queued reads recheck ownership and stale
  completions cannot decorate another conversation.
  SVG keeps its existing separate preview policy. `~/` links use Tauri's OS
  home, not Hollow's configured home. Unsupported non-web schemes stay in
  Hollow with an explanation; web-link tooltips name the URL and Command-click.
  Reply links are underlined in every theme, independently of hover or color.
- Remote images remain blocked on render. Each placeholder or **Load N images**
  explicitly requests the existing bounded/coalesced backend loader. Partial
  failures remain visible and retryable. Other Markdown hosts keep their policy.
- Code info strings take their first word as the language. Reserved Hollow
  fences still need a bare language; attributed ones remain visible source.
  The visible parser and both TS/Rust hidden-block readers pair ordinary
  attributed/no-space info strings the same way, preserving later suggestions.
  Code initially shows 24 lines, with **Show all N lines** and **Show fewer
  lines**; expansion survives streaming. **Copy [language] block** always copies
  the full source. Expanded Bash rows separately copy the command and output.
- The lazy reply decorator has a synchronous sanitized Markdown fallback.
  It preserves prose height, substitutes inert text for all images and blocks
  links until the decorator is ready; raw file URLs never reach the browser.
- Failed/changed video cards offer **Try again** and keep **Open in Preview pane**.
  A retry resets the automatic retry allowance; a path/session change releases
  old grants and discards late completions.

- **Flow layout check (R11):** a settled, current saved reply canvas with a
  composer shows collapsed layout findings below the frame, inline and expanded.
  Results, gallery, pins and earlier revisions have no audit controls. The lazy
  host audit estimates the authored direction, not runtime CSS or narrow-frame
  direction fallback; findings say "may". It detects links crossing unrelated
  nodes, link-label overlaps, shortened labels, and connections kept in the list
  or beyond kit limits. At most 20 findings plus an omitted count; eight flows
  per canvas; a 64-entry text/direction cache. Over 30 nodes or 60 links requires
  **Check layout** before routing. Nothing changes the kit or agent prompt.
- **Design check (repo ideas 2026-10-27 #1):** the same collapsed row also
  judges the parts of the canvas the agent wrote by hand, never the kit's
  elements. It is one row per canvas: "1 layout problem in "Flow"" with only
  one diagram's findings (as before), else "N layout problems", "N design
  problems" or "N layout and design problems", with a heading per part.
  Advisory only: it never blocks, hides or changes a canvas, and its rules
  are Hollow's own, for every provider (no marketing rules: Inter stays).
  - *Source pass* (`src/canvas/designAudit.ts`, pure): literal text colours
    (`color:` that names no `var()` token; fail), px/pt font sizes under
    11px (fail), `transition: all` or a transition with no property
    (advisory), hand-written `@keyframes` or `requestAnimationFrame`
    (advisory), emoji used as icons (emoji presentation, or a pictograph with
    U+FE0F; advisory), and inline SVG with no `aria-label`, `aria-labelledby`
    or `<title>` that isn't `aria-hidden`/presentation (advisory). Comments,
    scripts and attribute values are never read as CSS.
  - *Measured pass* (`sandbox-doc.js` §7, bridge `design-check` → `design`):
    one pass after the frame settles, for a settled reply's current canvas.
    It walks at most 5,000 elements and measures at most 1,500 text
    elements, skipping kit elements and their `data-hc-part` parts, script
    and SVG content, unpainted or 1px boxes and anything a running animation
    touches. Contrast is WCAG's, a glyph pixel against the ground beside
    it, both composited outwards through every box around the text as the
    page paints them (each box's background under what's inside it, then
    its opacity on the whole group), over the host's `--app-canvas`; any
    colour resolves through a 1×1 canvas. 4.5:1 is needed, 3:1 for large text
    (24px, or 18.66px bold); a ratio within 0.5 of it is advisory, below
    that a fail. Text over a background image or gradient (its own or an
    ancestor's), a positioned `::before`/`::after` pattern, `background-clip:
    text`, transparent text, or a positioned image, canvas, video, SVG or
    patterned box covering 30% of it is **unresolved**: never guessed. Once
    it reports a **complete** pass (every text element looked at: no cap
    reached, none skipped mid-animation, nothing thrown), its rendered sizes
    replace the source pass's small-text guess (an overridden or unused
    declaration isn't a problem); after an incomplete one the source's
    warning stays, as unresolved and "unverified".
  - *Measure on the picture* (repo ideas 2026-10-27 #4, after impeccable's
    hide-text pixel diff): with unresolved texts that have a box in the row,
    this button (the person's click only) asks Rust `canvas_measure` (main
    window, `user-only`, never an agent action, nothing saved) to draw the
    canvas four times through export's capture child (`run_capture`: one at
    a time, 15 s each, its view closed after), as the reply's frame drew it:
    its width, the workspace's canvas look (`exportPalette`), its state, and
    texture as the frame allowed it (texture shows only with motion on, so
    the capture host sets both from it). Every pass freezes animation and
    transition; the text is drawn as it is, transparent, black and white by
    setting only `-webkit-text-fill-color` (never `color`, so anything painted
    in `currentColor` stays put), shadows, strokes, decorations and emphasis
    marks out of the way (`CaptureHost.tsx` `measureCss`, id-weighted to
    outrank the agent's `!important`; an inline `!important` still wins). The
    four pictures must match in size, each at most 12 M device pixels
    (`MEASURE_MAX_PIXELS`, checked from the PNG header in Rust and again
    before decoding). The main window decodes them one at a time and keeps
    only each box (`measurePicture.ts`), then scores it (`pixelContrast.ts`,
    pure): where black and white text differ by 75%+ is the text's coverage,
    found apart from its colour so glyphs that vanish into their ground
    count; each covered pixel's ratio is WCAG's between the first picture
    and the transparent one, and the text scores the **lower quartile**, so
    a quarter of its glyphs below the need fails it; fewer than 8 covered
    pixels stay unresolved. All boxes together may cover 2 M device pixels
    (`CROP_MAX_PIXELS`; later ones stay unresolved, "too large"). The frame
    reports a box only for an element whose text is all its own: the union
    of its own text nodes' line boxes, inside the document; none when a
    child element has text, a form control sits inside, its glyphs paint a
    clipped background (`background-clip: text` in any layer, which a
    transparent fill doesn't remove), or any animation but the kit's own is
    in effect, running or finished and filling forwards (the capture
    freezes animation, which may lay things out differently). The kit tags
    its animations `id: 'hc-kit'` (chart entrance, flow motion: transform
    and opacity only, inside its elements). Such a text stays unresolved
    and says why. A passing text leaves the row, a failing one
    becomes "measures N:1 on the picture" (fail, or advisory within 0.5),
    and the counts follow; a note about the measurement belongs to the
    report it came from.
  - Findings are ordered fail, unresolved, advisory; at most 5 are listed
    (each with a one-line fix) plus "And N more design problems." The
    reply's receipt says the same count in its Canvases row ("1 canvas · 2.4
    KB · 3 design problems") while that findings row is on screen
    (`src/canvas/canvasProblems.ts`, keyed by run; a row that unmounts takes
    its count with it); Activity's copy of the receipt has no count, which
    reads as nothing, never zero. The Ask to fix draft keeps a share of its
    2,000 characters for each part, so many layout findings never push the
    design findings out.
- **Ask to fix** appends a plain correction draft of at most 2,000 characters
  with the same canvas id. It never sends or replaces existing draft text. A
  workspace capture adds a PNG through the existing composer image sink for
  Claude/Codex; AgentDescriptor currently has no image capability field, and
  Annotate has no provider check, so other providers conservatively get text.
  Capture failure leaves the text usable. A changed conversation, provider,
  revision, draft or a send while capturing discards the late picture. A missing
  composer sink reports the text-only fallback; captures never enter the global
  pending-image queue for a later composer. Native
  capture, VoiceOver and real follow-up fix turns remain separate hand checks.

Mermaid flowchart fences in finished replies and `.mmd` / `.mermaid` files
in the Preview pane draw as read-only `hc-flow` diagrams. The lazy host converter
(`src/canvas/importFlow.ts`) accepts at most 64 KB / 2,000 lines, keeping node
labels, connections and groups; the fidelity line counts omitted styles, click
handlers, unknown statements, flattened groups and the kit's 60-node/120-link
limits. BT/RL are drawn top-down/left-right and say so. Other Mermaid kinds
stay source with a reason. While streaming, all Mermaid stays code.
`MermaidDiagram.tsx` uses HtmlArtifact in replies (including its frame budget)
and HtmlDoc in the Preview pane. View source / Copy source retain the original Mermaid;
Read as text reads the converted connections. Reply diagrams are always Offline
and omit script/network grants. They do not create saved canvas revisions.
The kit still runs by nonce in Offline; no Mermaid runtime, prompt additions,
new model call or kit bytes are involved. Notes, draw.io and Excalidraw remain
outside this converter. Native VoiceOver and real `.mmd` opening need a check
after a separately requested install.

`chatSegments.ts` turns the fence into a `canvas` segment;
`src/canvas/chatCanvas.ts` (`chatCanvasView`, `resolveCanvas`) and
`src/components/ChatCanvas.tsx` draw it inline in every chat, workspaces
included, head "CANVAS · *title* · v3", in Canvas mode.

- **Skeleton** "Drawing *title*…" while the fence streams; a reply that
  settles with the fence still open is read as ended, so a truncated canvas
  is broken, never a skeleton forever.
- **Broken** (`invalid`, or a bridge `error`): the reason, the source and
  **Ask to fix**, which fills the message box with a one-line request naming
  the id. It never sends.
- **Saving** happens once, when the fence becomes ready **in the window that
  shows the live turn** (`liveRun`), and again at the turn's end in the
  backend (`workspace_canvases::record_reply`, 2026-10-14), which saves every
  closed fence of a reply no window watched (an unattended check-in) and
  leaves alone a canvas the window already saved for that turn. Re-renders, reopened conversations and
  the Expand copy only read.
- **Revisions:** "v3"; an older reply's canvas reads "v1 of 3" and says
  so; the label opens the version list, and an earlier revision renders
  read-only from `.revisions/`. Only the frame showing the current revision
  saves state (debounced 400 ms, CAS, focus required).
- **Frame budget:** at most six live frames per transcript
  (`FRAME_BUDGET`, `src/canvas/frameBudget.ts`); frames beyond a 600 px
  margin unmount to a placeholder at their last height and remount with
  `init` carrying their state.
- **Expand** opens the canvas full-height in the Preview pane.
  The pane's bar stays above the canvas's own scroll viewport (`src/components/ExpandedCanvas.tsx`).
  The canvas heading starts anchored at the top; resizing holds it until the person
  scrolls. **Back to canvas** returns to the heading without reloading the iframe;
  Enter/Space works; Escape hides the chip until the visual is reopened.
  Focus returns to the canvas.
  Both motion preferences disable smooth scrolling. Other replies leave the expanded
  canvas in place. This view reads existing revisions, never records another one.
  Browser checks use mock Chromium; native WKWebView, glass vibrancy and VoiceOver still need a hand check.
- **Theme** changes restyle mounted frames live (`watchFrameTheme`).

Details: [CANVAS_C3.md](history/CANVAS_C3.md#chat-side-wave-2),
[revisions and state](history/CANVAS_C3.md#revisions-and-state-wave-2-c2),
[frame budget](history/CANVAS_C3.md#frame-budget-wave-2-c3).

## The host contract (plan L3, 2026-10-14)

One saved canvas shows in six hosts: inline in a reply, expanded in the
Preview pane, Results, the gallery (Library → Results → Canvases), a pin,
and an older version opened in any of them (the seventh, the curated section
view, went with the workspace panel in COO rebuild L4).
`src/canvas/hostContract.ts` (`HOST_CONTRACT`) says, for each host, whether
it offers title, version, saved state, expand, pin, compare, copy/export,
the permission boundary, Read as text, answering and Accept,
and *why* when it doesn't; `hostContract.test.ts` checks it and
`tests/smoke/canvas-host-contract.spec.ts` checks the heads. Every host draws
through `HtmlArtifact`'s head (`data-host`), whose words come from the same
module:

| Host | Version on screen | Saved state | Not offered, and why |
| --- | --- | --- | --- |
| Inline | `v3`, `v1 of 3` | Saved in this workspace / Saved with the COO (quick-chats folder) / Saved with this conversation; **Not kept** (older than the 30 kept versions); **Not saved** (no store, or not in it) | Accept (a Results action) |
| Expanded | ActionMenu first line and frame aria-label; ⋯ → Versions and pin… opens version/pin controls | ActionMenu first line and frame aria-label, as inline | Expand (already full height); Accept |
| Results | `v3` and the list | Saved in this workspace · updated *when* | Expand; answering (Results only reads) |
| Gallery | `v3` and the list; the list item says workspace, version and when | as Results | Expand; answering; Accept (Open workspace opens its workspace) |
| Pinned | **Pinned v3 · current** / **Pinned v3 · a newer version exists** | Live · updated, Saved snapshot, Needs refresh | Export (live values are local; open its saved canvas); Compare (a pin keeps one version); answering (Open source chat) |
| Older version | **Older version (v1 of 3) · read only**, plus the existing note and "Back to v3" | the kept version's | Pin, answering and Accept: only the current version |

- **Compact panel heads:** expanded canvases keep only a named permission lock and ⋯ in the top-right corner. Internet permission uses a globe and warning tone. Refusal lines remain visible. The ActionMenu's first line and the frame's aria-label name the canvas; inline replies keep their full head. Expanded ⋯ → Versions and pin… reveals the existing controls, with Escape/Close controls returning focus to ⋯.
  The corner controls overlay about 76 px of the preview's top-right edge; canvas authors should leave that corner clear. Menu bounds wrap within the panel, and only the frame slot clips its corners.
- **Files and Visuals shelves** went with the side panel (run 15 P1). In the Preview pane a file's details (kind, the selected read's real size/time, full path) are its tab's tooltip and rows of its ••• menu, beside Copy path, and the **Files** dropdown (an icon in the pane's one header row since 2026-11-06) switches between the project's notes, a build workspace's changed files and the conversation's files.

- **Compare** lives in the gallery. A reply, the Preview pane and Results have
  no room for two frames side by side, so their ⋯ offers **Compare versions
  in the gallery** (a workspace canvas with two or more kept versions): the
  gallery opens on that canvas with Compare on (`src/canvas/galleryRequest.ts`)
  and a **Back to the conversation** / **Back to the workspace** button.
- **The gallery shows each action once**: Pin, Copy source and Read as text
  in the canvas's head, **Compare versions** as a button. It says what it is for ("Every
  canvas saved in your workspaces, newest first.").
- **Read as text** (every host's ⋯): the stored HTML's words in the page,
  never as markup: headings, paragraphs, lists, tables, a chart or stats as
  their data table, a checklist or steps with Done / Not done in words, an
  `hc-flow` as its links ("Plan → Build: when ready"), an image as its alt
  text; scripts and styles left out (`src/canvas/canvasText.ts`, built on
  `canvas/canvasModel.ts`, loaded on first use). A pin shows its saved
  values, not the live ones.

## Accessibility checklist for generated canvases

What an agent's canvas must do so that a person using a screen reader, the
keyboard, high contrast or reduced motion gets the same answer. The kit does
most of it for free; hand-written HTML has to do it itself. The agent's
short version is in `src-tauri/src/canvas_design_prompt.md`. The person-run
VoiceOver pass is [CANVAS_C6.md](history/CANVAS_C6.md#voiceover-checklist-for-the-person).

- **Headings and landmarks.** The canvas title is the frame's name
  (`Canvas: <title>`); inside, headings go in order (`h2` then `h3`, never
  skipping), and a canvas with several parts uses `hc-tabs` or sections with
  headings, not visual weight alone.
- **A text alternative for every chart.** Use `hc-chart` / `hc-bars` /
  `hc-sparkline` (each draws a hidden data table) or put the numbers in an
  `hc-table` next to a hand-drawn chart. An `<img>` or `<svg>` that carries
  meaning has `alt` / `aria-label`; a decorative one has `alt=""`.
- **Colour is never the only signal.** Tones come with a word or icon
  (`hc-stat` tone, `hc-badge`, `hc-callout` title); a red/green difference is
  also said in text ("2 failing").
- **Contrast through the kit's tones.** Colours come from the theme tokens
  the kit uses (`--hc-*`, `--app-*`), which hold contrast in light, dark and
  high-contrast themes; no fixed hex text colours on fixed backgrounds.
- **Data tables for data.** Rows and columns go in `hc-table` or a `<table>`
  with `<th>` headers, not in aligned `<div>`s.
- **Focus order and no keyboard traps.** Interactive parts are real buttons,
  links and inputs (or kit components, which manage focus), in reading order;
  nothing sets a positive `tabindex`, and Tab always leaves the canvas.
- **Labelled controls.** Every input and button has a visible label or
  `aria-label`; kit-ask fields take `label`.
- **Reduced motion.** Animation only through the kit, which settles at once
  under either motion preference (the frame's `init` carries it); no
  hand-written `@keyframes` or `requestAnimationFrame` loops.
- **Generated controls are never trusted app actions.** A canvas button can
  only (a) fill a draft in the composer, which the person reads and sends with
  Enter, (b) hand its answers to Hollow's own Send under an
  ask-canvas when the person presses it, or (c) ask to open a link or a
  workspace file, which the host confirms by name. It can't send, write a
  file, accept a result, pin, or change a setting; the host draws "Answered",
  "Waiting on your answer", the saved state and the permission tag itself, so
  a canvas script can't fake them.
- **A fallback for everyone.** **Read as text** shows the canvas's words in
  the page (above), whatever its scripts do.

## Ask-canvases

`<hc-canvas ask>` with kit-ask fields. The agent ends its turn after it, as
after a question card.

- **Hollow's Send sends (question cards, 2026-10-14).** The button is drawn
  by the host under the frame (`HtmlArtifact`, words from `<hc-ask send>`,
  default "Send answers"); there is none inside the frame. Its trusted click
  posts `collect {nonce}`. Validation stays in the frame: with errors the
  kit marks the fields and answers `ask-invalid {nonce, count}` (the host
  says "N answers need attention."); otherwise it answers `draft` with the
  nonce. The host takes it only for an ask-canvas, not locked, at its current
  revision, with a conversation to send to, and only as the reply to that
  click's `collect` within 3 s (`draftRefusal`, `unrequested` otherwise), so a
  canvas script can never send by itself. It sends
  (`ChatBody.sendFromCanvas` → the send controller's `sendAnswers`)
  `src/canvas/askAnswer.ts`'s `canvasAnswerText`: `Answered canvas "<id>":`,
  one `- label: value` line per answer (the question-card format, pinned by
  `answerLines.golden.json`) and a hidden one-line `hollow-canvas-answer`
  block holding the exact JSON `{canvasId, revision, answers}`. What the
  person typed in the message box is left alone. A refusal shows a plain line
  under the frame (`canvas/askSend.ts`), with Try again where it can work.
  **P1-33 (2026-10-25):** while the agent is answering, Send reads "Queue
  answers" and holds the answers for after the turn (Queue for after through
  the backend's held send, never Steer; `locked()` is read again just
  before the request goes, not when the backend sends the held message
  after the turn: once held, Send is off in every copy, so it is the only
  copy); the line says "Queued…" and Send stays off until the held send goes
  (the lock then hides it) or fails. A Send from one copy sets a `sending`
  hold before it starts, so the other copy's Send is off meanwhile. Send now
  collects the form again (answers changed since are sent). A held answers
  turn that is stopped or ends before it starts also comes back to the
  canvas (`answersBack`), never the box. A send that doesn't go never
  puts the answers in the message box: the Send group becomes a callout
  (`data-unsent`) with **Send now** (collects that copy's form again) and **Take
  them back** (Send returns; the answers stay in the form). Both states are
  one hold per conversation and revision (`canvas/askHold.ts`), so the
  transcript's copy and the Preview pane's show the same and send once; a held
  answers send that fails or is cancelled from its chip settles the hold,
  not the box (`returnHeldToBox`). Send is off (`aria-disabled`) from the
  click until the send settles, collect included. An earlier revision (this
  reply's fence older than the store's current one) shows no Send at all,
  only **Go to vN**, which scrolls to the current revision's fence and
  focuses its Send (or its callout's first button); when that reply isn't in
  this transcript, a line under the button says to scroll to it or reply in
  the message box. A restored `hc-matrix` keeps only answers in its current
  columns, and a restored date only an ISO date. The fields keep what
  the person chose in the canvas's saved state as they answer (the ask kit's
  `AskField._changed` → `keepView`, key `hc:<field tag>:<index>`; restored by
  `_restore` on load and on `hollow-init`), so a frame that loads again
  (scrolled far away, Read as text, a grant, a redraw after a failed send)
  shows the same answers; once answered, the host drops those keys from the
  saved state. The host takes a frame's state only while the person uses it,
  plus 600 ms after they press that canvas's Send (`STATE_GRACE_MS`: the
  runtime's 120 ms debounce means an answer chosen just before arrives once
  focus is on the button). The kit build mangles `_` members, so the kit
  reaches them by dot only (`kitAsk.test.ts` refuses a quoted `_` name). A pointer click on Send in the half
  second after the frame's drawn height changed (`tooSoon`, shared with the
  question dock) is refused with a line saying so, since the button just
  moved; a key press on the focused button is not;
  there is no copy-by-hand box any more. Before 2026-10-14 Send only filled
  the composer, and a canvas from a turn no window watched was never saved,
  so every Send was refused `not-current` and the person copied the answers
  by hand ("Hollow didn't take the answers").
- **Only the kit hears `collect` (overnight 10 A7, 2026-10-20).** The host
  takes the first valid draft carrying the click's nonce, so a canvas script
  that heard `collect` could answer first with answers of its own. The ask
  kit's `message` listener is added before any canvas script and in the
  capture phase, so it runs first, and for a host `collect` it calls
  `stopImmediatePropagation()`: a listener a canvas script adds, in either
  phase, no longer hears `collect`. `init`, `theme`, `live-data` and
  `answered` pass on as before.
  **Not fixed:** a script in the same frame can still patch prototypes:
  redefining the `MessageEvent.prototype` `data` or `source` getter hands it
  the nonce when the runtime's own earlier listener reads them, and it can
  then post a draft first; it can also patch `HcAsk.prototype._answers`,
  `JSON` or array methods, or set the inputs' values before Send. The host
  doesn't check the answers' names against the canvas's fields (M, not
  done); hardening would have the runtime save the original getters and
  `stopImmediatePropagation` before any canvas code runs. What stays true:
  the person sees the sent answer, drawn by the host in the transcript
  bubble.
- **The lock lives in the transcript.** The first sent message carrying a
  valid block for `id@revision` locks that revision (`answeredCanvases`; Rust
  `canvas_answers.rs` reads it the same way, both pinned by
  `answerBlocks.golden.json`). The host posts `answered`, the fields go
  inert, and the host says "Answered" under the frame; later drafts are
  refused whatever the script does. It survives restarts; a new revision is
  a new question; the lock is per conversation.
- **Waiting on you.** An unanswered ask at its current revision says
  "Waiting on your answer" and is listed by `list_needs_you` as kind
  `canvas` (`needs_you.rs`), whose row reopens the conversation scrolled to
  that canvas (`store/canvasReveal.ts`, `src/canvasTurn.ts`). Archiving the
  conversation ends the wait.
- The answer block is hidden in the bubble and stripped from session
  records, the learning job's prompt and About-you's recent asks
  (`next_step::strip`, sharing `hiddenFences.golden.json` with the chat).

Details: [CANVAS_C4.md](history/CANVAS_C4.md).

## Workspace sections and the curated layer

Both are gone (COO rebuild L4, 2026-11-10). The workspace panel's native
sections (Overview, Brief, Plan, Results, Sources, History) and the curated
`section="..."` canvas views that stood in for them were removed with the
panel; so were Make canvas views, Ask agent for a canvas view…, Make a canvas
from this file and the selection bar's Make a canvas. The heading stays for
older links; the history is in [CANVAS_C5.md](history/CANVAS_C5.md) and
[CANVAS_C6.md](history/CANVAS_C6.md#lane-k).

What is true now:

- **Project files are files.** The top bar's **Project files** opens
  WORKSPACE.md in the Preview pane; its **Files** picker pins WORKSPACE.md,
  PLAN.md, SOURCES.md and AGENTS.md, then a build workspace's changed files,
  then the conversation's (`src/projectFiles.ts`). PLAN.md's checkboxes tick
  in the Markdown view (`NoteRenderer` `tasks`; `toggle_plan_row`, user-only,
  CAS on file and line, `plan_rows.rs`).
- **A Result opens with a review bar.** A file in a workspace's `outputs/`,
  saved canvases included, opens in the Preview pane with
  `ResultReviewBar.tsx` (state and checks, origin, Mark ready, Accept,
  Compare with accepted, Ask for a revision, More). Only the person accepts.
  Canvas cards are a structural summary (`src/canvas/structure.ts`), never
  agent HTML in the host.
- **Library → Results** joins output cards with saved canvas metadata in
  one ruled list with All/Files/Canvases filters. Per-row review badges
  remain; Review owns review filtering. Stable keys retain rows and the
  selected detail. Files open Preview; canvas detail retains versions, Compare
  and source actions. Thumbnails (`ResultThumb.tsx`: raster images up to
  2 MiB, video posters, host SVG sketches from a saved canvas's structure)
  read near the viewport, at most 24 eligible, reserving two for a selected
  detail. CanvasThumb never suspends controls. Store metadata reads are
  bounded to six and failures stay explicit; viewing writes nothing.
- **A `section` attribute** on a fence is an unknown attribute and is
  ignored; the canvas saves under its own id. Older `_<section>` files stay
  on disk, unlisted (`RETIRED_CURATED` in `workspace_canvases.rs`).

## Pinned live canvases (overnight 2026-10-12/13 F1)

A saved canvas’s pin menu (chat header, Results or Library → Results → Canvases) pins its exact
revision to Work → Today. Existing `home`
destination keys stay compatible; a workspace pin made before the Overview section went can only be unpinned; the pin menu and Show action name Today. Pins live in this Mac’s
`localStorage['hollow.canvasPins.v1']`, keyed by vault plus canvas store and destination,
with at most 12 pins. Nothing edits the saved canvas, Markdown or its acceptance.
Unpin removes only that preference. Missing/pruned revisions remain visible as an error
with Retry/Unpin. Open source chat resolves the saved revision’s conversation by key.
A newer revision is announced, never adopted automatically: pin it again to replace the pin.
Ask-canvases are read-only snapshots here; answer them in their source conversation.

An author may mark `hc-stat`, `hc-progress`, `hc-gauge` or `span` with
`data-hollow-bind="plan.done"` (the host updates the `value` attribute, or a span’s text).
The pin menu says **Pin with live data** when such declarations exist. Allowed scalars:

| Binding | Source and scope |
| --- | --- |
| `plan.done`, `plan.open`, `plan.total` | The source workspace’s current PLAN.md, parsed with the existing plan model |
| `tasks.open`, `tasks.done`, `tasks.total` | Local task list; optional root `data-hollow-project="vault:projects/…"` filters by exact project reference |
| `usage.runs`, `usage.reportedUsd` | This Mac’s current day usage; incomplete/unavailable costs are unknown, never estimated or filled with zero |
| `sessions.running` | This Mac’s running/claimed chat queue entries |

These are scalar counts, not arbitrary file paths, queries, task bodies or IPC access.
Unsupported declarations are announced. Unavailable data clears the bound value; a failed
canvas read keeps the last view visibly stale. The host reads on mount, vault changes,
manual Refresh and every 30 seconds **while visible**. Reads coalesce; hidden/offscreen
pins stop timers and release their iframes. Late results after disposal are ignored.
Updates use the one-way `live-data` bridge message without replacing the frame. The
runtime accepts only the exact parent, known keys and finite numbers/null; children
cannot request reads or mutations. Pins always use the local Canvas policy, ignoring
internet grants and offering no grant control. No model runs or costs are involved.

Modules: `src/canvas/liveModel.ts` (validated pins, scalar semantics, refresh lifecycle),
`src/canvas/liveBindings.ts` (inert declaration reader), `src/store/canvasPinPresence.ts`
(launch hint), `src/store/canvasPins.ts`
(local preferences), `CanvasPinButton.tsx`, `PinnedCanvasSlot.tsx` (lazy boundary),
`PinnedCanvases.tsx` (visible reader/rendering). Tests: `liveModel.test.ts`,
`tests/smoke/live-canvases.spec.ts`. Native VoiceOver and installed-app interaction
remain separate.

## The prompt and the setting

**"Visual answers in Hollow"** (`src-tauri/src/visual_prompt.rs`) opens every
chat's appended prompt: every chat type and provider. In order: the
decision rule (plain text / canvas / an `hc-flow` diagram canvas /
ask-canvas), one "how often"
line, "state the answer in prose too" and "never write a canvas into your
notes", the canvas guide (`canvas_prompt.md`: fence, id rule, the Canvas
sandbox as served, `hollowCanvas`), the kit@1
reference (`canvas_kit_prompt.md`), the Ask-canvases section (a "when" line
per question channel, then `canvas_ask_prompt.md`). The board guide
(`whiteboard_prompt.md`) is no longer sent: whiteboards were removed on
2026-10-07 and diagrams are `hc-flow` canvases. With Canvases Off the agent
is told to answer in Markdown only and gets no visual guide at all.
`visual_prompt::tests` pin every claim to `artifacts.rs`, both kit
manifests, the fence limits, the answer format and the store. Editing it, or a chat changing its choice, restarts native sessions
once (the resume fingerprint); regenerate the billed probe's prompts
afterwards (`HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib probe_prompts`).

**Where the formats are delivered** (agent effectiveness G1, 2026-10-07).
The canvas guide, kit reference and Ask-canvases section together are the
*visual guide* (`visual_prompt::visual_guide`, about 8 KB; empty at Off).

| Canvases | Appended prompt | The visual guide |
|---|---|---|
| Most replies | the whole block, formats included (about 9 KB), as before | in the prompt |
| When it helps (default) | the decision rule, how often, prose first, the ask "when" line, where the guide is (about 2 KB) | in a turn's message when it needs it |
| Off | the decision rule: Markdown only (about 0.7 KB) | none (delivery `off`; no file written) |

Every chat turn (except at Off, which has no guide) writes the guide to its scratch folder,
`$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`, which the prompt and the
capability index name, so an agent can read it whenever it decides to draw
(Claude and Antigravity get the folder with `--add-dir`; Codex and Grok
get its path in `HOLLOW_SCRATCH_DIR` and read it with their own tools, which
no live run has checked yet). A turn carries it as a
`<hollow-visual-guide>` block ahead of the caller's text when its request
looks visual (`visual_prompt::visual_cue`: Hollow's own cue words such as
chart, diagram, board, canvas, visual, compare, plan, timeline, status,
options; four or more numbers; a named `.csv`, `.xlsx`, `.json` or other
data file) or when the file couldn't be written, once per native session
(a resumed session that already has it isn't sent it again; a fresh one
is). The block is outside the fingerprint, so whether a turn looks visual
never restarts a session. The run manifest's `compiler.visualGuide` says
which happened (`prompt`, `turn`, `given`, `onDemand`, or `off` at Canvases Off) and why (the cue,
Hollow's word).

**Canvases setting:** Settings → Chat → Canvases, Off / When it helps
(default, never stored) / Most replies (`AgentSettings.canvases`; settings
key `chat.canvases`, Confirm tier). The model menu's Canvases row overrides it
per chat (`src/canvasFrequency.ts`, `hollow.canvases.<session key>`, sent
as `chat_send`'s `canvases`).

**Question channels** (`visual_prompt::Asking`): Claude and Codex with
question cards use cards for simple choices and ask-canvases for visual
answers; Grok and Antigravity (no owned MCP) ask with ask-canvases and keep
the fenced `hollow-questions` block for a yes or no.

## Receipts

Each turn's evidence receipt carries `canvases: {count, bytes}`: the reply's
`hollow-canvas` fences, valid or not, and their body bytes
(`canvas_fence::reply_canvases`, split exactly as `parseSegments` does, pinned
by `src/canvas/replyCanvases.golden.json`; `evidence.rs`). The reply footer
shows "2 canvases · 14 KB" and the run details a Canvases row. Absent when
none. Activity lists a run's canvases.

## The bed rows (retired)

The canvas rows `canvas-c0` … `canvas-c6` ran on the candidate test bed, which
Alex dropped on 2026-10-09; its driver and row scripts were removed on
2026-10-12. The last bed (`d155639a`, wave 3) passed 45 / 13 skipped / 0
failed. What each row proved is recorded in
[TEST_BED.md](history/TEST_BED.md#canvas-c3-rows) and the slice reports above.
A covered or full-screened window still pauses WebKit's animation frames, so
keep the window visible when checking canvases by hand.

## Where the code lives

| Piece | Files |
| --- | --- |
| Served documents and CSP | `src-tauri/src/artifacts.rs`, `src/components/artifactCsp.golden.json`, `src/components/htmlSandbox.ts` (`sandboxDocParts`, `THEME_TOKENS`, `childCsp`), `src/components/sandbox-doc.js` (host runtime) |
| Frames | `src/components/HtmlArtifact.tsx` (chat, Preview pane, Results, gallery, pins), `src/components/HtmlDoc.tsx` (briefs, preview, `.html` Results) |
| Host contract | `src/canvas/hostContract.ts` (the table and the head's words), `src/canvas/canvasText.ts` (Read as text), `src/canvas/galleryRequest.ts` (Compare in the gallery), `tests/smoke/canvas-host-contract.spec.ts` |
| Bridge | `src/canvasBridge.ts`, `src/linkGuard.ts`, `src/store/browser.ts` (`openFrameLink`, `openFrameFile`), `src/canvas/frameFile.ts`, `src/canvas/bridgeRefusals.ts`, `src-tauri/src/canvas_bridge_log.rs` |
| Kit | `src/canvas/kit/` (`ask/`, `generated/`), `src/canvas/kitAssets.ts`, `src/canvas/kitAskAssets.ts`, `scripts/build-canvas-kit.mjs`, `scripts/sync-sandbox-csp.mjs`, `scripts/canvas-kit.test.mjs` |
| Fence | `src/canvas/fence.ts`, `src/canvas/names.ts`, `src-tauri/src/canvas_fence.rs`, `src/canvas/fence.golden.json` |
| Chat | `src/chatSegments.ts`, `src/canvas/chatCanvas.ts`, `src/components/ChatCanvas.tsx`, `src/canvas/frameBudget.ts`, `src/canvas/frameBudgetScope.ts` |
| Flow audit and design check | `src/canvas/canvasFlows.ts` (shared inert extraction), `src/canvas/flowAudit.ts` (cached geometry findings), `src/canvas/designAudit.ts` (the design check's source pass, measured findings and draft), `src/canvas/pixelContrast.ts` and `src/canvas/measurePicture.ts` (Measure on the picture: the pure scorer and the capture/decode half), `src/canvas/canvasProblems.ts` (per-run counts for the receipt), `src/components/CanvasLayoutAudit.tsx` (lazy reply row and guarded attachment) |
| Mermaid import | `src/canvas/importFlow.ts` (pure, bounded), `src/components/MermaidDiagram.tsx` (lazy reply/file host) |
| Store and Results | `src-tauri/src/workspace_canvases.rs`, `src-tauri/src/commands/canvases_api.rs`, `src-tauri/src/workspace_results.rs`, `src/components/ResultCanvas.tsx`, `src/mockCanvases.ts` |
| Ask-canvases | `src/canvas/askAnswer.ts`, `src/components/ChatBody.tsx` (`fillFromCanvas`), `src-tauri/src/canvas_answers.rs`, `src-tauri/src/needs_you.rs`, `src/waitingOnYou.ts`, `src/store/canvasReveal.ts`, `src/canvasTurn.ts` |
| Project files and Results | `src/projectFiles.ts`, `src/components/ChatPreview.tsx` (Files picker), `src/components/ResultReviewBar.tsx`, `src/resultReviewBar.ts`, `src/components/LibraryResults.tsx`, `src/components/ResultThumb.tsx`, `src/resultsCards.ts`, `src/planModel.ts`, `src-tauri/src/plan_rows.rs` |
| Prompt and setting | `src-tauri/src/visual_prompt.rs`, `src-tauri/src/canvas_prompt.md`, `src-tauri/src/canvas_kit_prompt.md`, `src-tauri/src/canvas_ask_prompt.md`, `src/canvasFrequency.ts`, `src-tauri/src/agent_settings.rs` |
| Receipts | `src-tauri/src/evidence.rs`, `canvas_fence::reply_canvases` |
| Probe | `scripts/canvas-real-reply.mjs` (billed; free dry run by default) |

## Known limits

- **Not checked by a person in the installed app:** VoiceOver (a checklist
  is in [CANVAS_C6.md](history/CANVAS_C6.md#voiceover-checklist-for-the-person)),
  glass over the real material, answering a real ask-canvas, a real chart
  canvas in a live chat. Browser checks (axe, keyboard) are Chromium.
- **A canvas is saved only by the window showing its live turn.** A turn
  that finishes while its conversation is in the background renders later
  from the transcript, unsaved.
- **Kit budgets are nearly spent:** measure current head with `npm run kit:build`;
  the caps are 99 KiB for kit@1 and 25 KiB for
  kit-ask. A new component needs trimming, a bigger budget with a reason,
  or `kit@2`.
- **Receipts count canvas fences only** (not html fences), and
  neither the usage ledger nor Settings → Usage shows canvases. A turn
  `hollow_app` sends uses the setting, not the chat's own choice.
- **State** saves only while the frame has focus; a change in the last
  120 ms before an unmount is lost. Two windows on one canvas meet only on a
  conflict or a re-read.
- **The ask lock is per conversation** and needs the answer message in the
  transcript; reopening needs the canvas fence in it.
- **Bounds:** "re-rendering never writes" holds for 530 versions per id and
  8,192 pruned app-data bodies; `workspace_canvases::list` cuts at 500 in
  directory order.
- **Not measured:** token savings for charts and inputs (C0's 18.4% is text
  cards only); the share of real replies that draw a canvas; a
  canvas-specific memory limit (the 30-canvas row holds the journey's).

## Risks

The plan's five risks, then the ones the wave reports raised. **Closed:**
nothing left to do. **Mitigated:** what holds it down, and what remains.
**Open:** who decides.

| Risk | State | Why |
| --- | --- | --- |
| **Cost and latency** (plan) | mitigated | The kit keeps canvases short (C0: 18.4% fewer tokens on text cards; the two real replies of 2026-10-05 were 1.3–1.4 KB canvases). Canvases Off / When it helps / Most replies, globally and per chat; Off also drops the canvas parts of the prompt (about 6.6 KB instead of 14.5 KB). Receipts show canvas count and bytes per reply.  **Remains:** the prompt overhead is paid on every turn's prompt; chart/input token savings and the per-choice cost are unmeasured, and the usage ledger doesn't break canvases out. Measuring more is a billed probe: the owner's call. |
| **Prompt injection** (plan) | mitigated | Prose comes first (prompt rule). Canvases can't send on their own (answers go only as the reply to the person's click on Hollow's own Send, `collect` nonce, which only the ask kit hears since 2026-10-20; Bypass included), can't fetch (Canvas CSP has no network; zero attempts on every bed vector, default path included) and can't open silently (scripted links need a confirmation naming the address, three a minute). Project files show as files, never as agent HTML in the host. Refusals are logged. **Remains:** a canvas can still display misleading content, and an ask-canvas's summary lines are worded by its script (the person reads them before Enter; the agent is told to trust the JSON); its script can still change what the form answers (patch the kit, set the inputs), which the person sees in the sent bubble. Interactive is network by the person's choice. |
| **WKWebView quirks** (plan) | mitigated | srcdoc's inherited CSP is avoided by serving `artifact://` documents with their own header; the srcdoc fallback seals its own nonce. ResizeObserver is never used (charts redraw on events; the frame budget observes from the host). Custom elements upgrade in served frames (C0, bed). WebRTC is removed by the runtime because CSP alone let STUN out (C0). **Remains:** WebKit pauses animation frames in a covered window (bed rows fail falsely; keep it visible); DNS prefetch is unobservable (skipped); `hc-date` is the platform's control; nothing ran with VoiceOver. |
| **Drift between canvas and Markdown** (plan) | closed | Project files are read as files on every change (`vault-changed` re-reads); nothing renders the Markdown as a canvas. Chat canvases are snapshots by design, saved with their session, turn and revision. Agents are told never to write canvases into notes. |
| **Unverified with VoiceOver and the real glass material** (C2–C6) | open | axe and keyboard pass in Chromium; the person runs the [VoiceOver checklist](history/CANVAS_C6.md#voiceover-checklist-for-the-person) in the installed app and reports anything unreadable. Owner. |
| **CI never ran** (every wave) | open | Every CI run this programme was refused for billing; the local gate on each head was the merge standard. Restoring CI billing is the owner's decision. |
| **Background turns leave canvases unsaved** (C3) | open | Saving at the backend at turn end would close it; not built. Low impact (the canvas still renders from the transcript). Owner, as future work. |
| **Kit budgets nearly full** (C2, C4) | open | The next component forces a choice: trim, raise a budget with a written reason, or `kit@2`. Owner, when one is needed. |
| **Ask summary worded by the canvas** (C4) | mitigated | The agent is told to trust the hidden JSON; the host draws "Answered" itself, and the sent lines show in the transcript. Since 2026-10-14 the person no longer reads the lines before they go (Send sends). |
| **Canvas-specific memory budget** (C3) | mitigated | Six live frames per transcript; the 30-canvas bed row holds the journey's RSS limits (138–150 MB measured). A canvas-only limit waits for more measured runs. |
| **Large vaults** (C4, C5) | closed | C6 P3 made Activity incremental and paged and Waiting on you read only changed sidecars, measured on generated fixtures. |
| **Answer JSON reaches retrieval and connector choice** (C6 P1) | open | Local only, never leaves the machine; stripping it changes which notes a turn retrieves and wants its own test. Owner, if it matters. |
