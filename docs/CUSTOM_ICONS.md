# Custom icons

Settings → General → App and COO icons contains both rows, each
with Upload an icon, Generate a logo and Reset. The app row also has Fold (default),
Threshold, Cut Glass, Aurora and Monolith preset buttons. On the COO page, the top
bar's Actions for COO menu → Change icon… opens the same lazy dialog (a header link
until 2026-11-06). No new shortcuts. Settings search opens the disclosure and focuses the matching row.

**The icon dialog (overhaul H1 and H2, 2026-11-12)** replaced the icon maker and editor
(shapes, colours, letters, symbols, offsets, Fit/Crop and Edit are gone). `CustomIconEditor.tsx`
has two modes, Upload an icon and Generate a logo, one preview, Apply icon and the target's
Reset button. Upload takes PNG, JPEG or SVG (`customIcons.ts`: by name, at most 8 MiB, never
reading an oversized file, then by bytes: a PNG or JPEG signature or an SVG document; JPEG since
11-13, decoded by the backend with the same limits as PNG), then
`prepare_custom_icon` runs the backend check below and the window draws the picture fitted
whole into the 512 px render that is saved (SVG is rasterized only after that check), at the
width and height `prepare` returns, so an SVG with only a viewBox keeps its shape.
Every save records `editor: {mode: "upload", fit: "fit"}`. Older records keep their saved
recipe and pictures; nothing migrates them, and the dialog never reads a recipe back.

Generate a logo (`logoGenerate.ts`) picks an installed agent: Codex when it has room, else
Claude, else any other installed agent with room, else the first installed in that order.
Room is Settings → Usage's own reading (`cfo_view` accounts); no readings count as room and a
failed read counts as none. `generate_logo_options(agent, target, request)` (`logo_options.rs`,
user-only) checks the agent is installed, makes one tool-free one-shot in the system temp folder
(metered `logo_options`, 180 s) asking for four dithered 512 px SVGs in the element subset the
SVG check allows, and returns only the ones `custom_icons::prepare` accepts, at most four,
duplicates dropped. The person's request (at most 400 characters) is fenced as a description.
The options show in a picker card (a radio group); picking one prepares it like an upload.
Browser mock mode and tests use a stub agent (`mockLogoOptions` in `mockCustomIcons.ts`;
`mockLogoOptionsError` and `mockLogoOptionsCount` in localStorage); no test makes a model call.
Closing the dialog while an agent draws stops the call: each request carries a `requestId`, and
`cancel_logo_options` (user-only) fires its stop signal through inline AI's stop table (ids prefixed
`logo:`), so the runner ends the agent's processes; a cancel that arrives first means it never
starts. Upload, pick and generate each take a ticket (`createIconTickets`), so an older result
never lands after a newer action.

Preview sizes are 16, 32, 64 and 128 beside the 192 px hero; COO previews include
busy/waiting/paused/three-chat states. Apply is explicit; Cancel keeps the saved icon.
Cancel, Escape and a backdrop-origin click confirm ("Discard this icon?") once an image is
chosen; dragging out from the card never dismisses it. Saved-record read failures disable
Apply. Damaged records name the target's Reset button, which the dialog also has;
other read errors offer Try again. ICON_DAMAGED keeps its own plain-language category and
raw Details. Conflicts reread the current revision; the dialog keeps the chosen image and
asks before replacing the refreshed revision; CAS checks again at the write boundary.
Picking a preset or Reset over Custom on a row asks "Replace your custom icon? Its original
is deleted."; Reset in the dialog asks "Reset to the stock icon?". Approval carries the
revision seen before asking, so a later icon change refuses instead of inheriting it.
Preset-to-preset changes remain immediate.

`store/customIcons.ts` shares one thumbnail per target and one app-lifetime change listener.
The app thumbnail updates HollowMark in the rail's existing 18 px slot and the
document favicon. Separate read guards prevent a late read from undoing Reset or
changing the other identity. The header's text, layout and accessible name are unchanged.
`MainAgentIndicator` uses it in the orb, page header and Main agent chat
avatars. The image stays inside the state ring; opaque stock badges stay outside
it. Other conversations retain their provider identity. Accessible names keep
Main agent. Stock artwork remains the fallback under read failure.

`custom_icons.rs` validates local uploads and stores independent `app` and
`mainAgent` records under the instance's app-data `custom-icons/` directory.
The vault, application bundle and installer are never changed. `app` is applied
with NSApplication.setApplicationIconImage on the main thread and restored on
launch. It affects the Dock while Hollow runs, not Finder or Applications.
The same runtime change updates the menu-bar icon (64 px source, full colour;
Reset restores the stock macOS template) and existing native window icons where
the platform displays them. Startup restoration runs after tray creation and holds
the write lock until its UI callback finishes. A partial runtime failure restores
both the previous storage and runtime artwork. Native About has no explicit icon
override and uses NSApplication's current icon; native verification remains manual.
What's new and Welcome tour have no app-logo image; their diagrams stay unchanged.
In-app notifications use severity dots, not an app identity mark. macOS notification
branding and Login Items belong to the signed bundle and stay stock. The decorative
Home/new-chat panorama, generic session doorways and
independent Main agent artwork are deliberately not replaced by the app choice.
The installer keeps its strict deep codesign verification; no safe persistent
Finder customization has been proven, so none is attempted.

The five user-only commands in `commands/custom_icons_api.rs` read, prepare,
save, reset and select an app preset. A change carries the last revision (null for default);
Read defaults to the 128 px thumbnail, clearing original bytes and editor data
(`includeOriginal: true` returns the whole record; the dialog no longer asks for it). These summary reads are not
full records to save back. The stored record is never truncated.
Prepared raster previews are bounded to 1024 px; originals stay untouched. This
avoids sending a decompressed multi-megapixel PNG through the preview channel.
the process-wide write mutex checks it before the atomic write. A failed Dock
application restores the preceding saved record. Bad existing records fail
closed. An explicit Reset with no loaded revision preserves damaged bytes in a
sibling `.damaged-<random>.json` file before recovery. Reset writes null; it clears AppKit's override with `setApplicationIconImage(None)`, restoring the bundle identity without deleting
unrelated files. Changes emit `custom-icon-changed` with the target.

Records are private (0600), bounded to 16 MiB, version 1 JSON: `revision`,
`original: {format, bytes}` (base64), `editor` (up to 8 KiB JSON), `pngs`
(base64 PNG indexed by 16, 32, 64, 128 and 512), plus optional `preset` metadata.
Legacy records have no preset and display as Custom; there is no migration.
Uploads are at most 8 MiB,
4096 px per side and 16,000,000 pixels total (4096 square is too large);
decode allocation is bounded. Rejected SVG element errors name the element.
Save requires
a decoded 512-square PNG and generates the smaller sizes with Lanczos3.
PNG/JPEG signatures are decoded using their declared format. ICNS accepts
bounded PNG representations (up to 1024 square, 64 chunks); legacy ICNS should
be exported to PNG. SVG accepts static shapes, text and local gradients only:
scripts, CSS, links, external references, entities and animation are rejected
before the dialog receives a renderable data URL. Originals are retained on this
Mac, but never rendered without preparation.

The native regression tests cover malformed formats, active SVG, dimensions,
ICNS chunks, multi-size output, persistence, private permissions and reset.
Dock display and restart persistence require a native hand check after an
authorized install; compiling or browser mocks do not verify AppKit behavior.

## Glyphs and project artwork

`IconName` derives from the literal factory keys in `icons.tsx`; props and
metadata maps carry that union rather than accepting arbitrary strings.
Factories still create fresh DOM nodes for every instance. `iconMeaning.ts`
separates Main agent (bot), What's new (gift) and Memory (book), and reserves
sparkles for creative drafts and skills. Sparkles uses a star, distinct from
Today's sun; Compare uses opposing arrows, distinct from pull requests.

Project artwork still comes from the fixed priority list of contained paths.
Its cache compares individual canonical candidate paths, lengths and mtimes
plus the config stamp (including a linked config, matching the existing owner
reader). A config dated in the future cannot mask replacement
of nested artwork, and a newly added higher-priority candidate invalidates it.

## Stock artwork and app presets (2026-10-28)

Fold is the dark static stock icon in the signed bundle, the existing
sidebar mark slot, and favicon. Finder/Applications/notifications use that stock
identity. No Assets.car or appearance switching is shipped: the native Dock
appearance route has not been proven on this host, and must be verified with
strict signing and installer checks before adoption. Do not infer adaptive
support from layered SVGs or Tauri's ability to compile a .icon input.

The App icon row's labeled native buttons expose aria-pressed selection and a
live status. Picking Fold or Reset to Fold writes null and releases
the runtime Dock override. The other four apply immediately through the existing
serialized compare-and-swap write, rollback and event path. They persist in the
same app-data record. Existing custom icons stay active until explicitly changed;
opening Settings or installing a newer bundle never selects a preset.
Main agent has no preset picker. Its default is the matching Fold artwork in
`src/assets/brand/main-agent-fold-512.png`; Reset restores it. Saved uploads
remain independent and are never replaced on startup. The same 20/32 image frame
and circular crop preserve the state ring, motion gates and opaque badges.
Counts still belong to the waiting state; selecting Fold adds no running-count
behavior. The 96 px rail image already exceeds the 18 px slot's Retina needs.
Its committed 96 px source is hash-pinned alongside the original small drawings.
An explicit empty editor preview still shows the plain doorway; an absent saved
Main agent override shows Fold. Old default Cut Glass choices were stored as
null and therefore follow the new stock default; selecting Cut Glass now saves
an independent preset record.

`app_icon_presets.rs` embeds the checked-in representations from
`public/app-icons/<id>/`; it does not resample 16/32 px images. Runtime Dock
artwork is an in-memory multi-representation ICNS assembled from the saved PNGs,
so AppKit can select the authored small sizes. An upload over a preset saves
Custom, leaving the immutable preset intact.

To regenerate stock assets, run `python3 scripts/build-stock-icon.py`
with Pillow available as an asset-authoring tool. It copies exact 16/32 PNGs,
builds PNG-backed ICNS/ICO representations, and updates bundle PNGs, sidebar and favicon.
The ICNS writer preserves PNG bytes instead of iconutil's ARGB conversion at tiny sizes.
`public/hollow.svg` is the same Fold source. The separate
`src-tauri/icons/tray-template.svg` retains the existing monochrome doorway:
layered dither would lose clarity at menu-bar size.
`python3 src-tauri/icons/gen_tray_template.py` renders that SVG with sips.
These authoring tools add no runtime or npm dependency. The stock tray remains
a template; custom/preset tray artwork uses the existing full-colour runtime path.

Fold's PNGs and rect-only SVG in `public/app-icons/fold/` are original artwork
selected by Alex on 2026-10-28. They derive from the three-direction local
identity study: four offset arch fields on an 8 px ordered-dither lattice,
separate 16/32 px pixel drawings, warm charcoal/paper and lilac. The Main agent
has a circular dark backing for light-ground contrast; it is not a reused app
tile. No third-party art, renderer or new runtime dependency is included.
