# Hollow design system 3.0

## Factual utility surfaces

Top-bar icon buttons keep their 28px box on press and use selected fill.
Held selected toggles retain an inner border independent of keyboard focus.
TaskEditor's 15px subtask checks keep their box, with fill or halo feedback;
checked marks retain their success fill and ink. Their reveal changes opacity
without scaling. Both reduced-motion policies still govern transitions.

The Tasks list's empty/filtered panel centers within its collection. It uses
shared empty-state type and spacing, retaining the 440px maximum and compact
artwork padding. State wording, filters, art eligibility and actions stay
with the page's existing owners.

Today/Weekly facts, Usage totals/instruments and Build receipts use flat
section rules. History and Commitments retain opaque reading panes and
existing scroll owners, with ruled records inside. Commitment controls no
longer fade in. Exact/null values, error and recovery semantics stay unchanged.

## Automations and Map

Automations uses ruled list rows, covers and editor sections. Selected rows
have the rail's leading tick on selected fill, with an explicit Highlight
outline in forced colours independent of focus; schedule
and metadata remain readable on selection. Preview/recovery content appears
without entrance motion. Map has a flat frame and ruled toolbar; the canvas,
fixed text, hit targets, List alternative and mutation controls are unchanged.
Zoom, List, Legend and Refresh wrap as one group with shared spacing tokens.
Find yields space to the group. The legend starts closed unless saved open;
the visible Legend button and L preserve the remembered choice.

## Review and Running

Review uses two primary destinations and compact secondary waiting filters.
Groups have flat label bands and ruled rows; Incoming places its selected
detail across a single divider. Running's List is one continuous column,
with flat Tracks and a restrained Orbit marker. Selection retains a visible
forced-colour outline. No row entrance delays the reading surface.
When Tracks has no turns, its existing loading, failed or empty state owns the
explanation. Cost coverage and the evidence footnote appear with actual tracks,
including finished-only tracks; unknown and pending costs remain explicit.

## Library Results

The preview grid includes ordinary files and saved canvases. Document
covers show their format and size on a flat sunken plane inside the card's
single outer frame. Image, video and canvas previews keep their own rendering.
Search, All/Files/Canvases and workspace filters share one toolbar. A selected
canvas switches to a compact list and lazy detail column; at compact widths
the list stacks above it, with both scroll regions retaining usable height.
Files use the app Preview pane. Keep selected fill, inset focus ring, review
badges and the separate workspace link.

## Shared states and presses

Empty, filtered and loading StatePanels have no visible frame except under
Increase Contrast or forced colours; their words, icons and loading semantics
remain. Offline, partial, blocked, stale, failed
and conflicted panels keep their existing borders, tones and recovery actions.
StatePanel and EmptyState share title leading; their action groups wrap, and
long action labels grow vertically inside the available width.
Shared buttons and icon buttons use fill feedback without press scaling.
Hover, keyboard focus, disabled and selected states remain distinct. Both
motion preferences still govern transitions and loading animations.
Combined ghost/danger buttons retain on-error ink during keyboard presses.
Held selected icon toggles keep an inner border independent of the focus ring.

## Overlays and questions

Confirmation/form dialogs, command palette, context menus and notices use the opaque shared
raised surface even in Glass. Scrims retain their existing motion/accessibility
gate. The keymap card and slash popup retain Glass.
Palette rows use static selection rails and unboxed icons; menus enter
together without stagger or scale bounce. Native title tooltips stay native,
and editor autocomplete keeps the same raised-surface tokens.

Question cards and their composer dock use one flat reading plane with dividers
below the fields and rectangular controls. Ask-canvas status stays host-owned below a
fine rule; the ask kit fields share the kit's 3.0 styling. Existing focus traps,
answer drafts, keyboard navigation, notice timing and scroll owners remain.
Confirmation dialogs have a viewport-bounded outer scroll region as well as
their bounded body, so long titles and controls remain reachable at Largest.
The inner checklist cannot shrink to zero; the outer scroll retains access.
The visible conversation question wrapper is a shrinking flex column, allowing
its existing dock scroll owner to fit below the header/context on short windows.

## COO

The COO page has no header band (since 2026-11-11); its status is one written
line. The day column is an opaque reading plane with ruled sections, wrapping
controls and unchanged chat geometry. Automations owns the bounded
action-history disclosure; timestamps and full summaries stay crisp and
selectable. The COO menu's "Actions it took on its own" link reveals and focuses
that disclosure once after accepted navigation; manual toggles and refreshes
never replay focus, and reveal has no animation.

## Home

The populated Home keeps a 56px folded-threshold crop, written status links
with explicit coverage, and compact ruled resume rows. Wide rows place the
workspace identity beside its next step; compact rows stack and grow with text.
Result, review and menu controls wrap below. Removed tile gradients, shadows
and entrance animation. Review and Automations own detail rather than Home
duplicating their lists.

## Shell and navigation

An open workspace's conversation fills the centre; its notes (WORKSPACE.md,
PLAN.md, SOURCES.md, AGENTS.md) read as files in the Preview pane, and a
Result there carries its review bar. The Workspace view's section band and
section styles went with the workspace panel (COO rebuild L4, 2026-11-10).

The sidebar uses the existing 240/60px rail tokens. Workspace headers, nested
conversation guides and static selection edges give the list its hierarchy;
status words, provider identity and pin order remain live. Selection belongs
to each row, including a Highlight outline in forced colours. There is no
selection measurement or observer. The 44px context bar retains native drag
regions, traffic-light clearance, history and both page-action mount points.
At compact widths the open Glass rail uses the opaque shared soft surface:
it overlaps app content, so desktop vibrancy cannot protect its text.

The shell's one right-hand slot is `.canvas-split` inside main.canvas: the
page and, only while something is previewed, the PreviewPane beside it under
the top bar, so the bar never moves when a preview opens (run 15 P1, which
replaced A2's empty `shell-preview-slot` beside the whole column). P1 owns
PreviewPane content, target/history, sizing and the compact replacement
policy. ActivityDock Manage uses the guarded existing
Work / Running destination, retaining the operations it displays.

The source of visual values is `src/styles/tokens.css`. Components extend
the existing primitives; they never create a parallel palette or component
for the same job. The direction is a quiet workbench: neutral white or charcoal,
Crimson accent for action, flat reading planes and restrained foreground depth. Dense
lists and fine dividers replace floating-card ornament. Existing capabilities,
draft ownership and keyboard routes survive the layout redesign.

The radius scale is 3/5/6/8px, the default page gutter 24px (16px compact),
section gap 24px, controls 32px and two-line row padding 6px. Text retains its
scaled reading ladder. Resting elevation and specular edge are transparent;
menus/dialogs retain foreground shadows. Custom palette derivation and frame
exports retain existing token names. Theme choice is never overwritten.

## Identity and texture

Home and empty chat lazy-load the original folded threshold through
DoorwayLandscape. `ThresholdArt` shares `thresholdField.ts`'s folded/orbital
scalar fields, with at most four cached 80x36 glyph grids. Each SVG has 36 text
runs, not one node per glyph, and no timers, canvas context, image download or
animation loop. Artwork is decorative, bounded to 600x300, cannot receive
pointer input and stays outside reading content. `--fs-art` is an unscaled
decorative cell size, never a text role. Add ornamental surface treatments
only with a real consumer; the foundation has no unused texture classes.

Identity may use glyphs; evidence may use static pattern fills; work stays
plain. Exact values, input contents, code glyphs, transcripts and user files
are never dithered. Canvas table bands and field gutters use the shared material.
`data-texture-active` starts absent and is set by the lazy policy
owner only after a11y checks; fallback is a plain doorway. App/system reduced
motion, reduced transparency, increased contrast and forced colors suppress
decoration directly in CSS too. The static field does no hidden/idle work.
Settings > General offers Full / Quiet / Off texture and a live shared-art
preview. `hollow.studio.texture` stores the level; missing or invalid values
default Full. Quiet thins identity glyphs and the shared canvas tile; Off keeps
plain drawings and disables canvas texture. Functional geometry stays exact.
The root `data-texture-level` is presentation-only, with accessibility above it.
Selected segmented options use a 2px underline in forced colours, where
background/shadow markers disappear. The shared keyboard-focus outline
remains separate; forced colours still owns every colour.

Every canvas defaults to static dither across its field, kit surfaces and
graphics. Legacy root plain uses that default; local graphic plain remains an
opt-out and ASCII remains available. Solid text/control plates and meaningful
chart boundaries stay above the material. One frame tile supplies CSS bands,
SVG patterns and chart fills without scroll work or new motion. The frame
follows host policy plus its own accessibility veto. See [Canvases](CANVAS.md).
ASCII heatmaps retain their glyph vocabulary and original hit cells; missing
data is never painted as zero. Agent guidance still calls for small original
content-specific artwork in addition to the shared material.

## Typography and numbers

Tasks uses flat ruled board/week lanes, tabular dates and compact list rows.
Group headers carry the quiet surface; selected rows and drop targets retain
their own state fills. Detail properties and workspace links form plain
sections. The shared editor uses growing native controls and no entrance
animation. No texture appears in task text, checkboxes or drag targets.

Home's greeting/date uses `--sp-1` bottom spacing before the compact art,
keeping Largest text within the existing tabs-to-glance bound without
shrinking type or controls.

Isolated Review, Incoming and Runbooks failures use the existing compact
RecoverableView/StatePanel treatment, with Retry and Copy details. Destination
tabs stay outside the error owner; no new material or motion is introduced.

Main's day uses existing hairline/body/muted roles to distinguish successful
secondary sections from Today, Review and read failures. Empty-success rows
remove card padding; warnings keep weight. Action times align in an em-sized
gutter with tabular figures. No motion, color or material token is added.

Plan Summary leads with its existing progress stat, then native Plan notes,
then lanes. Notes reuse sec-plan-fold and preamble typography; no new colors,
materials or motion. Notices remain unfurled above the summary.

Filtered-empty Results use the existing StatePanel action slot and small
buttons. Clear search and Reset filters act independently; focus returns to
the corresponding toolbar control. No new material, motion or read is added.

Canvas comparisons lead with Before/After captions and rendered previews.
Source changes use a native disclosure with ink text, primary keyboard ring
and existing spacing/radius tokens; no motion or new material is introduced.

Keep Inter Variable for UI/prose and JetBrains Mono for code. The existing
scaled ladder is 11 caption, 12 metadata, 13 UI, 15 reading, 17 panel,
20 heading/page title and 28 display; `--text-scale` applies to all.
`--type-optical` enables font optical sizing. Shared page titles use
`--title-optical-size` 20 times text scale and `--lh-title` 1.25; other text keeps automatic
optical sizing. Weights remain 400/500/600. `--type-numeric` means tabular
lining figures for metadata, counters and pills, not all prose.

Page headers share 12px vertical padding and the responsive page gutter.
Tasks and Library inherit this rhythm rather than overriding it. Main's
title stays 20px with a 25px line box from the shared leading; its compact
header grid, padding and controls stay in place.

## Materials and depth

Companion status and scope selection use solid material, the shared edge and
medium/large radii. Workspace names wrap at body size; metadata stays on the
existing meta scale. Count values use panel-size tabular figures in a responsive
definition grid. Controls and scroll owners remain unchanged, with no motion.

Pinned canvas hosts use solid reading cards, inset edges and a header divider;
freshness timestamps use the numeric role. The host copy dialog groups editable
items and the Plan preview on solid planes with existing medium radii. Close
and Confirm stay outside the body's scroll owner; source/copy behavior and
frame palette contracts are unchanged. No blur or motion is added.

| Role | Token | Use |
| --- | --- | --- |
| Solid | `--material-solid` | Buttons, fields, selected segments, reading/state cards |
| Glass | `--material-glass` | Existing Glass cards use the reading-well opacity floor |
| Strong Glass | `--material-glass-strong` | Menus, popovers and dialogs; raised solid fallback |
| Boundary | `--material-edge` | Existing strong Glass border or solid hairline |
| Specular edge | `--elev-edge` | Decorative inset top highlight; never the only control boundary |

Materials are live aliases of `--surface-card`, `--surface-raised`,
`--glass-well`, `--glass-well-strong` and `--glass-border-strong`. No custom
theme field or frame token is added. Custom themes continue deriving the
source colors; Canvas kit uses its existing bridged source palette. Do not
pass unresolved host aliases into a frame. No new backdrop blur is introduced.

Keep the elevation tiers: transparent `--elev-1` for resting/selected controls,
`--elev-2` menus/popovers, `--elev-3` dialogs/palette. `--elev-edge` and
`--elev-action` retain their API but no longer add highlights or glows.
The radius family is 3/5/6/8px and pill; existing aliases stay valid.

## Accent and contrast

Sam's Acme-red primary ramp is light `#b5251c` / active `#dd3227`, dark
`#eb847d` / active `#f0a39e`. The dark tint supports contrast; soft and selected
fills use signature-red RGB. Risk keeps violet, distinct from amber warning.
Warning/error state pills use triangle/cross marks beside their words; Canvas
Kit tones retain icons and words. Do not use accent as a substitute for status.
Tinted link/pill text uses `--primary`; `--primary-active` is a button fill,
whose lighter light-theme red is not AA text over the tinted background.
The contrast suite checks the rendered wikilink hover's composed fill.
Sandbox and kit fallbacks match the stock light palette.

`--on-primary` stays white in light and charcoal in dark. The build checks
146 normal-text pairs in both solid themes, including primary and active
button labels. A separate regression composites both
Glass wells over the opposite extreme backdrop and checks every normal text
token at 4.5:1. The regular-well opacity floor is 0.97 in both themes;
strong is 0.98 in both. Native backdrop rendering still needs an
installed hand check. Custom themes retain their existing AA clamp.
`--focus-outline`, `--focus-offset` and `--ring` preserve visible keyboard
focus. Native title tooltips keep their OS rendering and accessible labels.

## Motion and accessibility

Use the existing `--ease`, `--ease-out`, `--ease-spring` and `--t-*` family.
`--t-stagger` is 20 ms; existing menu cascades stop at `--t-fast` (120 ms).
Do not add layout animation or entrance opacity to make a surface feel new.
P3-07's app/system reduced-motion policy remains the authority; this row adds
no JavaScript motion. Experimental effects belong to the shared Interface
switch, not this stable foundation.

The last `a11y` layer wins. Reduce transparency and Increase contrast replace
the Glass source wells with opaque surfaces. Strong material explicitly becomes
`--surface-raised` there, so floating menus retain their foreground distinction.
Increase contrast overrides boundaries and focus; both Reduce motion gates
remove transition/animation timing. Hidden-window motion keeps its existing
pause policy. Native Glass, VoiceOver and installed font rendering remain
post-install hand checks.

Custom themes set `data-theme` from their `isDark` polarity before applying
derived colors, so their specular edge chooses the matching theme strength.

## Experimental visuals

Settings > General > Ambient motion (formerly Experimental visuals) is on by default and stored as
`hollow.studio.experimentalVisuals` (`on`/`off`). `experimentalActive` in
`store/studio.ts` starts false; the lazy controller alone updates it and
`html[data-experimental-active="true"]` after checking app/system Reduced
motion, windowAway, document focus/visibility, increased contrast, forced
colors and reduced transparency. New motion scopes to that attribute;
JavaScript reads the signal. Static identity uses the same owner's separate
texture decision, independent of saved ambient-motion consent.

Main's shared indicator aura retains one four-second waiting breath; idle has
no loop. The pointer spotlight, Home/Map parallax and added rail aurora are
removed, including their global pointer/scroll handlers and ResizeObserver.
The older background wash and existing live-status effects retain their owners.
No identity motion is added: static SVG is the zero-idle-work first release.

## Surface owners

Sources places size, coverage and freshness words in one wrapping metadata
group below the full filename. Body-colored caption text carries every state;
the decorative glyph and warning edge are redundant. No extra focus stop,
read or motion is introduced; native disclosure and action owners stay intact.

Library Results first-load rows reuse the actual single-column grid, document-cover
height and title/workspace slots. Their four inert silhouettes have one loading
announcement, no focus stops or preview readers, and no shimmer under any motion
setting. The shared unsuspended resource mirror keeps the toolbar mounted;
loaded rows remain during refresh and failures keep their existing local panel.
Mapped cards retain their existing result-key identity across refreshes.
Task row DOM stays keyed by URL through acknowledgement payloads so its
one-shot check animation survives without replay or delayed writes.
Commitments count pills use `--sp-1` padding to keep New beside the search.

Transcript search marks use primary-soft with ink and a primary underline;
speaker and text retain their existing columns. The wrapping match count and
Clear search row uses muted text and tabular numbers. Source strings, loaded
segment filtering and recording ownership remain unchanged; no new motion.

Utility surfaces use solid reading planes and the decorative inset edge:
Build's existing app section, Usage totals/charts, History's list and detail,
Runbooks and Commitments. Selected rows retain explicit accent/ink and
semantic status words; History metadata wraps. Runbook actions wrap within the
same owner. Card hover/selection tints composite over solid material. Commitment group headings
use sentence case. No new motion, data reads, routes or write behavior. People and Goals have no
standalone page; their existing vault-note routes remain unchanged.

Empty-view sketches use angular plates and static stippled scene strokes under
the shared texture policy, with solid accents and plain accessibility fallbacks.
Lazy/evidence owners and 160x112/80x56 dimensions stay unchanged; per-drawing
animation observers are removed. The 2026-10-27 release cover reuses ThresholdArt
inside the lazy reader; the historical cover remains. Covers use ruled planes
and crisp optical type, with compact geometry following the window container.
Decorative SVGs add no focus stop or motion. See [Empty views](EMPTY_STATES.md).

Canvas kit@1 translates the 6px plane radius, flat ruled edges and
400/500/600 hierarchy through its existing `--hc-*` roles. It reads only
bridged `--app-*` colors with fallbacks; no host-only material alias is sent
into frames. Stats use optical/tabular values, callouts use solid reading
planes with tone edges/words, and chart/comparison heads have quiet dividers.
Tables use compact ruled rows; comparison recommendations have an inset
accent rail and a written badge. Square steps retain outlined check marks;
the current step is filled. Ask options and ranks form continuous rows with
native controls; their selection, focus, error and answer owners are unchanged.
ASCII flows retain their established graph layout and text while changing
only routes and box borders. Progress texture stays within its true fraction.
All exact-data tables, keyboard routes and motion gates persist.

Overlays share strong foreground material and existing elevation tiers.
Snackbars deliberately match the calmer notice family; their primary-edged
action pills keep Undo distinct, and failures keep the semantic error rail.
Snackbars now use ink on that surface, sentence-case primary actions, an
opaque Details label and the existing countdown ring; holds and receipt
ownership do not change. Question cards/docks use solid choice wells, selected
ink and tabular counts; stale text stays fully opaque with its status words.
Menu focus has an inset ring. Native title tooltips remain platform-owned.
Dialog/modal/notice entrances translate six pixels using `--t-med`/`--ease-out` and
motion gates, with content opaque from the first frame and no scale overshoot.
Backdrop fades, exit holds and countdown timing keep their original owners.

Automation covers put title/routine on one line, wrap actions separately and
group schedule facts in a definition row. The paid-run note follows actions
directly; compact sp-1 gaps/sp-2 block padding retain text and control sizes.
Next run uses semibold weight and keeps warning color above the base value rule.
A static semantic edge replaces the
horizontal pulsing seam; status words stay authoritative. Next runs form a
labelled timeline with unchanged times and minute cadence. Run durations use
only loaded records, oldest to newest, capped to the reading list's 25 entries;
zero is real and missing values break the path. The static SVG is decorative
and the dated/status-labelled list provides exact values. Map label plates,
legend and tooltips are solid, with shared material edges/elevation. Node and
label placement, pan/zoom and hit geometry remain owned by the existing map.

Command palette uses strong foreground material, inset grouped rows and solid
icon wells. Selection keeps fixed padding; its detail text uses selected ink.
The optional plain-text preview is a separate solid reading region, lazy and
read-only under `experimentalActive`, at least 1100px wide / 620px high.
It shows up to 1800 characters from a note/brief or loaded task description;
queries, selection, close and policy changes cancel delayed delivery. No action
runs on highlight. Reading focus keeps arrows/Enter; Escape still closes.

Library Notes uses a compact description/utility band; count and Pin/Save wrap
together, and saved-view controls stay grouped. Notes and Results form continuous
ruled lists, with full workspace metadata below result titles. Result previews
fit a 48px strip; document format and size remain in the small cover. Only listed
titles and summaries appear. The open-reader sidebar keeps its compact composition.
Recording rows use a selected accent rail and two-line titles, retaining date
and duration. Transcripts keep a fixed nine-character time gutter, tabular
numerals and speaker above words, with hairlines between segments. Proposed
tasks use an inline section. No new motion, reads or waveforms; missing
speaker/summary/transcript states remain explicit.

Running child chats and all-work Review retain separate labels and coverage.
The shared orb uses existing material/edge roles and keeps its fixed footprint.
Working providers use existing theme hues: Claude syn-number, Codex syn-function,
Antigravity syn-type, Grok ink; unknown/mixed is primary. These are app hues,
not brand colors, and recorded-provider words accompany them. Waiting/failure
keep semantic color and shape. Idle is still; one waiting aura replaces both
the old shell pulse and A5's top-bar-only glow, under the same combined policy.

Chat uses a continuous transcript with left-aligned labelled prompt bands,
ruled tool/work disclosures and a flat compact composer. Transcript and input
fill the pane inside the same side gutter (no reading-width cap); file chips
and framed canvases retain their existing actions. The composer is one
rounded box (`--r-lg`): the message on top, then a bar with one control per
job: + (attach and commands), the access chip (the permission mode, in
`--warning` for Full access) and Context with Connectors at left; usage, the
model menu (Permissions too), Dictate and a round Send at right (overnight
11-12, C3). As the bar narrows, effort, a safe mode's word and the Steer now
label drop, never overlapping. Folded-threshold artwork leads the empty state only.
Disclosure identity, reading anchors and control order stay unchanged.
The existing typing caret carries a small opacity-only
glint under `data-experimental-active`; off/away leaves a static caret. No
animated text, new scroll owner, blur or streaming re-render is introduced.

Review's kind groups and Activity's day groups use solid material, the shared
edge and inset highlight. Failures lead in data order with a semantic edge
and the word Failures. Review loading uses the shared Skeleton without motion;
its empty state retains the existing evidence rules. Running uses strong
foreground material. Timelines keep actual chronological events and loaded-day
counts; no decorative data, new animation or duplicate bell is added.

Settings uses a quiet section index with wrapping labels and static selected
edges: twelve sections in one list, no group caps (S2). Introductions and
agent defaults are ruled sections without nested frames; connector
configuration, profile notes and Skills follow that same grouping. A part (a
former section kept whole inside one) opens with a ruled `h3` a step larger
than a sub-header (`--fs-xl`, `--sp-6` above it), never a card; Skills, the
last part of Agents, is a bounded pane about a window tall whose list and
detail scroll inside. Scope facts remain labelled and wrapping. Selected/focused index
and Skills rows keep separate forced-colour outlines. Theme/cap samples retain
their solid reading material. Theme samples use listed colors through the existing
custom text clamp, with readable title/body/selection/action treatments and
no focus stops; Use alone selects. Samples do not simulate native vibrancy.
Built-in caps reuse the shared meter track/fill and retain all spend/state
words. A percentage exists only for complete spend and a positive matching
period limit; zero caps and incomplete costs have no numeric meter. No new
motion is added; existing shared meter motion obeys the a11y layer.

Shell chrome follows the compact geometry and static selection in
"Shell and navigation" above. Existing page morphs use
`--ease-out` within `--t-slow`; the entering snapshot is opaque from its
first frame and the old snapshot is hidden to prevent Glass ghosting. Task rows may target their existing detail pane through
`morphFrom(source, target)`, leaving the page still. Immediate task choices
call `flushViewTransition` before changing selection, so a waiting visual
update cannot overwrite them. Inline transition names restore on completion
or refusal; both motion gates use plain cuts.

`ui.css`: buttons, segments, status pills, state cards and ActionMenu.
`foundations.css`: generic inputs and keyboard focus.
`select.css`: Select/MultiSelect.
`effects-glass.css` and `glass.css`: existing floating-surface and Glass tiers.
`library-today.css`: ContextMenu and its bounded cascade.
Surface rows consume these tokens and add only their local composition.
Record additions here and in the [build log](BUILD_LOG.md).

Tasks Board and Not this week (under Agenda's Week) keep solid card surfaces and the existing radius
family. Priority and label chips use sunken surfaces with body text; checked
controls use the primary/on-primary pair. Compact titles allow three lines,
while List retains its dense single-line composition. A newly completed check
settles with `--t-slow` / `--ease-spring` behind the shared experimental policy.
Only its SVG transforms; the primary/on-primary state, hit target and text stay
fixed. Render-local arrival markers are consumed once and expire in a microtask;
initial completed rows, later reveals and policy re-enabling never replay them.
Turning the policy off cancels the effect. Filtered-out rows leave immediately.
