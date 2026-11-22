# Styling

## Motion kit

[Motion kit](../MOTION_KIT.md) owns measured height/width wrappers and visible
operation cues in Settings, Library and rail search. It consumes the shared
panel-duration token and base-layer timing tokens. Native details retain their
keyboard and focus behavior. Zero duration or either reduced-motion preference
snaps geometry and keeps operation labels/static loading shapes visible.
Repeat cues stop while hidden or offscreen; forced colours remove overlays.

## Shared design system

Chat approval buttons and conversation-row actions keep fixed geometry on
press and reveal, including with either reduced-motion preference. Allow
adds an inner on-primary edge; Deny and conversation actions use held fills.
Disabled controls receive no hover or press feedback. Visibility, focus,
approval state and archive/delete handlers remain component-owned.

Top-bar icon buttons and TaskEditor's subtask checks use fill or halo press
feedback without transforms. A held selected top-bar toggle keeps its inner
border and fixed border-box size; focus remains separate. Subtask marks fade
without scaling. See [Design system](../DESIGN_SYSTEM.md).

Empty Tasks lists center inside their collection and inherit the shared
empty-state heading/body styles. Their 440px maximum and evidence-gated
artwork remain, with compact art padding and the existing actions.

Automations and Map use continuous ruled surfaces. Automation selection has
a trailing edge, independent of the shared focus ring. The Map frame stays
flat while its interactive canvas and List retain their own scroll owners.

Version 3.0 uses neutral paper/graphite, flat resting planes and compact
3/5/6/8px radii. Sam's palette port replaces paper/lilac with white/charcoal
and Crimson accent; geometry and accessibility policy remain shared.
DoorwayLandscape lazy-loads shared static glyph artwork;
its plain fallback and the CSS a11y layer keep all reading content intact.
The pointer spotlight, parallax and extra rail aurora are removed. Details:
[Design system](../DESIGN_SYSTEM.md#identity-and-texture).

Chat's existing transcript, fold, chip and composer rules consume the shared
materials. The typing caret alone glints through the experimental root policy;
text remains opaque and stationary. See [Design system](../DESIGN_SYSTEM.md).

[Design system](../DESIGN_SYSTEM.md) names the type, material, depth, radius,
accent and motion roles used by shared primitives. Material aliases remain
derived from the existing palette and Glass wells; they do not add custom
theme fields or frame colors. Keep the existing a11y layer authoritative.

## Shared control presses

Shared filter chips, toast actions and modal close buttons keep their size on
press and use fill feedback. Selected chips keep a stronger border while held;
chip selection has no bounce. Toast actions also strengthen their border
while held so pointer presses differ from hover. Disabled controls receive no press treatment.
Their existing focus rings and both reduced-motion policies remain in force.

## Home workspace cards

Resume rows keep project artwork, two-line titles and wrapping metadata on a
continuous ruled surface. Wide rows put the next step alongside identity;
compact rows stack. Result and menu actions remain independent. No tile edge,
shadow or entrance animation remains. Written status links wrap naturally and
retain unknown/error/partial coverage.
Greeting/date use the existing minute timer and `todayBrief.greeting`.
No new data reads, shortcuts or motion are introduced.

## Workspace reading surfaces

Workspace documents (WORKSPACE.md, PLAN.md, SOURCES.md, AGENTS.md) read as
files in the Preview pane's Markdown view; the workspace panel, its section
strip and its section stylesheets are gone (COO rebuild L4, 2026-11-10). A Result's review bar
(`.result-review-bar`) sits above the file. Document Result covers in
Library → Results (`ResultThumb`) are inset sheets on `--surface-sunken`,
with listed type and tabular size. They never read or invent file contents.
No new colors or motion; theme and text-size inheritance stay shared.

## View control levels

`Tabs` defaults to `ui-level-primary`: destination views retain their underline.
Pass `level="secondary"` for task scopes; the same class on `Segmented` gives
Library/Results review filters one quiet pill treatment.
Library's manual primary row carries the same primary class. Selection remains
`aria-pressed`, each group keeps its name, and Tab/Shift-Tab plus Enter/Space
remain native. These classes change no routing, focus or state behavior.
Segmented controls elsewhere retain their existing styling unless opted in.

## Words on buttons and toasts (P1-04)

The person-facing words and the collisions they settle are in
`scripts/check-plain-words.mjs` (`WORDS`), which writes the glossary's "Words
we use" table and refuses the retired words in shown text. Verbs:

| Verb | Means | Example |
|---|---|---|
| New | Makes a blank thing here | New task, New workspace |
| Add | Brings something that exists into this place | Add source, Add a blocker |
| Import | Brings an outside file or link into the Library | Import a source…, Import into Library (Add's button for a Library source) |
| Create | The last button of a form that makes the thing | Create workspace |
| Write | An agent drafts text for you to read | Write this week's take |
| Draft for the Main agent | Fills the Main agent's message box; you send | Today's suggestions |
| Apply / Apply ticked | Makes the change a review card describes | Tidy up |
| Open / Show / Close | Moves you to a place / reveals it in place / dismisses it (never "esc" as a label) | Open Running |

A `⋯` menu is `<Thing> options` when it belongs to a whole surface (Section,
Workspace, Automation, Send) and `<Thing> actions` when it acts on one item
(File, Canvas, More actions on a turn); Library actions predates the rule.
A toast is a sentence: a capital first letter and no `+ ` token ("Copied",
"Automation Weekly added"); the checker refuses the rest.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Styling" except its
feature-surface subsections (those are in [Surfaces](surfaces.md)), moved
there word for word on 2026-10-13 (only relative links were re-based). Read it
before touching CSS, tokens, themes, page chrome, the app shell or motion.
Keep it current with the code in the same change, as AGENTS.md asks.

## Day and week reading surfaces

Today and Weekly review use opaque section headings and ruled factual groups,
with no entrance fades. Utility surfaces keep factual groups too: Launch checks
pin time, Accounts checks select routing, Usage checks skip only built-in caps,
and overnight names unreported costs. Recommendation drafts sit inside native details; the
action stays outside. Day/week-scoped keyed rows preserve mounted preview state
on equal refreshes, with changed content and scope creating fresh cards. Day
ring geometry is unchanged except its larger center label field and 240px
display; SVG font compensation retains the previous physical text size. Agenda
week group labels use muted text and hairlines, never color alone. No new data
reads, agent calls, motion or experimental effects are added.

## Styling

The side panel summary card's Recall list uses hairline item separators,
ink subjects, body paragraphs and muted monospace paths. Count and lifecycle
labels use caption text; feedback actions wrap within the card. Its rules
live in `side-panel.css` inside the components layer and use existing tokens.
The former composer Recall pill styles are removed; context chip rules now
target only the remaining context pills.

Tasks uses ruled board/week lanes instead of nested cards. Board/Not this week
titles clamp after three natural lines; priority and label chips remain plain.
Empty columns use words directly below the group header. Today has an accent
header rule; selection and drag targets retain their stronger fills. List rows
start at 40px (36px Compact) and grow with wrapped metadata. The existing scroll
floor and checkbox gutter stay intact. Detail properties are ruled rows,
Work on this is an inline section, and the shared editor has no entrance animation. Native
priority controls grow with text. The pane editor stacks its field grids when
its 420px container query matches or the window is at most 53.75em; otherwise
the grids keep their two- and three-column layouts. The modal keeps its own
responsive layout.
No data, view preference or route changes.

One token system (frontier F2, 2026-09-29): a neutral palette with one accent
(Crimson accent `--primary`), quiet type, hairline borders and three elevation levels,
the same in light, dark, Liquid Glass and custom palettes.
`components/HollowMark.tsx` renders the saved app icon in the rail's existing 18 px
mark slot; Reset restores the transparent Studio doorway asset (`src/assets/brand/`).
It changes only the image source, preserving the header geometry and accessible name.
Home's decorative doorway panorama stays stock. See
Product Studio for the earlier implementation map;
its palette and motion notes are superseded by this section.

`components/DoorwayLandscape.tsx` adds the bundled dithered doorway panorama above
Home's status links and resume rows (a 56px populated crop), or Ask on empty
Home, and the empty full-size chat's starter prompts. It is decorative,
static and removed with the empty state after the first message. A transparent CSS
mask fades into the actual theme surface; no network or image-processing loop runs.
Compact chat omits it; forced colors hides it. See [Doorway landscape](../DOORWAY_LANDSCAPE.md)
for the artwork provenance, dimensions and verification limits.

`components/SessionIndicator.tsx` is the one session indicator (asks 10-11 A1/A2): each
state is its own shape (running = open ring with one turning arc, queued = dotted ring, waiting on
you = solid "!" disc, failed = ring with ×), colour second. `SessionRollupBadge` rolls a workspace's
conversations into one mark + count (collapsed rail head, Work rows, Mission Control groups; the
map draws the same shapes on its canvas); `MainAgentIndicator` is the Main agent's variant (doorway in
the ring: resting / watching / working / waiting / paused by the cost cap) for the top bar's orb
(`MainAgentOrb.tsx`; an Ask button 10-13 to 10-14) and its space's header. States are pure in `sessionIndicator.ts`; `store/sessionStates.ts` derives every
conversation's state from the run queue, background chats, approvals, "waiting on you" and Mission
Control's feed (no polling; opening a failed chat marks its failure seen). Only the running arc
animates (`styles/session-indicator.css`), paused by `--ambient-play` and off under both
reduced-motion gates. See [Active session motion](../ACTIVE_SESSION_MOTION.md).

### Collapsed rail state

Pinned minis and Chats carry `SessionRollupBadge` with the state in
their accessible names. Work carries one rollup for project sessions not
represented by a visible pinned mini, deduplicated by session key; Chats
has its own mini. The extra Work badge appears only when collapsed.
The minis load from `RailMinis.tsx` only while collapsed, inside a local
loading/recovery boundary; the rest of the rail and list-error Retry stay eager.
Chats opens its latest conversation or starts the first through the
existing navigation guards. Expanded build workspace headers carry git state
once; idle conversations retain provider marks, active ones provider-labelled
state marks. Relative ages use `minuteClock.ts`.
Age/status text stays opaque; selected-row metadata inherits the row's ink
so the selection fill cannot reduce its contrast below the text threshold.

The rail keeps a static loading skeleton until the first workspace list
resolves. A failed list retains previous rows and offers Retry locally. In the
collapsed rail the error icon and Retry stay visible; the full sentence stays
in accessible text and the tooltip. The open workspace's top-bar status keeps
its glyph, full accessible name and tooltip at <=1100 px, hiding only its words.
Obsolete 700/600 px rules were removed from `styles/workbench.css`; the native
minimum is 720 px. Native top-bar fit remains a hand check.

### Tokens (`styles/tokens.css`)

Every design value is defined once, in [`styles/tokens.css`](../../src/styles/tokens.css):
the light `:root` block, the `:root[data-theme="dark"]` block and the
`:root[data-glass]` glass tints. Component stylesheets only read tokens.
The token *names* are the shared vocabulary; add names, don't rename them.

Groups: type (`--fs-caption` 11 … `--fs-display` 28, all scaled by
`--text-scale`; nothing renders below 11 px), density (`--row-h`, compact
mode), depth and neutral fills (derived with `color-mix` from `--ink` /
`--primary`), weights 400/500/600 only, tracking, line heights, radii
(`--r-xs` … `--r-pill`), elevation (`--elev-1..3`; resting surfaces are flat
with a hairline), focus (`--ring`), motion (`--ease*`, `--t-fast` 120 /
`--t-med` 200 / `--t-slow` 300), spacing (`--sp-1` … `--sp-8`), colour and
fixed values. The full table is in the
frontend archive.

**Base layer (T0, 2026-11-20).** `tokens.css` ends its token block with a second, additive
group adapted from t3code's MIT design system (licence notice in
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md); Hollow's own colours, no T3 marks).
It defines no colour: only derived mixes of existing tokens, so the four-place colour
registration does not apply, and it sits after the dark block so `artifactSandbox.test.ts`
still reads only colours there. Groups: contrast (`--contrast-boost`,
`--contrast-border-boost`, `--ink-boosted`, `--muted-boosted`, `--hairline-boosted`), alpha and
opacity vocabulary (`--alpha-*`, `--op-meta*`, `--op-disabled`), type (`--lh-caption`, `--lh-meta`,
`--lh-snug`, `--fw-*`, `--ls-wide`..`--ls-widest`), spacing and layout (`--sp-half`, `--sp-1h`,
`--sp-2h`, `--topbar-h`, `--chat-measure`, `--card-w`, `--content-gutter` and `--content-gutter-wide` (switch with a named window container query, never a bare media query), `--hit-coarse`), radii
(`--r-float` 12, `--r-shell` 16, `--r-sheet` 20), layers (`--z-*`), elevation extras
(`--elev-composer`, `--elev-dialog`, `--elev-popover`), easing (`--ease-standard`, `--ease-decel`,
`--ease-drawer`, `--ease-css-out`, `--ease-toast`), durations by use (`--dur-*`), panels
(`--panel-duration`, written by PR 0.2, default 0ms, and `--panel-ease`) and stillness (`--dur-pulse`,
`--dur-skeleton`, `--dur-ping`, `--dur-shine`, `--steps-*`). **Motion rule:** multiply a duration by
`--motion-scale` (1, or 0 under reduced motion or the system preference) at the call site; loops
are stepped, run only while visible and motion-safe, and hold still otherwise. `baseLayerTokens.test.ts`
pins the names and the one-switch reduced-motion rule.

**Panel animation duration (0.2, 2026-11-20).** `store/panelAnimation.ts`: a persisted
0 to 400 ms setting (`hollow.studio.panelAnimationMs`, default 0 = off), Settings → General →
**Panel animation** (range, readout, Turn off button). It writes `--panel-duration-user` on
`<html>`; `--panel-duration` reads it, and reduced motion (app setting or system) and navigation
(`data-panel-still`) force 0ms. `applyActivePage` calls `suppressPanelAnimation()`, which holds
panels still until two frames after the new page paints. `panelDurationMs()` is the gated value for
JS; `createPanelPresence(open)` keeps a closing panel mounted for its exit duration and a reopen
cancels it. Only panels read it: other animations keep their own `--dur-*` times. Contract tests:
`panelAnimation.test.ts` (both motion preferences, suppression, presence).

The accent marks only the primary action, focus and selection.
`--accent-amber` is the warning colour (`var(--warning)`) and `--accent-teal`
is a quiet slate for info marks; "done" and "ok" states use `--success`. The
syntax palette (`--syn-*`) uses red keywords, sage strings, blue calls,
steel types and ochre literals. Fonts: Inter Variable
(`--ui`, `--display` is an alias) and JetBrains Mono (`--mono`).

**Unused classes:** `scripts/check-orphan-css.mjs` (in `npm run test:scripts`, night ST3
2026-10-14) fails when a stylesheet under `src/` styles a class that nothing in `src/`,
`src-tauri/src`, `public/` or `index.html` names or builds (`kind-${k}`, `'diff-' + t`,
`l${level}`; a prefix inside an `id=`, `for=`, `aria-*`, `data-*`, `href`, `key` or `name` value
doesn't count). A prefix keeps every class that starts with it, so `is-${status}` keeps all
`is-*` rules. Specs and docs don't count; CodeMirror's own `cm-*` classes in `codemirror.css` (not the app's
`cm-md-*`/`cm-ai-*` decorations) and the generated kit sheet are skipped; a class only a library adds goes in `LIBRARY_CLASSES`.
Deleting a component's last use of a class means deleting its rules in the same change.

`npm run css:check` (`scripts/check-css-tokens.mjs`, also run by `npm run
build`) fails on a raw hex colour or a px font size anywhere outside
`tokens.css`, on `!important` outside the `@layer a11y` block, on an undefined
custom property read without a fallback, on a dangling `animation-name`, on a
stray declaration, on an unlayered style rule (including rules after a layered
prefix or inside a media query), and on an infinite animation that neither opts into the
ambient pause nor is live work (`LIVE_WORK`, `LIVE_WORK_KEYFRAMES`). There is
no allowlist for the token contract. **Contrast** is checked the same way
(`scripts/check-contrast.mjs`, overnight F5): every text token the app paints
as `color:` must reach WCAG AA, 4.5:1, on `--canvas`, `--surface-soft`,
`--surface-card` and `--surface-raised`. The same holds for `--on-primary` and
`--on-error` on the primary and error fills, and for the `--syn-*` colours on
code surfaces, in both themes. A label drawn on an error fill (`.btn.danger`,
the browser pane's numbered marks, a failed delivery mark, an active danger mode button, the attachment remove button on hover)
uses `--on-error`.
That is white in light, and dark text in dark, where the error colour is light.
Custom palettes derive it like `--on-primary`.
- **Changed to pass:** in light, `--muted-soft`, `--success`, `--warning` and
  three syntax colours were darkened, which brings `--muted-soft` close to
  `--muted`. In dark, `--muted-soft` and `--syn-comment` were lightened.
- **`KNOWN` lists pairs that still fail, with the reason.** They print as
  warnings, and the list may only shrink. It is empty since overnight F5b, and
  its test pins it empty.
- **Not covered:** custom palettes and glass, derived at runtime; translucent
  and `color-mix()` surfaces. The Canvas Kit's own fallback colours (used only
  without a host) still hold the old values.

Don't move a colour token without running `npm run css:check`.

### The cascade (`@layer`)

[`src/styles.css`](../../src/styles.css) declares the layer order first:

```
@layer tokens, base, components, shell, glass, a11y;
```

Layers: `tokens` (`tokens.css`); `base` (`foundations.css`: reset, form
defaults, keyframes, focus ring, exit choreography); `components` (`ui.css`
first, then every page and component stylesheet, lazy ones wrapped in
`@layer components {}`; every rule for an element lives in the stylesheet
that owns it, with no override pass); `shell` (only app-wide element
affordances in `shell.css`, plus a component's deliberate exception in a
trailing `@layer shell {}` block); `glass` (`glass.css`); `a11y` (the
reduced-motion gates, the only `!important`, `.sr-only`, `.hidden`).

The shell's 2px keyboard outline wins over component focus shadows. Buttons in
`.ui-tabs`, `.side-pane-tabs` and `.ws-section-row` use an inward 2px offset
so their scrolling strips do not clip it. Incoming keeps its selected inset bar
alongside the focus shadow. Inputs retain their soft focus treatment. Row quick
actions remain visible at half opacity; they and chat-chain toggles are 24px.

`css:check` inspects rule blocks in `src/styles/*.css` and `src/styles.css`
rather than searching for the word `@layer`. Layer names must be declared in
the manifest; tokens, base, glass and a11y are restricted to their owning sheets.
The only unlayered exception is editor-scoped rules in `codemirror.css`,
because CodeMirror injects an unlayered theme. Shared `.rbk-card` sizing and
custom-icon, canvas-look, export and past-day overrides all use components.

**Inline AI in the editor (overnight 10-11 B5).** `editor/inlineAi.ts` adds
to `MarkdownEditor` (only when the host passes `ai`; `WorkFileEditor` does
for every editable file): a selection bar (a CodeMirror tooltip: Rewrite,
Shorten, Expand, Fix, Translate…, Ask…), the suggestion as an inline diff (a
mark on the original, a block widget beneath with Accept/Reject; Tab or
⌘Enter accepts, Esc rejects, at `Prec.highest`; Tab accepts only with the
cursor on the suggestion's lines and is indent everywhere else; ⌘⇧Enter
focuses the bar, ←/→ move, Esc goes back) and the `/` menu at a line start
outside code and frontmatter (Task, Table, Callout, Date, Link a note, Ask
Hollow…), a second completion source beside `[[`. State, request and menu
items are `inlineAiModel.ts` (node-tested): one suggestion, mapped through
edits elsewhere, dropped when its own text is edited or deleted (the answer
would be stale); Accept is one undoable change (`userEvent: input.ai`) that
keeps the selection's leading/trailing whitespace; context never splits a
UTF-16 pair. Each request carries a `requestId`; Dismiss, Esc, a newer
request and closing the editor call `inline_ai_cancel` with it, which
requests that run's `StopSignal` (idempotent; a no-op once finished; an early
cancel is remembered so the request never starts). The host's `key` (the
path, from `WorkFileEditor`) carries the live suggestion across a remount (a
vault change re-reads the file): it waits 2 s under the key, and the new
editor re-attaches it via `inlineAiModel.reanchor` (same text, nearest
place) or shows "The suggestion expired because the file changed."
(`role="status"`); unclaimed, a pending call is cancelled (overnight S3).
The key is a per-`WorkFileEditor` instance id plus the path, and its
re-read/rename setters run in one `batch` so the editor remounts with its
old state. `carrier()`'s `update()` is the one place a pending suggestion
leaving the field uncarried gets cancelled (and, when no `userEvent` caused
it, leaves the expired notice). Request ids are `crypto.randomUUID()`.
The model call is the backend's
`inline_ai` (`inline_ai.rs`): the default agent through
`claude_runner::run_oneshot_supervised` with that stop signal, every tool denied, a temp cwd, 90 s, metered
`inline_ai`; selections over 20,000 characters are refused; the reply loses
a wrapper fence, quotes or a "Here is…:" line only when the selection had
none (never a fence for Ask). `user-only` in the command map: agents write
text with their own tools; so is `inline_ai_cancel`. Agents without
tool restrictions (Antigravity, Grok) are refused by the runner and the
error shows in the suggestion slot. Replies arrive whole (one-shots don't
stream). Mock: `inline_ai` answers after 700 ms.

`codemirror.css` (imported by `editor/MarkdownEditor.tsx`) is deliberately
**unlayered**: CodeMirror injects its own theme as an unlayered style, and an
unlayered rule beats every layer, so overrides of its properties must be
unlayered too. Keep that file to `.cm-*` rules.

Two consequences worth knowing. A layered normal declaration loses to an
inline `style=""`, so a resizable width is written as a custom property
(`ConversationHistory.tsx` sets `--chat-aside-w`), never as inline
`width`, or the narrow-window rules could not override it. And specificity
only matters *within* a layer. Don't add selectors to win: change the owner
rule (the `shell` layer is only for the app-wide element affordances above).

### Appearance preferences

Theme samples are passive labelled miniatures of title, body, selection and
action styling; only Use selects the theme. They derive readable colors from
listed metadata through the existing clamp, without reading full theme files.
Stock sample colors follow the current base palettes; native Glass is not
simulated by a decorative sheen. Selected cards have an outline and In use.

Settings → General → Appearance includes **Match macOS**, a live system
preference saved as `theme=system`. Explicit stock, glass and custom selections
stop following. `theme()` still reports the effective light/dark polarity.
The app settings bridge reads and accepts `system` as the saved preference.
Selection tickets protect boot, picker and bridge reads; a late custom response cannot
replace a newer choice or clear its saved pointer. Storeless windows use the
same boot polarity and follow system changes. Merely following does not persist
the effective polarity. Native glass uses an explicit window theme with the
same UnderWindowBackground material for both polarities. Leaving glass clears
the native override (`set_theme(None)`): tao sets application-wide appearance,
so keeping an explicit value would prevent WKWebView from following macOS.
Picker and bridge requests reserve their ticket before reading the palette,
including while a newer choice is still pending. Stale picker errors stay quiet.

`store/textScale.ts` owns the four visible sizes (90, 100, 112, 125); unknown
stored values, including the formerly hidden 150, normalize to 100 without a
startup write. Frames receive `--app-text-scale` live, including named Looks;
the default canvas/document bodies scale from 14/15 px without reloads.
Light/dark tokens set `color-scheme` for native controls. Increase Contrast
uses opaque ink hairlines and solid focus rings. Reduce Transparency and
Increase Contrast replace glass tints with opaque surfaces over an opaque body.

Custom core text tokens meet 4.5:1 against canvas, soft, card, strong and raised
surfaces. A mid-tone surface mix is pulled toward the original background when
needed to retain one readable text polarity; the source file stays unchanged.
The lazy editor's pure `settings/themeContrast.ts` reports the minimum derived ratio and whether display colors
were adjusted. Incomplete or unsupported drafts remain unmeasured; Rust still
owns Save and its validation. Named canvas Looks retain their own adjustment
receipts and unsupported-color warnings.

Native appearance, accessibility preferences and WKWebView still need the
written hand checks after installation; browser mocks are not native proof.

### Custom themes (kitty-style palette files)

Workspace **Canvas look…** reuses the pure derivation in `src/customThemeVars.ts`
directly. Named Looks use raw derivation followed by
their own foreground adjustments; the app independently clamps core text.
Theme files are unchanged. See
[Workspace canvas look](../CANVAS.md#workspace-canvas-look) for contrast targets,
cache refresh, fallback and the visible menu path.
The export Look selector and layout-correction pictures reuse these pairs;
named export palettes are read afresh and baked into the capture document before
scripts start. Choosing an export theme does not change the app or workspace theme.

Users can drop kitty-format `*.theme` files into the themes folder
(`~/Library/Application Support/Hollow/themes/` on macOS,
`~/.config/Hollow/themes/` on Linux) and pick them from Settings →
General → Appearance. The Rust parser in [`themes.rs`](../../src-tauri/src/themes.rs) accepts
both `accent = "#hex"` and the kitty space form `accent #hex`; only
`accent`, `foreground`, `background` are required, the rest fall back to
derived values. On first run the folder is seeded with `Graphite.theme`
and `Solarized Dark.theme` as starters.

**`CUSTOM_VAR_KEYS` and `deriveCustomVars()` in
[`store/theme.ts`](../../src/store/theme.ts) are the contract:** a new
colour-bearing token in `styles/tokens.css` must be appended to both, and to
`THEME_TOKENS` in [`htmlSandbox.ts`](../../src/components/htmlSandbox.ts) so
artifact frames and the Canvas Kit get it (`artifactSandbox.test.ts` fails on
a colour the export misses).
`setActiveTheme()` is the single theme switcher. Liquid Glass is a structural
overlay (`html[data-glass]`, `styles/glass.css`) over native macOS vibrancy,
verified only in a built `.app`.

### Page chrome and shared primitives (frontier F3)

[`components/ui.tsx`](../../src/components/ui.tsx) holds the one version of each
page primitive, styled once in [`styles/ui.css`](../../src/styles/ui.css) (the first
file in the `components` layer, so a page may still size a primitive in
context with a more specific selector). Tasks, Library, Automations and
Settings build their chrome from them; new pages should too.

Page titles use the existing 20px heading step (`--fs-title` aliases
`--fs-xl`), 600 weight and 1.25 leading. Shared header padding is 12px
vertically, with `--page-gutter` horizontally; Tasks and Library have no
local padding override. Text size still scales the type and responsive gutter.

### Window widths

`tokens.css` names compact (<=53.75em, 860px at Default), medium (<=68.75em,
1100px), and wide (>68.75em). These are queries against the root named
`hollow-window` inline-size container. The root font scales with `--text-scale`,
so larger type moves the layout thresholds. Media-query em uses the browser's
initial font and does not follow the app's text setting; a browser regression
compares both mechanisms. `windowLayout.ts` reads `--window-compact-em`,
`--window-medium-em` and the computed root font. App collapse/Escape,
rail navigation, history selection, the Preview pane's narrow-window policy
and composer focus all use that reader. Automations' back button and list split share
the medium tier. Resizing within a compact range preserves a manually opened rail;
leaving compact preserves the person's collapsed choice.
Compact startup diagnostics keep status words, tiers and launch arguments
on extra rows. Home summary headlines wrap beneath their heading.
The flex-sized transcript wrapper is a named `chat-wrap` size container.
At heights up to 20em, Jump to latest and Back to canvas share the tools band
outside the scroller; taller transcripts retain their floating controls.
Recovery buttons stay outside the tools toolbar so its semantics are unchanged.
They keep their position after transcript actions in the focus order, including
at normal heights. The scroller remains a direct child. Chat hosts must give
the wrapper a definite flex/grid height. Its padding query names `chat-column`
so nested height containment cannot change the width it measures.

Consolidation deliberately moves former 600-1050px cutoffs to compact or
medium: layouts stack earlier so the native minimum and larger text have room.
Root `rem` dimensions now follow Text size too; use bounded widths when a
minimum could otherwise exceed a pane. Inline-size containment also makes
the root a containing block for fixed descendants. Preserve the viewport-sized
`html, body` height contract and inner-pane scrolling, including secondary
windows; a scrolling document needs its own fixed-overlay review.

`scripts/check-window-breakpoints.mjs` lists window widths and rejects new
ones; its tests run in the script gate. The remaining sub-720 media blocks
are labelled individually for the 390px Playwright host. Unnamed local
container queries still measure their pane/card, including narrow preview
and quick-entry content. The isolated canvas kit owns its own responsive CSS.
Tasks gives its list a minimum usable height at every width and scrolls
the owning page when wrapped bulk actions need more room. Medium windows
stack the backlog below two day columns; an independent `task-list` container
wraps row metadata when the details pane narrows the available list. Wide week
cards fit available space with a 16em minimum instead of forcing seven columns
beside the backlog.
Selected-row metadata uses `--body-strong` against the selection fill;
overdue dates retain their error color. The
minimum layouts keep the checkbox gutter and its outward focus ring inside
the list scroller. Both required
minimum projects run at 720x480; Largest uses 125. Their checker verifies
each project's viewport independently.

### The app shell

The context bar shares the canvas plane; the 240/60px rail uses static
row-owned selection edges without measurement observers. Compact Glass rail
overlays use the opaque soft surface so page text cannot bleed through. Task
row clicks reuse `viewTransitions.ts` with an explicit detail target, leaving
the page still. The target/source/page inline names restore exactly. Immediate
task selection, Close and New flush any waiting visual update before changing
state. Motion gates and existing overlay-click recovery remain authoritative.

```
.app.hollow-shell (flex row, 100vw × 100vh)
├── .work-rail.rail      (Rail.tsx: 44 px brand row (name only), labelled
│                         Work / Tasks / Library / Automations (nav
│                         "Main"), the workspace tree, and the Settings row
│                         (nav "Shared tools"); collapsible)
└── .canvas              (main column, flex:1)
    ├── .top-bar         (44 px, asks 10-11 A3. Left: sidebar toggle,
    │                     back/forward, breadcrumb, then the open workspace's
    │                     own state (topBarSlot: live roll-up, branch chip).
    │                     Right: activity dock, then one global group:
    │                     Search (⌘K), New ▾ (conversation ⌘N, chat in Chats,
    │                     workspace, Add…; appMenu.ts runMenuAction), the
    │                     Ask (Main agent presence); far right, the workspace
    │                     page's Project files button (topBarPaneSlot;
    │                     COO rebuild L4, where the workspace panel
    │                     button was))
    └── .canvas-split    (run 15 P1: the page and, only while it shows
        │                 something, the Preview pane beside it, under the
        │                 top bar so the bar never moves when a preview opens)
        ├── .page.active (display:flex column, flex:1 — the active page)
        └── .preview-pane (PreviewPane.tsx, lazy; resizable, sidePaneWidth)
```

### Product-quality layer (2026-09-28)

The overnight product-quality build (audit H01–H15, Product Studio R01–R04;
status and evidence in the handoff) added
shared primitives. Use them instead of new one-off controls:

- **`components/Select.tsx`** (`Select` / `MultiSelect`) is the only in-app
  dropdown: no native `<select>` elements. `components/ActionMenu.tsx` is the
  overflow menu, `components/TaskEditor.tsx` the one task form.
- **`goToPage(page)`** (`store/workspaces.ts`) — the one user-facing page jump
  for the rail, Cmd+digit, the palette and the app bridge. "Work" always
  means the overview (H01); a bare `setActivePage('today')` keeps the open
  workspace and is for internal use only.
- **`selectWorkspace(id, key?)`** (`store/workspaces.ts`) switches to the
  Work page *before* it loads (since 2026-09-29), so a rail click from
  Settings, Library or anywhere else shows "Opening workspace…" rather than
  seeming to do nothing. A failure (`openFailed`) sets the Work page banner,
  prefixed "Couldn't open TITLE:", **and** shows an error toast, because the
  banner sits above a list the user may have scrolled. Smoke:
  `rail-tree.spec.ts`; the mock fails `read_workspace` for the id in
  `localStorage.mockWorkspaceReadError`.

### Interaction and motion

Ambient motion has one default-on Interface switch (the saved Experimental
visuals key is retained) and a lazy shared
controller; [Design system](../DESIGN_SYSTEM.md#experimental-visuals) defines
the active signal/root attribute every effect must use. The existing ambient
layer now follows real working state and this combined focus/a11y policy.

**Entrance animations: only on lists that genuinely arrive.** Solid's `<For>`
(`mapArray`) reconciles by `===`, so a list fed fresh IPC objects destroys and
rebuilds **every** row on every refetch — and an entrance animation on that row
replays, which is why a vault-change burst read as the list flashing. Rows
whose source is fresh objects (`.task-row`, `.list-row`, …) carry no entrance;
entrances stay only where the DOM is genuinely reused (keyed Board groups,
`.toolcall` rows keyed by stream event). Those rules carry
a comment saying why. Do **not** solve a replaying entrance with a
first-paint class gate keyed to a container; containers are rebuilt with
their rows.

Settings → General → Motion selects full or reduced motion; system Reduce
Motion always wins, and both gates live in the `a11y` layer. Motion stays
within 120–300 ms on one easing family; the only infinite animations are
status (spinners, `status-pulse`) and the one ambient layer below.

App JavaScript uses `src/motion.ts` (`motionReduced`) for the same combined
policy. Its live system query is reused per window; Map subscribes through
`motionQuery` and disposes its listener. The storeless `main.tsx` boot check
and isolated canvas kit keep their own checks. Dialog and snackbar exits
remove immediately under either preference, and Settings search uses an
instant scroll, as does Library's file outline. Frame init and live motion
updates send the same combined value and observe system changes. Full motion
retains the existing exit durations.
Spinners continue while the window is away; session arcs and chat working
marks deliberately pause (see [Active session motion](../ACTIVE_SESSION_MOTION.md)
and [Chat motion](../CHAT_MOTION.md)). The boot entrance changes only transform,
keeps content opaque from the first frame, and excludes the ambient layer.

**The shell's motion layer (overnight 10-11 B1).** `src/viewTransitions.ts`
runs the person's page jumps inside `document.startViewTransition`:
`goToPage`, `navBack` / `navForward`, and `visitWorkspace` (the rail's,
Work's and the palette's workspace and chat opens; it waits ≤120 ms for
the load so the new snapshot is the loaded page). `selectWorkspace` stays
still, so launch restores, merges, moves and agents' navigation never
animate. Only the page column animates (`hollow-page`, named only while
`html.is-transitioning`); the rail and top bar don't (the old root snapshot
is hidden, so glass can't ghost). Clicks don't pass through the overlay in
Chromium (it hit-tests `<html>` while one plays, whatever
`::view-transition { pointer-events }` says), so a click or right-click on
`<html>` mid-transition skips the motion and is replayed on the element under
the pointer, unless that is in the page column, the press didn't hit `<html>`
in place (a drag) or it is a double-click's second click (overnight 10-11/12 P4;
`tests/smoke/transition-click.spec.ts`). A
click handler may call
`morphFrom(e.currentTarget)` first (Work's cards, rail workspace and chat
rows): that element takes the page's name for the old snapshot and grows into
the page. No transition (the update runs directly) under either motion gate,
at launch (`is-booting`), in an away/hidden window, without the API, or while
one runs (never nested). The update runs when the engine has captured the old
frame (normally the next frame); one not started within 300 ms is applied and
the motion skipped, and any navigation in that gap applies the waiting one
first, so a later jump is never overwritten by an earlier one.
`styles/motion.css` holds the transition pseudo-elements and `.ambient-layer`
(App.tsx): two accent-tinted glows drifting over 72 s, transform-only, behind
every page (`.canvas` is positioned and isolated; `.work-page` is transparent
for it). Its `inset: -20%` overflows `.canvas`, which must stay `overflow: clip`:
an `overflow: hidden` box is still a scroll container, and a focus or
scrollIntoView in the chat once scrolled the whole column, top bar included, off
the window (10-11; `tests/smoke/top-bar-visible.spec.ts`). Peak tint `--ambient-tint` (6 % light, 8 % dark: the most
`--muted-soft` text keeps 4.5:1 on), hidden in glass, forced colours and
Increase Contrast. Shared buttons and icon buttons use fill feedback without
press scaling. Switch press motion requires both motion gates open
(`prefers-reduced-motion: no-preference` and not `data-motion="reduced"`).
Rail selection is static and owned by each selected row, with a forced-colours
outline. The old glide component, geometry helper and observers are removed.
Workspace session guides and token-sized rail geometry share workbench.css.
A lazy page's Suspense fallback is a page-shaped skeleton (`.page-skeleton`,
still `.page-loading` for the smoke test).

Archive: styling detail.

## Visual contracts moved from CLAUDE.md (2026-11-13)

Short visual rules that sat at the top of CLAUDE.md until the 2026-11-13
accuracy pass. Detail is in [Design system](../DESIGN_SYSTEM.md).

- Shared titles, neutral states, Library covers and press fills follow the
  design system; shell press geometry is in
  [Shared control presses](#shared-control-presses).
- Design system 3.0 and the shared static glyph artwork: reuse the lazy
  `ThresholdArt` component and the deterministic field engine
  (`src/thresholdField.ts`); never add a per-surface renderer or palette.
- The 3.0 shell uses token-sized rail geometry and static selection edges.
- Visual policy loads through `components/ExperimentalVisuals.tsx` with a
  local failure boundary. Static identity is independent of the saved motion
  switch; both obey accessibility
  ([Experimental visuals](../DESIGN_SYSTEM.md#experimental-visuals)).
- Settings → General → Appearance's Texture and Ambient motion controls reuse
  the shared policy owner and keep the old ambient preference key. Quiet thins
  identity art; Off uses plain drawings.
- Empty artwork is static stipple with plain accessibility fallbacks; no local
  animation observers remain. Line art, lazy boundaries, coverage and motion
  gates: [Empty views](../EMPTY_STATES.md).
- Overlay materials and question docks follow the shared 3.0 reading planes;
  their focus, timer, answer and navigation owners stay unchanged. Native
  title tooltips stay native.
- Canvas dither: [Canvases](../CANVAS.md).
