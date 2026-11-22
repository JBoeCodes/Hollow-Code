# Hollow design language

Status: adopted 2026-11-15 (Alex, 2026-11-14 10:47 PM, "use the design rules as written"). The decisions he took on the draft (N36 to N49) are in [Decisions](#decisions) and are rules, not proposals.

**Base look.** The base look is the Acme look adopted on 2026-10-27 (Sam's): neutral surfaces, Crimson accent for the one primary action, focus and selection, flat resting planes. Everything here refines that look; nothing replaces it.

**Pinned to main** (N49). This doc describes `main` at `983e3d46f` (2026-11-15). The draft was written against `a8f57a55a` (2026-11-13) and re-checked against `983e3d46f` for this version; values were read from source. Where code and doc differ, a rule marked "decided, app fix is row DL1" is the target and the code is behind. A rendering check of the installed build `d3bb1f9` (dark theme, Compact density, one window size) agreed on structure. `d3bb1f9` is an ancestor of `983e3d46f` (477 commits behind) and its `tokens.css` differs only in a comment. Known differences:

| What | `main` (`983e3d46f`) | Installed `d3bb1f9` (checked 2026-11-13) |
|---|---|---|
| Dark colours | `tokens.css` values in [Colour, 2.3](#2-colour) | Measured about `#212121` canvas, `#1b1b1b` rail, `#292929` composer, `#de9188` primary fill: a user palette ("Obsidian"), not a code difference (N37) |
| Rail width | 240px default, draggable 200 to 420 | Measured about 260px: a drag (N46) |
| Bypass always-ask list | six actions, plus the GraphQL class and the apply of `workspaces.archive_shared_docs` | seven actions (`tracker.archive_task` too), plus the GraphQL class |
| Plain words | no "Quick chats" ban | "Chats, not Quick chats" banned |
| "Not verified" in a run record | not traced on main | green mono (an app issue, see [Still open](#still-open)) |

Check a capture before relying on an exact value. What no review looked at is listed under [Not yet verified](#not-yet-verified).

## How to use this

Use it before building a new Hollow feature or changing a surface, so the feature matches what already exists.

1. Read [Foundations](#foundations) for tokens. Write no raw hex, px font sizes or hand-built shadows (the checker's actual scope is in [CSS conventions](#9-css-conventions)).
2. Read [Components](#components) and reuse the primitive that already does the job.
3. Read [Principles](#principles) for the rules that decide whether a feature belongs at all.
4. Run the [New-feature checklist](#new-feature-checklist) before the change merges.
5. Follow [Decisions](#decisions) (N36 to N49) for times, colour, tabs, selected rows, chips, disclosure, chat width, buttons, Settings names, rail width, confirm cards and this checklist. [Still open](#still-open) lists the gaps still to measure or fix.

Sources are cited inline as `path:line` in this repo unless a path starts with `outputs/` or `workspaces/`, which are in the Hollow workspace of the person's vault. The draft and its reviews are in that workspace's `outputs/design-language-2026-11-13/` (`part-1-foundations.md`, `part-2-components.md`, `part-3-principles.md`, `review-combined.md`).

## Foundations

Source for everything here: `src/styles/tokens.css` (361 lines), `src/styles/foundations.css`, `src/styles.css:10`, and the design rules in `docs/DESIGN_SYSTEM.md`. `scripts/check-css-tokens.mjs` enforces part of the token-only rule (raw hex, px font sizes and `!important` outside the `a11y` layer; see [CSS conventions](#9-css-conventions)).

### 1. Layer order

`src/styles.css:10`:

```css
@layer tokens, base, components, shell, glass, a11y;
```

| Layer | Holds |
|---|---|
| `tokens` | `:root` values, theme blocks, glass tints (`src/styles/tokens.css`) |
| `base` | reset, document defaults, shared keyframes, primitives (`foundations.css`) |
| `components` | component sheets (e.g. `motion.css`) |
| `shell` | app-wide element affordances no component rule may undo (disabled cursor, button focus ring), plus a few deliberate exceptions such as the tab focus offset. Not the layout container (`src/styles/shell.css:1`, `:21`) |
| `glass` | Liquid Glass overlay |
| `a11y` | motion, transparency, contrast and forced-colour gates, last so they win |

Component CSS goes in `@layer components { ... }` in `src/styles/*.css` (see [Components, Foundations](#foundations-that-every-component-uses)). The `a11y` layer is the only place `!important` is allowed, for motion preferences and for the contrast tokens (`--hairline`, `--ring`, glass borders under `prefers-contrast: more`, `foundations.css:664`). `CodeMirror` and inline styles can outrank a layered normal declaration, so the gate has to come last (`foundations.css`, "Motion preferences" comment).

### 2. Colour

#### 2.1 Light theme (`:root`, `tokens.css:189-239`)

| Role | Token | Value |
|---|---|---|
| Canvas (page) | `--canvas` | `#ffffff` |
| Soft surface | `--surface-soft` | `#f3f3f4` |
| Card | `--surface-card` | `#ffffff` |
| Raised | `--surface-raised` | `#ffffff` |
| Cream strong | `--surface-cream-strong` | `#e8e8e9` |
| Sunken well | `--surface-sunken` | `color-mix(ink 3%, canvas)` |
| Hairline | `--hairline` | `#d5d6d8` |
| Soft hairline | `--hairline-soft` | `#e8e8e9` |
| Ink (main text) | `--ink` | `#1a1a1a` |
| Body strong | `--body-strong` | `#393839` |
| Body | `--body` | `#454545` |
| Muted | `--muted` | `#626262` |
| Muted soft | `--muted-soft` | `#666666` |
| Primary (action) | `--primary` | `#b5251c` (darker, for small text) |
| Primary active | `--primary-active` | `#dd3227` (signature red) |
| Primary disabled | `--primary-disabled` | `#d6c7c5` |
| On primary | `--on-primary` | `#ffffff` |
| Primary soft | `--primary-soft` / `-strong` | `rgba(221,50,39,.06)` / `.12` |
| Accent teal (info mark) | `--accent-teal` | `#5f6b7a` |

#### 2.2 Semantic status colours

| Status | Token | Light | Dark | Soft fill (light / dark) |
|---|---|---|---|---|
| Success | `--success` | `#3d7634` | `#82c46f` | `--success-soft` `rgba(74,143,63,.14)` / `rgba(115,191,95,.18)` |
| Warning | `--warning` | `#896213` | `#ecbd62` | `--amber-soft` `rgba(184,132,26,.14)` / `rgba(236,189,98,.16)` |
| Error | `--error` | `#b73d3d` | `#e57272` | `--error-soft` `rgba(183,61,61,.12)` / `rgba(227,108,108,.16)` |
| Info (slate) | `--accent-teal` | `#5f6b7a` | `#9aa6b4` | `--teal-soft` `rgba(95,107,122,.12)` / `rgba(154,166,180,.14)` |
| Risk | `--risk` | `#855699` | `#c79be0` | `--risk-soft` `rgba(138,90,158,.14)` / `rgba(199,155,224,.18)` |

Aliases: `--accent-amber` = `--warning`. `--on-error` is `#ffffff` in light and `#1a1a1a` in dark (5.6:1 on the error fill).

Rules the source states:
- **One accent.** `--primary` is the only action accent. Teal is a quiet slate for info marks and amber is the warning colour. Neither is an action accent (`tokens.css:209-210`).
- **Status is never signalled by the action accent.** Do not use `--primary` for success, warning or error. Status words and icons carry meaning on their own (`tokens.css:186-188`). Checking that rule needs a mapping: green (`--success`) means a confirmed pass or done, never "not verified" or "unknown". Amber is warning, red is error, slate is info.
- **Text reaches WCAG AA (4.5:1)** on the surfaces the script lists, in both stock themes. `scripts/check-contrast.mjs` checks fixed token and surface lists (`:9`, `:24`, `:28`), runs in `npm run build` and `css:check`, and fails under the floor. A new text role must be added to those lists. It does not check custom palettes or Liquid Glass. See [Principles, Accessibility](#accessibility).
- **Muted text on tinted layers** is capped by `--ambient-tint` (6% light, 8% dark), the most tint `--muted-soft` can sit on and keep AA (`tokens.css:212-214`, `:302-305`).

#### 2.3 Dark theme (`:root[data-theme="dark"]`, `tokens.css:248-301`)

Neutral charcoal, no blue cast. These are the values in `tokens.css` on main, and they are the same in the installed `d3bb1f9`. The installed app was measured painting different dark colours (canvas about `#212121`, rail `#1b1b1b`, composer `#292929`, primary fill `#de9188`). None of those hex values appears in `src/`; the cause is a user palette ("Obsidian" shows beside General in Settings), not a build lag. Decided (N37): this table is the stock default, and a palette may change only what [Palettes](#palettes) lists. A re-measure on the default palette is still to be done ([Still open](#still-open)).

| Token | Dark value |
|---|---|
| `--canvas` | `#1a1a1a` |
| `--surface-soft` | `#151515` |
| `--surface-card` | `#222222` |
| `--surface-raised` | `#2b2b2b` |
| `--surface-cream-strong` | `#373737` |
| `--hairline` / `--hairline-soft` | `#414141` / `#303030` |
| `--ink` | `#f9f9f9` |
| `--body-strong` / `--body` | `#e7e7e7` / `#c8c8c8` |
| `--muted` / `--muted-soft` | `#ababab` / `#a3a3a3` |
| `--primary` | `#eb847d` (contrast tint, not a separate brand colour) |
| `--primary-active` | `#f0a39e` |
| `--primary-disabled` | `#55403f` |
| `--on-primary` | `#1a1a1a` |
| `--primary-soft` / `-strong` | `rgba(221,50,39,.10)` / `.18` |

Dark also sets `color-scheme: dark`, and its elevation values are stronger (see [6.3](#63-elevation-three-levels)).

#### 2.4 Fills, fixed colours and syntax

- Neutral fills derive from `--ink`, so every palette gets them: `--fill-hover` (ink 5%), `--fill-selected` (ink 8%), `--fill-strong` (ink 12%).
- `--accent-glow` (primary 26%) is for arrivals and the primary action's focus only. `--accent-wash` (primary 7% on card).
- Fixed in every theme: `--white`, `--black`, `--scrim` (`rgba(0,0,0,.22)`, behind dialogs and the palette), `--scrim-strong` (`.6`, over images).
- Syntax (`--syn-*`): light keyword `#b5251c`, string `#3b7556`, comment `#666666`, number `#805320`, function `#3a64b0`, type `#2f6f8f`. Dark uses lighter equivalents (keyword `#eb847d`, string `#9fcfa6`).

#### 2.5 Material roles (live aliases, not separate colours)

| Token | Resolves to |
|---|---|
| `--material-solid` | `--surface-card` |
| `--material-glass` | `--glass-well`, else `--surface-card` |
| `--material-glass-strong` | `--glass-well-strong`, else `--surface-raised` |
| `--material-edge` | `--glass-border-strong`, else `--hairline` |

Custom palettes re-derive colour tokens in `src/store/theme.ts` (`CUSTOM_VAR_KEYS`) and `src/customThemeVars.ts` (`deriveCustomVars()`). A new colour token must be registered in four places: both theme blocks, those two files, the glass block if glass needs its own value, and `THEME_TOKENS` in `src/components/htmlSandbox.ts`. Layout, type, radius, motion and elevation tokens are outside that contract.

#### Palettes

Decided (N37): the tables in this section are the stock default (light and dark). A palette (Settings > General > Appearance) may change only the colour roles in `CUSTOM_VAR_KEYS` (`src/store/theme.ts:106`):

- surfaces (`--canvas`, `--surface-soft`, `--surface-card`, `--surface-raised`, `--surface-cream-strong`) and hairlines;
- text (`--ink`, `--body-strong`, `--body`, `--muted`, `--muted-soft`);
- the primary family, `--accent-teal`, `--accent-amber`, `--on-primary`, `--on-error`;
- status colours and their soft fills (`--success`, `--warning`, `--error`, `--risk` and the `-soft` tokens);
- syntax colours (`--syn-*`).

Everything derived from those (fills, `--accent-glow`, material roles) follows. A palette never changes layout, type, radius, spacing, motion or elevation, and it never changes what a colour means: green stays a confirmed pass, red stays error. Palette contrast is advisory ([Principles, Accessibility](#accessibility)). Measured values in this doc must come from the default palette; the dark re-measure on the default palette is still to be done ([Still open](#still-open)).

### 3. Liquid Glass (`html[data-glass]`, `tokens.css:307-361`)

Glass is a structural overlay on either polarity. It overrides surfaces only and never redefines `--canvas`, which sandboxed iframes read and need opaque.

| Token | Light glass | Dark glass |
|---|---|---|
| `--glass-chrome` (rail, top bar, asides) | `rgba(239,239,239,.80)` | `rgba(32,32,32,.80)` |
| `--glass-panel` (resting planes) | `rgba(248,248,248,.88)` | `rgba(36,36,36,.88)` |
| `--glass-raised` (floating chrome) | `rgba(255,255,255,.94)` | `rgba(43,43,43,.94)` |
| `--glass-well` (reading well, AA) | `rgba(255,255,255,.97)` | `rgba(26,26,26,.97)` |
| `--glass-well-strong` | `rgba(255,255,255,.98)` | `rgba(26,26,26,.98)` |
| `--glass-border` / `-strong` | `rgba(40,40,40,.16)` / `.24` | `rgba(255,255,255,.12)` / `.20` |
| `--glass-saturate` / `--glass-brightness` | `180%` / `105%` | `165%` / `112%` |
| `--glass-blur` / `-heavy` | `18px` / `22px` | same |
| `--glass-text-halo` | `none` | `0 1px 2px rgba(0,0,0,.40)` |

Blur comes from the native window material, so `backdrop-filter` in CSS is a no-op for the page (`tokens.css` comment). Reduced transparency or increased contrast sets glass to opaque `--surface-soft`, `--surface-card` or `--canvas`, and blur to `0px` (`foundations.css`, a11y gate). Overlays and menus use the opaque raised surface even in Glass (see [Components, Menus](#menus-popovers-and-palettes)).

### 4. Type

#### 4.1 Families

| Role | Token | Stack |
|---|---|---|
| UI and prose | `--ui` | `'Inter Variable', 'Inter', -apple-system, BlinkMacSystemFont, system-ui, sans-serif` |
| Display | `--display` | `var(--ui)` |
| Mono (code, paths, permission values) | `--mono` | `'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, Consolas, monospace` |

Both are self-hosted in `main.tsx`.

#### 4.2 Scale

The base is `font-size: calc(16px * var(--text-scale))` on `:root`. Every step multiplies by `--text-scale`.

| Token | Size | Use |
|---|---|---|
| `--fs-caption` | 11px | counts, kbd, smallest meta; the floor (`--fs-micro` aliases it) |
| `--fs-meta` | 12px | row metadata, secondary lines |
| `--fs-body` | 13px | UI base (also `--fs-code`) |
| `--fs-md` | 15px | reading text, section titles (`--fs-prose`) |
| `--fs-lg` | 17px | panel titles (`--fs-prose-h2`) |
| `--fs-xl` | 20px | page titles, 600 (`--fs-title`, `--fs-prose-h1`) |
| `--fs-display` | 28px | rare: document covers |
| `--fs-art` | 7px | decorative SVG glyph cells only, never reading text |

Text scale (Settings > General > Text size sets `html[data-text-scale]`; labels from `src/store/textScale.ts`): Smaller (`90`) = 0.92, Default (`100`) = 1, Larger (`112`) = 1.12, Largest (`125`) = 1.25.

#### 4.3 Line heights, tracking, measure

| Token | Value | Use |
|---|---|---|
| `--lh-flat` | 1 | |
| `--lh-tight` | 1.25 | |
| `--lh-title` | 1.25 | shared page titles |
| `--lh-base` | 1.5 | UI |
| `--lh-prose` | 1.55 | reading surfaces |
| `--lh-reading` | 1.6 | |
| `--ls-tight` | -0.01em | titles |
| `--ls-display` | -0.02em | page titles and larger |
| `--ls-caps` | 0.04em | uppercase labels |
| `--measure` | 76ch | reading width for notes, summaries and help text. Decided (N43): chat keeps ignoring it. `.chat-main .chat-scroll` is `max-width: none` and fills its pane (`chat-workspace.css:55`), so lines run long by design (the chat-fills-pane rule) |

Rules the source states:
- Weights are 400, 500 and 600 only (`tokens.css:64-66`, `DESIGN_SYSTEM.md`).
- Tracking is 0 except `--ls-tight` on titles and `--ls-caps` on the few uppercase labels.
- Inter for UI and prose; JetBrains Mono for code.
- Optical sizing: `--type-optical: auto`. Page titles use `--title-optical-size: calc(20 * var(--text-scale))`.
- Numbers: `--type-numeric: tabular-nums lining-nums`, for metadata, counters, pills and dates, not all prose (`DESIGN_SYSTEM.md`, "Typography and numbers"). Several sheets set `font-variant-numeric: tabular-nums` directly; prefer the token role.
- **Text never carries texture.** Task text, checkboxes and drag targets have none (`DESIGN_SYSTEM.md`). Exact values, input contents, code, transcripts and user files are never dithered. See [Foundations, Texture](#10-texture-dither-policy).

### 5. Spacing

One 4px rhythm.

| Token | Value |
|---|---|
| `--sp-1` | 4px |
| `--sp-2` | 8px |
| `--sp-3` | 12px |
| `--sp-4` | 16px |
| `--sp-5` | 24px |
| `--sp-6` | 32px |
| `--sp-7` | 48px |
| `--sp-8` | 64px |

Layout tokens built on this scale:

| Token | Value | Notes |
|---|---|---|
| `--page-gutter` | `--sp-5` (24px) | 16px in compact window (`--sp-4`) |
| `--section-gap` | `--sp-5` | between page sections |
| `--viewer-pad` | `--sp-6 --sp-7 --sp-8` | |
| `--control-h` | 32px | 28px in compact density |
| `--row-h` | 30px | 26px in compact density. A minimum for single-line rail and list rows: two-line rows, Library and Settings rows set their own pitch (the installed app showed 37 to 62px row pitch in Compact) |
| `--row-pad-y` | 6px | two-line row; 4px compact |
| `--list-gap` | 2px | 0 compact |
| `--rail-w` / `--rail-w-collapsed` | 240px / 60px | sidebar rail. Decided (N46): 240 is the default (`RAIL_WIDTH_DEFAULT`); the person can drag it between 200 and 420px and it is remembered (`src/store/prefs.ts:79-81`, `Rail.tsx:105`, `:169`). The code does this today. The 260px measured in the installed app was a drag, not a different default |
| `--aside-w` | 320px | conversation history aside only, not the preview panel (see [Side panel](#side-panel)) |

Density: `html[data-density="compact"]` tightens rows and controls with the same components.

### 6. Radii, borders and elevation

#### 6.1 Radii

| Token | Value | Use |
|---|---|---|
| `--r-xs` (`--r-hair` alias) | 3px | chips other than labels, small controls (N40) |
| `--r-sm` | 5px | buttons, inputs, rows |
| `--r-md` | 6px | menus, list containers |
| `--r-lg` (`--r-xl` alias) | 8px | cards, panels, dialogs, composer |
| `--r-pill` | 999px | label pills (Tasks labels, project tags), status pills and badges (N40) |

The scale stops at 8px. `DESIGN_SYSTEM.md` restates it as "3/5/6/8px".

#### 6.2 Borders

Resting surfaces are flat with a 1px `--hairline` border. Dividers are `--hairline-soft`. The two media gates differ (`foundations.css:664`, `:675`): `prefers-contrast: more` replaces `--hairline`, `--hairline-soft`, the focus ring and glass borders with `--ink`; `forced-colors: active` does not replace root tokens and only gives empty, filtered and loading states a `CanvasText` border.

**Ambient wash.** Pages are flat, with one sanctioned exception (N41): `.ambient-layer` (`src/styles/motion.css:51-59`) sits under every page, behind all content, and paints two soft radial tints, `--primary` top right and `--accent-teal` bottom left, each at `--ambient-tint` (6% light, 8% dark). The installed app showed it as a faint warm patch top right on Home, Automations and Settings. It drifts only while an agent is working and the experimental layer is active (`ambient-drift`, paused by `--ambient-play`). Texture Off removes it; Full and Quiet keep it. Done in DL1 (2026-11-15); before it, on main no rule ties `.ambient-layer` to `data-texture-level`, so Off still shows it.

#### 6.3 Elevation (three levels)

| Token | Light | Dark |
|---|---|---|
| `--elev-1` | `0 0 0 transparent` (compatibility, flat) | same |
| `--elev-2` (popovers, menus) | `0 1px 2px rgba(24,20,30,.06), 0 8px 24px -6px rgba(24,20,30,.16)` | `0 1px 2px rgba(0,0,0,.35), 0 10px 28px -6px rgba(0,0,0,.55)` |
| `--elev-3` (dialogs, palette) | `0 2px 6px rgba(24,20,30,.06), 0 24px 56px -12px rgba(24,20,30,.26)` | `0 2px 8px rgba(0,0,0,.40), 0 28px 64px -12px rgba(0,0,0,.70)` |
| `--elev-edge` | `inset 0 0 0 transparent` | same |
| `--elev-action` | `0 0 0 transparent` | |

Resting elevation and specular edge are transparent. Menus and dialogs keep foreground shadows (`DESIGN_SYSTEM.md`). Old names `--lift-0/1/2` alias `--elev-1/2/3`.

Focus:

| Token | Value |
|---|---|
| `--ring` / `--sh-focus` | `0 0 0 3px var(--primary-soft-strong)` |
| `--focus-outline` | `2px solid var(--primary)` |
| `--focus-offset` | 2px |

The token offset is 2px, but the `shell` layer sets buttons, summaries and selects to a 3px offset, and tab buttons to `-2px` so scroll strips do not clip the ring (`shell.css:21`).

### 7. Motion

#### 7.1 Easing

| Token | Value |
|---|---|
| `--ease` | `cubic-bezier(0.3, 0.7, 0.3, 1)` (default) |
| `--ease-out` | `cubic-bezier(0.22, 1, 0.36, 1)` |
| `--ease-in` | `cubic-bezier(0.55, 0, 0.78, 0.22)` |
| `--ease-spring` | `cubic-bezier(0.34, 1.30, 0.64, 1)` |
| `--ease-snap` | `cubic-bezier(0.2, 0, 0, 1)` |
| `--ease-soft` | `cubic-bezier(0.4, 0, 0.2, 1)` |
| `--studio-ease` | `cubic-bezier(.16, 1, .3, 1)` |

#### 7.2 Durations

| Token | Value | Use |
|---|---|---|
| `--t-xs` | 60ms | |
| `--t-fast` | 120ms | hover, colour, press |
| `--t-med` | 200ms | shadow, filter |
| `--t-slow` | 300ms | lift, page-level |
| `--t-xl` | 480ms | |
| `--t-stagger` | 20ms | menu cascade; total delay capped at `--t-fast` |

Shared transitions: `--motion-base` (background, colour, border and opacity at fast; shadow and filter at med; transform with spring) and `--motion-lift` (raised items, shadow at slow with ease-out). Use the shared strings rather than new transitions.

#### 7.3 Reduced motion

Three layers, in order:
1. `foundations.css` (`@media (prefers-reduced-motion: reduce)`): every animation and transition runs at `0.01ms`, delay `0s`, iteration count 1 (see `foundations.css:705`, `:716` for exactly which properties each gate resets; the OS gate does not zero every `transition-delay`). Animation delay is zeroed because staggered entrances use `both` fill, so a leftover delay would hold rows invisible.
2. `foundations.css` `:root[data-motion="reduced"]`: `animation: none` and `transition: none` on everything. The app's own setting is `data-motion`.
3. `src/main.tsx:113` reads `document.documentElement.dataset.motion === 'reduced'` or `prefers-reduced-motion`, so the in-app setting and the OS setting agree.

Decoration is gated too: `empty-art.css`, `threshold-art.css` and `foundations.css` suppress artwork under reduced motion, reduced transparency, increased contrast or forced colours. `motion.css` says every rule in it is decoration: with motion off the app looks the same, only still. Components with their own gates: `chat-motion.css`, `subagents.css`, `chat-frontier.css`, `chat-outline-rail.css`, `tracker-tasks.css`. Many sheets use `@media (prefers-reduced-motion: no-preference)` to add motion only when allowed.

Motion that does not end is allowed only in two forms (`scripts/check-css-tokens.mjs:191`, `:202-216`): ambient motion gated by `animation-play-state: var(--ambient-play, ...)` so it pauses, and live-work indicators (spinner, status pulse, the running subagent signal at `subagents.css:47`) that run only while the work is active. Decorative entrances are finite.

Design rule (`DESIGN_SYSTEM.md`): shared buttons use fill feedback, no press scaling. Overlays and menus enter without stagger or bounce. See [Components, Buttons](#buttons).

### 8. Breakpoints and layout queries

Named container: `container: hollow-window / inline-size` on `:root`. Use it for layout rules.

| Name | Width | Em | Px at default |
|---|---|---|---|
| compact | ≤ 53.75em | `--window-compact-em: 53.75` | 860px |
| medium | ≤ 68.75em | `--window-medium-em: 68.75` | 1100px |
| wide | > 68.75em | | |

```css
@container hollow-window (max-width: 53.75em) { body { --page-gutter: var(--sp-4); } }
```

- `em` in a container query follows `--text-scale`. Media-query `em` uses the browser default, so do not use `@media` for these breakpoints.
- The `--window-*-em` values are read by `windowLayout.ts` for navigation and pane policy. Custom properties cannot be used inside a query, so keep the numbers in step with that file.
- Local unnamed pane queries are independent of the window container.
- Sub-720px media rules carry a narrow-window host comment at each block.

### 9. CSS conventions

- **Convention:** write no raw hex, px font sizes or hand-built shadows in component sheets.
- **What `scripts/check-css-tokens.mjs` enforces:** raw hex, px font sizes and `!important` outside the `a11y` layer. It excludes `tokens.css` and `styles.css` (`:289`, `:291`). It does not reject shadows: `foundations.css:607` builds one inline. Reviewers catch the rest.
- Colour and theme tokens use `color-mix(in srgb, …)` to derive tints from `--ink`, `--primary` or `--canvas`. Custom palettes re-derive them.
- Fallbacks such as `var(--glass-well, var(--surface-card))` are for optional roles only. Do not use one to hide a missing token.
- New layout or type values go into `tokens.css` first, then get referenced by name.
- Token names are public. Custom palette derivation and frame exports keep existing names (`DESIGN_SYSTEM.md`).
- `--fs-art` is a cell size, never a text role.

### 10. Texture (dither) policy

Source: `DESIGN_SYSTEM.md`, "Identity and texture". The principle version is in [Principles, 6](#6-dithered-art-is-identity-or-evidence-never-data).
- Identity may use glyphs. Evidence may use static pattern fills. Work stays plain.
- Exact values, input contents, code glyphs, transcripts and user files are never dithered. Text and controls sit on solid plates above any material.
- Settings > General > Texture offers Full, Quiet and Off texture (`hollow.studio.texture`; missing or invalid defaults to Full). The root `data-texture-level` is presentation only.
- The foundation has no unused texture classes. Add an ornamental treatment only with a real consumer.
- Artwork is decorative, capped at 600×300, never takes pointer input, and stays out of reading content.

## Components

Rule for new features: reuse these primitives. `docs/DESIGN_SYSTEM.md` says components extend the existing primitives and never create a parallel palette or component for the same job.

### Foundations that every component uses

- Tokens: `src/styles/tokens.css`. Radii: `--r-xs` (chips, tags), `--r-sm` (buttons, inputs, rows), `--r-md` (menus, list containers), `--r-lg` (cards, panels, dialogs, composer). See [Foundations 6.1](#61-radii).
- Sizes: `--control-h` 32px (28px compact), `--row-h` 30px (26px compact; a minimum, since two-line rows set their own pitch), `--row-pad-y` 6px for two-line rows, `--page-gutter` 24px (16px under a narrow window), `--section-gap` 24px. See [Foundations 5](#5-spacing).
- Fills: `--fill-hover`, `--fill-selected`, `--fill-strong` for hover and selected states. Never new hex values (see [Foundations 2.4](#24-fills-fixed-colours-and-syntax)).
- Elevation: resting elevation is flat. Only menus and dialogs get shadows (`--elev-2`, `--elev-3`). See [Foundations 6.3](#63-elevation-three-levels).
- Layers: keep component CSS in `@layer components` (see [Foundations 1](#1-layer-order)).
- Motion: `--t-fast`, `--t-med`, `--ease`. Both reduced-motion preferences apply to every transition (see [Foundations 7.3](#73-reduced-motion)).

### Buttons

- Component: `Button` and `IconButton` in `src/components/ui.tsx` (exports at lines 19 and 39). CSS: `src/styles/ui.css`.
  - `.btn`: default (hairline border, solid material). Height `--control-h`.
  - `.btn.small`: 28px height.
  - `.btn.primary`: Crimson accent fill, for the one main action of a surface. Disabled uses `--fill-strong`.
  - `.btn.ghost`: no border or fill at rest. Secondary and toolbar actions.
  - `.btn.secondary`: a supported variant (`ui.tsx:17`), used by Retry and Reload in `RecoverableView.tsx:16`.
  - `.btn.danger`: error fill or ghost styling for actions with a destructive consequence. Used in `ConfirmDialog`, as the ghost Stop button in `ChatBody.tsx:1920` and as the install button in `AppBuildPanel.tsx:161`. Styling is separate from the rule that a destructive action needs a confirm.
  - `.btn-link`: text-only action inside a sentence or row.
  - `.icon-btn`: 28px box, hover `--fill-hover`, selected via `aria-pressed="true"` (fill `--fill-selected`). A held selected toggle keeps an inner 1px `--body-strong` border that is separate from the focus ring.
- Pick (N44): one `primary` per surface; `ghost` (or `secondary`) for the rest; `danger` for destructive consequences. A page header and its content are one surface. Done in DL1 (2026-11-15): the header's "New note" (Notes view) and the list's "New note" (All view) are never on screen together, so each stays primary; the Notes empty state's "Write a note" became ghost.
- Labels start with a verb (N44). "New X" creation labels ("New note", "New project", "New conversation") are exempt. Other noun phrases are not: "All projects" (`StudioHome.tsx:64`) is a view switch, not an action button.
- Press feedback is fill only, never `transform: scale`. Icon buttons keep their 28px box on press (top-bar icon buttons included).
- Focus: `--focus-outline` with `--focus-offset`. Focus is separate from selected: a held selected control keeps its inner border with or without focus.

### Inputs and selects

- Radius and height by component, not shared: buttons use `--r-sm` and `--control-h`; text fields use `--r-md` with padding and no fixed height (`foundations.css:553`, `:570`); `Select` uses `--r-md` and a literal 32px (`select.css:9`). The installed app looked uniform in Settings.
- Switches: `src/components/Switch.tsx` (`role="switch"`, `aria-checked`). Checkboxes stay native. Toggle buttons (`Tabs`, `IconButton`) use `aria-pressed`.
- Multi-choice: `MultiSelect` with a portalled popover; `Select` shows a search field at nine options (`Select.tsx:9`, `:15`).
- Dropdowns: `src/components/Select.tsx` with `src/styles/select.css`. Do not write new `<select>` styling.
- Segmented choice: `Segmented` in `ui.tsx` (`.ui-segmented`). Use for 2 to 4 mutually exclusive views.

### Menus, popovers and palettes

- `src/components/ContextMenu.tsx`: right-click menus.
- `src/components/ActionMenu.tsx` and `MoreButton.tsx`: the ⋯ menu beside a row or header. Item styles are `.studio-action-menu`, `.studio-action-menu-body`, `.studio-action-menu-sep`, `.studio-action-menu-kbd` in `ui.css` (the menu item class names were not all checked individually).
- Model and usage popovers: `ChatModelMenu.tsx` (`chat-model-menu.css`), `ChatUsageMenu.tsx` (`chat-usage-menu.css`).
- Command palette: `CommandPalette.tsx`. Shortcuts and the palette only speed up a visible click path; they never replace one ([Principles, 1](#1-every-feature-has-a-visible-click-path)).
- Menus enter together without stagger or scale bounce. Menus and dialogs use the opaque raised surface even in Glass.

### Modals and dialogs

- `src/components/ConfirmDialog.tsx`: all confirmations. Loaded lazily from `App.tsx`, so do not static-import it (see [Lazy loading](#lazy-loading-and-the-app-chunk-cap)).
- Form dialogs: `PrepPackDialog.tsx`, `AttachCanvasDialog.tsx`, `CanvasLookDialog.tsx`, `ConflictCopiesDialog.tsx`.
- Each dialog has a viewport-bounded outer scroll region and a bounded body, so long titles stay reachable at Largest text.
- Scrims keep their motion gate. Focus rules are in [Principles, Interaction](#interaction).

### Toasts and notices

- `src/components/SnackbarStack.tsx`: short confirmations and undo.
- `src/components/NotificationToastStack.tsx`: app notifications. Bell and centre: `NotificationBell.tsx`, `NotificationCenter.tsx`.
- `src/components/ViewToastRegion.tsx`: toasts scoped to a view.
- Notices use the same opaque raised surface as menus.

### Chips, badges and pills

- Shape (N40): labels are pills, every other chip is `--r-xs`.
- Chips: `ComposerAccessChip.tsx`, `HeldChips.tsx`, `ReplyFileChips.tsx`. Chip radius is `--r-xs`. Do not give chips a button fill unless they are clickable. Done in DL1 (2026-11-15) for any non-label chip that is not on `--r-xs`.
- Label pills: Tasks labels and project tags (`.issue-label`, `.issue-project`, `tracker-tasks.css:41`) are fully round (999px, `--r-pill`) with a hairline. That is the sanctioned label shape. The sheet writes a literal `999px`; it now names `--r-pill` (DL1).
- File chips use `src/fileChip.ts`: the file name and a small type icon, never the full path (path on hover and in copy). See [Principles, Interaction](#interaction).
- Status pills: `StatusPill` in `ui.tsx` (`.ui-pill`), with `tone` and `quiet` options.
- Count badges in tabs: `.ui-tab-count`.

### Disclosure and folds

- Use a native `<details>` or a fold button with `aria-expanded`. Decided (N42): one disclosure glyph, the chevron `Icon` at 12 to 15px (for example `chevronRight` in Library groups, `Library.tsx:244`). Done in DL1 (2026-11-15); before it, tool-call rows still use a text caret, `▸` closed and `▾` open (`.toolcall-caret` in `ToolCall.tsx:152`, `ExecutionReceipt.tsx:134`, `ToolLedger.tsx:231`); they move to the chevron. The `▶` and `▼` seen on Automations and Tasks groups in the installed app move too, wherever they are on main.
- Examples: Subagents disclosure (`subagents.css`), `.inline-error-toggle` and `.inline-error-details` in `ui.css` (the "Details" for raw errors), `ChatOutlineRail.tsx`.
- Keep fold reveals free of entrance delays; both reduced-motion preferences still apply (`docs/DESIGN_SYSTEM.md`, "Overlays and questions").

### Tabs and segmented views

- On main today: `Tabs` in `ui.tsx` (`.ui-tabs`, `.ui-tab-count`): switching views within one page (for example Library All / Files / Canvases). It takes `level="primary"` (default) or `level="secondary"` (`ui.tsx:70`, `ui.css:176-193`). Primary is the underline look (Home, Library); secondary drops the background and radius, and its selected tab is a filled pill. The buttons sit in a `<nav>` and use `aria-pressed`: they are not ARIA tablists, so Tab moves between them and Enter or Space selects (`docs/reference/styling.md:69`). `Segmented` takes the same `level` prop.
- Decided (N38): two sanctioned levels. Primary (page-level views) is the underline. Secondary (a view inside a view) is boxed: the selected tab sits in a box, the rest stay plain text. Done in DL1 (2026-11-15); before it, on main a secondary tab is a `--r-pill` button and the selected one a filled pill (`ui.css:178-192`); the box is `--r-sm` with a hairline, and the installed app showed three looks (underline, a boxed selected tab on Tasks "All", boxed pills on Library "Recent" and "Meetings"); every tab strip moves to one of the two levels.
- Side panel tabs are their own component: `SidePanelCard.tsx` and `PanelNewTab.tsx` (`side-card-*`, `panel-newtab-*`).

### Lists and rows

- `ListRow` in `ui.tsx`: `leading` or `icon`, `title`, `meta`, `trailing`. Renders a button when it has `onClick`. CSS: `.ui-row`, `.ui-row-main`, `.ui-row-title`, `.ui-row-meta`, `.ui-row-trailing`, `.ui-row-icon`. Hover is `--fill-hover`. Decided (N39): a selected row shows the rail's leading tick, everywhere, on top of `--fill-selected`. Done in DL1 (2026-11-15); before it, selection has three forms today. The generic `ListRow` is only `background: var(--fill-selected)` (`ui.tsx:154`, `ui.css:228`); rail rows have the leading tick (seen in the installed app); and the selected Automations card has a left and right bar, the "static trailing edge" that `docs/DESIGN_SYSTEM.md:23` describes. The Automations bars and the plain fill move to the leading tick. Wherever selection would otherwise vanish in forced colours, give it an explicit forced-colours treatment. No rendered forced-colours proof exists.
- Sidebar rows: `src/components/Rail.tsx` (`.work-rail`, `.work-rail-primary`, `.work-rail-scroll`, `.work-nav-row`). Rail widths (N46): `--rail-w` 240px default (draggable 200 to 420px, remembered), `--rail-w-collapsed` 60px. Workspace headers and nested guides give hierarchy; selection is per row, with the leading tick (N39).
- **Hover-only controls** stay in layout and fade with `opacity`. Never `display: none` or hidden-until-hover, because specs use `toBeVisible` on those controls (see [Principles, Interaction](#interaction)).
- Ruled lists: flat dividers, no card per row (Tasks, Automations, Review use this).
- Section wrapper: `Section` in `ui.tsx` (`.ui-section`, `.ui-section-head`, `.ui-section-count`, `.ui-section-actions`).

### Empty, loading and sync states

- `EmptyState` in `ui.tsx` (`.ui-empty`, `.ui-empty-icon`, `.ui-empty-title`, `.ui-empty-body`, `.ui-empty-actions`). Optional `art` renders `EmptyArt.tsx`.
- `StatePanel` in `ui.tsx` (`.ui-state*`). `kind` is a `SurfaceState` (empty, loading, filtered, offline, partial, blocked, stale, failed, conflicted). Pass `error` for failures: it shows the plain sentence and a Details disclosure.
- Empty, filtered and loading panels have no frame except under Increase Contrast or forced colours. Offline, partial, blocked, stale, failed and conflicted panels keep their borders and recovery actions.
- Loading: `Skeleton` in `ui.tsx` (`.ui-skeleton`, `.ui-skeleton-bar`) and `Spinner.tsx`. Sync or refresh: `RefreshNotice.tsx` (`.refresh-notice`) and `SourceRefresh.tsx`.
- Use `StatePanel`, not a new empty block. Wording rules are in [Principles, Wording](#wording).

### Side panel

- Owner: `PreviewPane.tsx` (`src/styles/preview-pane.css`), a mixed strip of document tabs and web tabs. The 2026-11-10 decision named three tabs (Files, Browser, Subagents; `outputs/coo-leads-subagents-2026-11-10/design.md`, section 5) and removed Overview, Brief, Plan, Results, Sources and History. That is history: the panel on main has more parts than those three.
- Parts: the tab strip with a `+` that opens the new-tab Tools page (`PanelNewTab.tsx:14`); a summary card (`SidePanelCard.tsx:16`) shown when no tab is selected; a Changes tab for build workspaces (`sidePanelModel.ts:77`, `:83`).
- Files: a file tab shows the file, with `PreviewVisual.tsx` and `PdfPreview.tsx` for media. Notes (WORKSPACE.md, PLAN.md, SOURCES.md, AGENTS.md) open here as files.
- Browser: `BrowserPane.tsx` (`browser-pane.css`), with `BrowserAnnotate.tsx` and `BrowserReviews.tsx`.
- Subagents: `SubagentsPanel.tsx` (`subagents.css`, classes `subagents-row`, `subagents-group`, `subagents-composer`). Rows show state, elapsed time and tokens; the full session is readable here. Running and queued rows show a current step, trail and plan line; finished rows collapse to one line (`subagents.css:28`).
- Width: a remembered `--preview-pane-w`, default `clamp(340px, 42%, 720px)`, capped at the window minus 360px (`preview-pane.css:14`). `--aside-w` (320px) belongs to conversation history, not this panel. At 68.75em or narrower the page hides and the panel fills the slot (`preview-pane.css:24`, `:70-73`).
- Chat in the remaining width: the conversation keeps filling its pane, so opening the panel narrows it (the installed app reflowed chat to about 470px and the composer dropped its model name). Only the top bar's placement is fixed ([Principles, 4](#4-full-width-chat)).
- A section that shows a canvas uses the canvas view; no extra header repeats the title.
- Shell slot: the page and at most one preview sit in `.canvas-split` inside `main.canvas`, under the top bar. The top bar does not move when a preview opens.

### Composer and chat message layout

- Chat body: `src/components/ChatBody.tsx`, `ChatMessage.tsx`, `ToolCall.tsx`, `TurnFooter.tsx`. CSS: `chat-workspace.css` (`.chat-main`, `.chat-scroll`, `.turn.assistant`, `.chat-user-chip`).
- Chat scroll: `src/components/chat/chatScrollController.ts`. Chat motion: `chat-motion.css`.
- Composer: `.chat-input` in `chat-workspace.css` (padding `--sp-3` / `--sp-6`). Slash popup and composer commands: `ComposerCommands.tsx` and `composer-commands.css`. Composer files and drops: `src/composerFiles.ts`, `src/composerDrop.ts`.
- Chat width: the assistant turn is `max-width: 100%` in the chat column; the chat scroll has no max width of its own. Use `--sp-5` and `--sp-6` for gutters ([Principles, 4](#4-full-width-chat)).
- Question cards and their dock: `QuestionCard.tsx`, `QuestionDock.tsx` (`question-card.css`). One flat reading plane with dividers below fields.
- Subagents in chat: `SubagentsInChat.tsx`.

### Settings section layout

- Page: `src/pages/Settings.tsx` (`.settings-page`). Sheet and sections: `settings-sheet.css`, `settings-graph.css`.
- Structure: one screen per section (decision 2026-11-08). Each section is a `Section` (`.ui-section`) with ruled rows. `Switch` for booleans, `Select` for choices, `Segmented` for 2 to 4 options.
- Names (N45): say the app's section names, from `src/settingsSections.ts`. The twelve sections are General, Chat, Models, Accounts, Personalization, Connectors, Sources, Background work, Devices, Usage, Keyboard shortcuts and Advanced. Keyboard shortcuts is its own section, not part of General. Three former sections are parts inside one: Self-hosted models (in Models), Hollow folder (in Sources) and Search & indexes (in Advanced). Appearance, Texture, Motion, Density and Text size are controls in General (there is no "Interface" or "System" section any more).
- Hollow folder and Connectors styling: `vault-settings.css`, `connectors-library.css`.

### Pages

- Top-level destinations are the rail's visible labels: **Projects** (with Home and Running views), **Tasks**, **Library** and **Automations**, plus Settings (`src/actionDefs.ts:216-219`). Code ids differ (`work`, `vault`, `workflows`); say the visible label in docs and UI.
- Pages load lazily from `src/App.tsx` through `track(...)` (Review, Tasks, Vault, Workflows, Settings), except `Work`, which is a static import and renders immediately (`App.tsx:14`, `:50-66`). New pages should be lazy. Tonight, Weekly review and the workspace map were cut as screens; "the unattended window" is a concept, not a page.
- Page structure: `PageHeader.tsx` (`.page-header`, `.page-title-block`, `.page-title`, `.page-meta`, `.page-actions`), then `.page-body` (scrolls, padding `--sp-4` top and `--page-gutter` sides). The COO page has no header band (since 2026-11-11).
- Tasks: `src/pages/Tasks.tsx`, with `pages/tasks/*` (IssueList, IssueDetail, ProjectList, ProjectDetail, ViewBar) and `tasks-workspace.css`.

### App shell layout

- `Rail.tsx` (left, 240px by default, draggable 200 to 420px, collapses to 60px; N46), `TopBar.tsx` (44px context bar with native drag regions and traffic-light clearance), then `main.canvas` holding the page and the optional `PreviewPane`.
- The conversation fills the centre of a workspace. Shell CSS: `src/styles/shell.css`, `shell-workspace.css`.
- Compact windows: the open Glass rail uses the opaque soft surface.

### Lazy loading and the App chunk cap

- `scripts/check-bundle-budget.mjs`: `app` chunk (`App-*.js`) limit 340 KiB (line 39); `css` limit 344 KiB **per CSS asset** (line 22, `:89`); `mission-control` 26 KiB; `editor` 600 KiB. Also a 800 KiB launch set, 1 MiB per image and 16 KiB for the rail mark (`:56`, `:59`, `:66`).
- Anything not on screen at launch goes behind a dynamic `import()` or `lazy()` boundary, not a static import in `App.tsx`. `OnceOpened` only delays mounting; a statically imported child stays in the launch chunk (`OnceOpened.tsx:4`, `:11`, `App.tsx:80`). Wrap a lazy child in `OnceOpened` when it should stay mounted after the first open. Heavy UI (dialogs, palette, add sheet, export, editors, Settings, pages) already lazy-loads this way.
- The budget script fails the run if a budget names a chunk that no longer exists, so rename or drop the budget in the same change that moves a chunk.
- The 2026-11-07 note recorded the shell chunk near its limit (about 0.3 KiB of room). Treat any new eager code as a regression risk. Measure the current size before adding any ([Still open](#still-open)).

### More components and patterns

Added after the two reviews. Source-side items are cited to main; items marked "installed app" were seen in `d3bb1f9` and only those whose component exists on main are listed.

- **Errors and recovery:** `RecoverableView` (`RecoverableView.tsx:19`, a page-level retry with a secondary Retry or Reload button), `ErrorText`, `PlainErrorText` and `ErrorDetails` (`ui.tsx:228`, `:244`, `:253`). If the lazy `ConfirmDialog` fails to load, the fallback is cancel-only (`docs/reference/keyboard-shortcuts.md:39`).
- **Empty art needs evidence:** `src/emptyScene.ts:4`, `:9` refuses art for unread, failed or partial coverage; a filtered empty keeps its icon, not art.
- **Launch and art budgets:** see [Lazy loading](#lazy-loading-and-the-app-chunk-cap).
- **Actions and routes:** a new action registers in `ACTION_DEFS` and `ACTION_RUN` (`src/actionDefs.ts`); the generated `ROUTE_LEDGER.md` and `ACTIONS.md` are regenerated in the same change (`routeLedger.test.ts:109`).
- **Composer and permissions:** each chat's permission menu shows its agent's own modes (`docs/PER_AGENT_PERMISSIONS.md`). The "Suggested" strip under the composer lists its keys as `kbd` chips (`ChatBody.tsx:1842`). Turn footers are `TurnFooter.tsx`. User messages are the left-aligned `.chat-user-chip` (`ChatBody.tsx:1402`); assistant turns are bare text.
- **Subagent rows in chat:** active rows show step, trail and plan; done rows collapse to one line (`subagents.css:28`).
- **Canvases:** a canvas root defaults to whole-canvas dither. A legacy root `plain` still gets whole-canvas dither; a local graphic `plain` is an opt-out; ASCII is supported; the host and frame accessibility settings can veto it (`docs/DESIGN_SYSTEM.md:170`).
- **Settings chrome:** a "Where things went" link (`src/settings/whereThingsWent.ts`) and the palette pill beside General.
- **Chat outline rail:** `ChatOutlineRail.tsx`, a column of tick marks that jump to turns.
- **Metadata separator:** a middle dot (`·`) separates metadata everywhere in the installed app.
- **Master-detail (Automations)** and the **top bar** (breadcrumb, status text, bell, avatar, list and panel toggles) were seen in the installed app and not traced to a main component here.
- Not found on main by name: the steer line ("Enter steers Claude at its next step") and its "Steer now" button.

### Where to look first

- Visual values: `src/styles/tokens.css`. Primitives: `src/components/ui.tsx` and `src/styles/ui.css`.
- Design rules in prose: `docs/DESIGN_SYSTEM.md` (the "Shared states and presses" and "Shell and navigation" sections match this part).
- No single component catalogue exists in the repo. This part is a survey, not a generated inventory. Class names were read from `ui.css`, `shell.css`, `side-panel.css`, `subagents.css`, `chat-workspace.css` and component files on main ([Still open](#still-open)).

## Principles

Source: `docs/DESIGN_SYSTEM.md`, `docs/reference/keyboard-shortcuts.md`, `docs/reference/styling.md`, the agent memory index, and the workspace decisions cited in each item. Principles 5 and 6 restate the trusted-recall rules for canvases and texture.

### 1. Every feature has a visible click path

A shortcut, the command palette (⌘K) or a release note may speed up an existing route. None may be the only way to find a feature. Each retained capability has a visible place (Home, a page, a menu, a button); register a palette action or shortcut as well where one applies, but neither is required (`paletteCoverage.test.ts:15-31` allows `elsewhere`). `src/routeLedger.ts` and `src/routeLedger.test.ts:32` fail on a new palette action, `g` target or shortcut with no click-only route; they do not require every route to have an accelerator. Do not add a redundant surface to satisfy a palette entry. Source: `docs/reference/keyboard-shortcuts.md` ("Keyboard shortcuts" section), CLAUDE.md "Reference docs" table.

### 2. Consolidate, don't duplicate

Overlapping features get one primary home. Renaming a button or linking an old surface does not count as consolidation. Each thing is shown once: Home defers to Review and Automations for their lists (`docs/DESIGN_SYSTEM.md`, "Home"), and Settings rows have one home each (`outputs/settings-redesign-2026-11-08/plan.md` rules 6 and 7). Checked in [New-feature checklist, 2](#new-feature-checklist).

### 3. Calm, flat, uncluttered

The quiet-workbench direction: neutral surfaces, Crimson accent only for the primary action, focus and selection. Dense lists and fine dividers replace floating-card decoration. Resting elevation is transparent; shadows appear only on menus and dialogs (`docs/DESIGN_SYSTEM.md`, "Shell and navigation"). Decorative art (the threshold, orbital fields) is bounded, never takes pointer input and never sits in reading content (`docs/DESIGN_SYSTEM.md`, "Identity and texture").

### 4. Full-width chat

The conversation fills the width of the pane it has. When the preview panel opens, chat is narrowed to what is left (the composer then drops its model name), and the panel can take the whole slot in a narrow window. The top bar does not move when a preview opens (`docs/DESIGN_SYSTEM.md`, "Shell and navigation"; memory note "Chat fills its pane"; panel width in [Components, Side panel](#side-panel)). Decided (N43): chat keeps ignoring `--measure`. Layout values: [Components, Composer](#composer-and-chat-message-layout).

### 5. Markdown is the record; canvases present it

WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md stay the agent's context and source of truth. HTML canvases are the person's view and never write back into those files. Canvases are saved under `outputs/canvases/` with up to 30 revisions (`[learnings/markdown-stays-source-of-truth-beneath-html-canvas]`, `[learnings/html-canvas-persistence-frequency-and-source-of-truth]`). Canvas dithering is in item 6.

### 6. Dithered art is identity or evidence, never data

Glyphs and static pattern fills may mark identity or evidence. Exact values, input contents, code, transcripts and user files are never dithered (`docs/DESIGN_SYSTEM.md`, "Identity and texture"). Hollow canvases are dithered as a whole (`texture="dither"`, not `plain`), with the kit owning text and data (WORKSPACE.md decision, 2026-10-30, per the shared dithered-design rule). Texture levels: [Foundations 10](#10-texture-dither-policy).

### 7. Consolidation check for new work

Before adding a surface, check whether an existing page, panel or dialog already owns the job. Components extend the shared primitives and never create a parallel palette or component for the same job (`docs/DESIGN_SYSTEM.md`, "Shell and navigation"). Checklist form: [New-feature checklist, 2](#new-feature-checklist).

### Wording

**Plain words, one meaning per word.** `scripts/check-plain-words.mjs` fails a build when shown text uses a banned engineering word. The table in `WORDS` is also the glossary's "Words we use" table (`docs/reference/glossary.md`).

| Say | Not |
|---|---|
| run record | receipt |
| output, error output | stdout, stderr |
| the conversation | session id, session key |
| request to Hollow | IPC, backend call |
| automation | workflow |
| Import | ingest |
| History & recovery | Vault History |
| Running | Activity (Work's view) |
| Library | vault (for where notes live) |

Agent-only sentences go in the script's `ALLOWED` list with a reason. The UI names are COO, leads and subagents, and agents address the COO by that name too (`outputs/coo-leads-subagents-2026-11-10/design.md`, section 1). "Results" is the name for a saved, accepted output of a run in the workspace (WORKSPACE.md, Overview and Results).

**Case and punctuation**
- Sentence case for labels, headings, menu items and buttons. Buttons start with a verb ("Create workspace", "Run", "Undo"); "New X" creation labels ("New note", "New project") are exempt (N44).
- No em dashes and no en dashes as punctuation, in anything. Use commas, colons, parentheses or full stops (operator guardrail, `me/Communication.md`). The `…` ellipsis marks a menu item that opens another step ("Archive older entries…").
- Short labels. Help lines are at most 90 characters. Longer help moves behind the row's own More, and only where the row is risky (`outputs/settings-redesign-2026-11-08/plan.md`, rule 4).

**Times and durations** (N36). One format everywhere:
- Past, relative: `16h ago` (no space between number and unit: `5m ago`, `16h ago`, `3d ago`).
- Future, relative: `in 23h`.
- Durations: `2m 55s`, `1h 4m` (never `175s`, never `4 m 12 s`).
- A clock time today: `9:40 AM`.
- Before today: `Fri 11-13 9:40 AM` (weekday, month-day, then the time).
- Older than 7 days: an absolute date, no relative form.

Done in DL1 (2026-11-15); before it, there is no single formatter yet, and the current ones disagree. `src/executionReceipt.ts:521` (`formatPhaseDuration`) writes `12ms`, `2.3s`, `2m 5s`, `1h 4m`; `src/components/RecentActivity.tsx:281-295` writes `just now`, `Nm ago`, `Nh ago`, `Nd ago`, then a short date; `src/aboutYou.ts:58` writes `5 min ago`, `3 h ago`, `2 days ago`; `src/homeReturn.ts:10` writes `12 min ago`, `3 h ago`, `yesterday`, `4 days ago`. The installed app also showed `175s` (Automations) and `1:09 AM Fri 11-13` (Home). New code uses the rule above, not the owning surface's old format.

**Errors and empty states.** Say what happened, then what to do, in one or two sentences, with a visible recovery action. Offline, partial, blocked, stale, failed and conflicted panels keep their tones and recovery actions. Empty states share one title and spacing. A filtered or loading state keeps its words even when its frame is hidden (`docs/DESIGN_SYSTEM.md`, "Shared states and presses"). Don't blame the person. Don't use "unable" or "invalid" where a cause can be named.

**Names used in the UI**
- **COO**: the main agent. Available for prompts and questions; delegates real work.
- **Lead**: one chat per project. It keeps the project's context and delegates to subagents. Say "the project's lead", not "the project agent".
- **Subagents**: Hollow sessions started by the COO or a lead, hidden from the sidebar, shown in the Subagents tab and the chat's Subagents chip.
- **Results**: the accepted output of a run, kept under the workspace.
- **Automations** for scheduled prompts. "The unattended window" is a term for overnight work, not a screen: Tonight was cut as a page.

### Interaction

- **Approval and confirm cards.** Decided (N47): the code is the authority, `asks_in_mode` (`src-tauri/src/app_actions.rs:661`). In Bypass mode Hollow shows no card except three kinds, all checked before the bypass branch. A fixed list, `ALWAYS_ASK` (`app_actions.rs:597`): `gardener.apply`, `records.records_supersede_decision`, `records.records_set_decision_status`, `friction.draft_fix`, `build.install_built_app` and `settings.cfo_approve_extra_usage` (`tracker.archive_task` left it on 2026-11-15). An argument-dependent kind: destructive or access-granting `tracker.graphql` mutations always ask (`app_actions.rs:667`, `src-tauri/src/tracker/graphql.rs:709`). And the apply of `workspaces.archive_shared_docs` (Archive older entries; `app_actions.rs:677`). The run's `<hollow-mode>` note (`src-tauri/src/mode_contract.rs:250`) names the fixed list and the archive apply but not the GraphQL class; it now names the GraphQL class (done in DL1, 2026-11-15). Auto does not keep every card: it asks selectively (`app_actions.rs:684`). Other modes keep their cards. Every card names the action and its effect in plain words. Cancel is the default focus on an action that bills or deletes (`ConfirmDialog`, `focusCancel`, `docs/reference/keyboard-shortcuts.md`, "Dialog focus").
- **Cards need the person present.** Agent approval cards that go through the computer channel time out after about two minutes (`src-tauri/src/computer_mcp.rs:39`, 120 s), so send them while the person is at the Mac (`[memory: cards-need-user-present]`). Ordinary UI confirmation dialogs (`showConfirm`, `src/store/overlays.ts:121`, `ConfirmDialog.tsx:45`) have no timer.
- **Keyboard shortcuts.** Each command shortcut is a default that Settings > Keyboard shortcuts (its own section, not under General) can change or remove. The **Use keyboard shortcuts** switch turns off app and global commands: no command key, menu accelerator or single-letter list key works. Local in-surface commands (Find, selection actions, the browser toolbar's Reload) stay, owned by `LOCAL_COMMANDS` and the focused surface (`docs/reference/keyboard-shortcuts.md:181`, `:185-194`; `src/localKeys.test.ts`). Arrow keys, Enter, Escape closing things, the chat's send keys, `/`, and editor keys (⌘S, ⌘Z) always work (`docs/reference/keyboard-shortcuts.md`).
- **Shortcuts never take a macOS system command.** The New workspace form uses ⌘↵ and Esc only inside the form, with visible buttons. Check a new chord against macOS first, and keep it out of the global namespace unless it is a deliberate system-wide key.
- **Shortcut card.** A new shortcut is listed in the `?` card and in Settings > Keyboard shortcuts > Show all keys, in the same change (`docs/reference/keyboard-shortcuts.md`; `src/components/ShortcutsOverlay.tsx`).
- **Focus.** Dialogs trap Tab in a fixed order, start on Cancel for billed or destructive actions, and return focus to whatever had it on close (`src/focusHold.ts`, `src/focusReturn.ts`). Nested overlays close one layer per Esc. The palette is a combobox whose `aria-activedescendant` names the selected option. An Undo snackbar holds its countdown while it has focus or the pointer.
- **Hover and right-click.** A row's click opens it. The right-click (or the row's ⋯ button, where shown) opens its menu. Sidebar project rows: a click opens and folds, a right-click is the menu (PLAN.md, 11-07). Hover reveals secondary controls by opacity only, so they keep their names and stay reachable by keyboard (`[memory: hover-controls-keep-a-box]`).
- **Side panel.** The 2026-11-10 decision was three tabs (Files, Browser, Subagents; `outputs/coo-leads-subagents-2026-11-10/design.md`, section 5); the shipped panel is a mixed tab strip with extras, described in [Components, Side panel](#side-panel). A file link opens in the side panel and shows the file's name with a type icon, the path on hover and in copy (WORKSPACE.md decision, 11-10).

### Accessibility

- **Contrast.** Three separate checks, not one. (1) Stock tokens: `scripts/check-contrast.mjs` checks a fixed list of text tokens against a fixed list of surfaces (`:9`, `:24`, `:28`) at **4.5:1** in the light and dark themes; `--on-primary` and `--on-error` are checked on their fills; `--report` lists every pair; a new text role is not covered until it is added to the lists. (2) Custom palettes: `src/settings/themeContrast.ts:15` is advisory, at the same floor, and the script does not include them. (3) Liquid Glass: not covered by the script; verify it on a rendered capture. The script checks text only, not non-text contrast such as focus rings and borders (see [Still open](#still-open)).
- **Largest text.** The Largest setting is 125%. Confirm dialogs have a viewport-bounded outer scroll so long titles and buttons stay reachable. The minimum window, 720 by 480, is checked at Largest (`docs/DESIGN_SYSTEM.md`, "Overlays and questions"; `docs/reference/styling.md`).
- **Reduced motion.** Two preferences govern motion: the OS `prefers-reduced-motion` and Hollow's own `data-motion="reduced"`. Every transition, loading animation and entrance honours either one (`docs/reference/styling.md`, "Interaction and motion"). Reduced motion removes entrance animation and stagger, not state change: a selection still shows. Token mechanics: [Foundations 7.3](#73-reduced-motion).
- **Increase Contrast and Reduce Transparency.** Glass tints become opaque surfaces. Increase Contrast uses opaque ink hairlines and solid focus rings. Forced-colours mode gives empty, filtered and loading states a `CanvasText` frame; it does not replace root tokens (`foundations.css:664`, `:675`; `docs/DESIGN_SYSTEM.md`, "Shared states"). Selected rows have no generic forced-colours rule: a surface whose selection would vanish needs its own, and the leading tick (N39) must survive forced colours ([Components, Lists and rows](#lists-and-rows)). Nothing here has been checked on a render.
- **Focus visibility.** The shell's 2px keyboard outline wins over component focus shadows. Focus, hover, disabled and selected stay distinct. A selected row keeps its state with no focus ring.
- **VoiceOver names.** Every control has a name a person can hear without seeing it: icon-only buttons carry `aria-label` with the same words as their tooltip, toggle buttons expose `aria-pressed`, switches expose `aria-checked` (`role="switch"`), checkboxes stay native, each group keeps its name, and decorative art is `aria-hidden` and not focusable (`docs/reference/styling.md`; `docs/DESIGN_SYSTEM.md`, "Identity and texture"). At compact widths an icon button keeps its full accessible name and tooltip and hides only its words.
- **Keyboard-only.** Every feature works with the keyboard alone. `tests/smoke/keyboard-only.spec.ts` covers the nested-overlay rule. `docs/verification-2026-10-13/evidence/keyboard-a11y.log` is the earlier audit record.
- **Test-only checks.** `src/a11yPass.test.ts` pins palette focus return and related rules. The `paletteCoverage` and `routeLedger` tests keep the click-path rule.

## New-feature checklist

Decided (N48): this list holds only what the repo checks. Every item names the script or test that fails when it is broken; `npm run gate` runs all of the scripts and unit tests (`gate:frontend` in `package.json`), and the Playwright specs run in the smoke suite. Run the gate on the rebased head before a UI change merges.

1. **Click path.** A new palette action, `g` target or shortcut has a click-only route, and the generated route docs are current. Checked by `src/routeLedger.test.ts` and `src/paletteCoverage.test.ts` ([Principles, 1](#1-every-feature-has-a-visible-click-path)).
2. **Plain words.** No banned engineering word in shown text. Checked by `scripts/check-plain-words.mjs` (`npm run test:scripts`) ([Principles, Wording](#wording)).
3. **Tokens and layers.** No raw hex, px font size or `!important` outside the `a11y` layer; no `var()` that nothing defines; component rules inside `@layer components`; no animation naming a missing `@keyframes`; no stray declaration. Checked by `scripts/check-css-tokens.mjs` (`npm run css:check`, `npm run build`; layer cases in `scripts/check-css-layers.test.mjs`) ([CSS conventions](#9-css-conventions)).
4. **Endless motion pauses.** An infinite ambient animation opts into `--ambient-play`. Checked by `scripts/check-css-tokens.mjs` ([Foundations 7.3](#73-reduced-motion)).
5. **Reduced motion, sampled.** System-only motion reduction is honoured by the Library outline and Settings search. Checked by `tests/smoke/motion-preferences.spec.ts`; other surfaces are an intent below ([Foundations 7.3](#73-reduced-motion)).
6. **Text contrast.** Every text token in the script's lists meets 4.5:1 on its surfaces in light and dark. A new text role must be added to those lists, or it is not checked. Checked by `scripts/check-contrast.mjs` (`npm run css:check`, `npm run build`) ([Principles, Accessibility](#accessibility)).
7. **No orphan CSS.** Every class a stylesheet styles is used by the app. Checked by `scripts/check-orphan-css.mjs` (`npm run test:scripts`).
8. **Window breakpoints.** Window-width rules use the named breakpoints only, and `--window-*-em` matches them. Checked by `scripts/check-window-breakpoints.mjs` (its test in `npm run test:scripts`) ([Breakpoints](#8-breakpoints-and-layout-queries)).
9. **Bundle budgets.** The App chunk stays under 340 KiB, each CSS asset under 344 KiB, the launch set under 800 KiB. Checked by `scripts/check-bundle-budget.mjs` (`npm run bundle:budget`) ([Lazy loading](#lazy-loading-and-the-app-chunk-cap)).
10. **Shortcuts.** A new chord is in the shortcut table and switches off with Use keyboard shortcuts; local in-surface keys stay. Checked by `src/shortcuts.test.ts`, `src/settings/shortcutTabs.test.ts`, `src/localKeys.test.ts` ([Principles, Interaction](#interaction)).
11. **Focus and dialogs.** Palette focus return and the related focus rules. Checked by `src/a11yPass.test.ts`; nested overlays close one layer per Esc in `tests/smoke/keyboard-only.spec.ts` ([Principles, Interaction](#interaction)).
12. **Minimum window, Largest text, axe AA.** The 720 by 480 minimum window cases, an axe AA pass and a keyboard-only walk in light and dark. Checked by `tests/smoke/a11y-sweep.minimum.spec.ts`, `tests/smoke/focus-rings.minimum.spec.ts` and the other `*.minimum.spec.ts`; `scripts/check-required-specs.mjs` refuses a skip in them ([Principles, Accessibility](#accessibility)).
13. **Settings names.** A Settings control's search label is text the Settings sources really show. Checked by `src/settingsSections.test.ts` ([Settings section layout](#settings-section-layout)).

**Intents.** No script checks these; a reviewer does:

- One home: a new surface extends the page, panel or dialog that owns the job ([Principles, 2 and 7](#2-consolidate-dont-duplicate)).
- Markdown stays the record: canvases present it and never write back ([Principles, 5](#5-markdown-is-the-record-canvases-present-it)).
- The decided rules in [Decisions](#decisions): times (N36), one primary per surface (N44), tab levels (N38), the leading tick (N39), chip shapes (N40), the chevron (N42).
- Sentence case and short labels; no em or en dashes.
- A new chord does not take a macOS system command.
- Palette and Glass contrast, and non-text contrast (focus rings, borders), on a rendered capture.
- Both reduced-motion preferences on every new transition, with inspected captures at Largest text and in the reduced-motion state.

Verification for a UI PR: the full check (`npm run gate`, see [`AGENTS.md`](../AGENTS.md#gate-and-install)) plus inspected captures. Do not run the browser suite per change; run it once at the end of a long run.

## Decisions

Alex's decisions on the draft, 2026-11-14 10:47 PM ("use the design rules as written"). Each is a rule. "DL1" marks where main does not follow it yet; row DL1 is the app fix.

| # | Rule | Where it applies |
|---|---|---|
| N36 | Times read `16h ago`, `in 23h`, `2m 55s`; `9:40 AM` today, `Fri 11-13 9:40 AM` before today, an absolute date after 7 days. One formatter (done in DL1). | Every shown time and duration ([Wording](#wording)) |
| N37 | The colour tables here are the stock default; a palette may change only what [Palettes](#palettes) lists. Re-measure on the default palette (still to do). | [Colour](#2-colour) |
| N38 | Two tab levels: underline for primary, boxed for secondary (done in DL1). | `Tabs`, `Segmented` ([Tabs](#tabs-and-segmented-views)) |
| N39 | A selected row shows the rail's leading tick, everywhere (done in DL1). | Rail, `ListRow`, Automations ([Lists and rows](#lists-and-rows)) |
| N40 | Labels are pills (`--r-pill`); every other chip is `--r-xs` (done in DL1). | [Chips, badges and pills](#chips-badges-and-pills), [Radii](#61-radii) |
| N41 | The ambient page wash is sanctioned; texture Off removes it (done in DL1). | `.ambient-layer` ([Borders](#62-borders)) |
| N42 | One disclosure glyph, the chevron `Icon`; tool-call carets move to it (done in DL1). | [Disclosure and folds](#disclosure-and-folds) |
| N43 | Chat keeps ignoring `--measure`: full width, the chat-fills-pane rule. | [Full-width chat](#4-full-width-chat), [Line heights](#43-line-heights-tracking-measure) |
| N44 | One primary button per surface (done in DL1 for Library notes); "New X" labels are exempt from verb-first. | [Buttons](#buttons), [Wording](#wording) |
| N45 | Settings names are the app's: General; Keyboard shortcuts is its own section. | [Settings section layout](#settings-section-layout) |
| N46 | Rail width: default 240, draggable 200 to 420; the 260 measured was a drag. Main does this. | [Spacing](#5-spacing), [App shell](#app-shell-layout) |
| N47 | Confirm-card authority is the code (`asks_in_mode`); the mode note gains the GraphQL class (done in DL1). | [Interaction](#interaction) |
| N48 | The checklist holds only what the repo can verify, plus a short list of intents. | [New-feature checklist](#new-feature-checklist) |
| N49 | The doc pins to main, with a differences table against the installed build. | Top of this doc |

Base look: the Acme look from 2026-10-27 (Sam's) stays the base.

### Still open

1. **Non-text contrast (gap).** No script checks focus rings or borders, so the claim that they pass is unverified. A check is needed before this doc can say so ([Accessibility](#accessibility)).
2. **Chunk headroom (measure before adding).** The shell chunk was about 0.3 KiB under its 340 KiB cap on 2026-11-07. The value is stale; measure it before adding eager code ([Lazy loading](#lazy-loading-and-the-app-chunk-cap)).
3. **Component inventory (caveat).** No generated component catalogue exists. Class names were read from source, and a few menu class names were not checked individually ([Menus](#menus-popovers-and-palettes)).
4. **Palette re-measure (N37, to do).** The dark values measured in the installed app came from a user palette. Re-measure on the default palette with the rail reset, and on a build from main. Light, Glass light and Glass dark were not measured at all.
5. **"Not verified" shown green (app issue).** In the installed Automations run record "Not verified" was green mono, against the status rule in [Colour](#2-colour) (green means a confirmed pass). Seen only in the installed build; the cause was not found on main.

## Not yet verified

No review looked at these, so nothing above should be read as checked for them:

- Light theme, Liquid Glass light and Glass dark (Foundations 2.1, 2.5, 3).
- Menus, popovers, palettes, dialogs, toasts and notices, and elevation (Foundations 6.3).
- The Files and Subagents tabs, the Changes tab, the new-tab Tools page and the summary card; only one panel width (about 1146px) was seen.
- Motion and both reduced-motion gates; breakpoints and narrow windows (one window size only).
- Hover and focus states, keyboard focus rings, forced colours, increased contrast and non-text contrast.
- Text sizes other than Default, Comfortable density, and texture Quiet and Off.
- Canvases and dithered art, and Principles 1, 2 and 7 (click paths and consolidation).
- Settings sections after General (the window reported "closed" mid-capture).
- Build, tests, contrast measurement and bundle headroom were not run (for the draft or for this version).

## Sources

- Reviews: `outputs/design-language-2026-11-13/review-combined.md`, `review-source.md` and `review-running-app.md` in the Hollow workspace; installed build `d3bb1f9` (captures in that folder's `app-captures/`).
- This repo at `983e3d46f` (draft read at `a8f57a55a`, 2026-11-13): `src/styles/tokens.css`, `src/styles/foundations.css`, `src/styles.css`, `src/components/ui.tsx`, `src/styles/ui.css`, `src/settingsSections.ts`, `src/store/theme.ts`, `src/store/prefs.ts`, `src-tauri/src/app_actions.rs`, `src-tauri/src/mode_contract.rs`, `docs/DESIGN_SYSTEM.md`, `docs/reference/keyboard-shortcuts.md`, `docs/reference/styling.md`, `scripts/check-contrast.mjs`, `scripts/check-plain-words.mjs`, `scripts/check-css-tokens.mjs`, `scripts/check-bundle-budget.mjs`, `src/executionReceipt.ts`, `src/components/RecentActivity.tsx`, `src/aboutYou.ts`, `src/homeReturn.ts`.
- The Hollow workspace: `outputs/design-language-2026-11-13/part-1-foundations.md`, `part-2-components.md`, `part-3-principles.md`; WORKSPACE.md, "Decisions and constraints"; PLAN.md (11-07 sidebar rows; 11-12 composer and subagents entries).
- `outputs/coo-leads-subagents-2026-11-10/design.md` (sections 1, 3, 5); `outputs/settings-redesign-2026-11-08/plan.md` (section 3.1).
