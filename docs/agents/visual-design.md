# Visual design, styling and motion

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Full references: [Design system](../DESIGN_SYSTEM.md), [Styling](../reference/styling.md), [motion kit](../MOTION_KIT.md).

## Design system 3.0

- Neutral white/charcoal planes, flat resting surfaces and the 3/5/6/8px radius family; stock themes use the Acme-red accent. Risk is violet, distinct from amber warning; warning/error pills pair words with icons.
- Shared visual roles (material aliases, three elevation tiers, tabular numerals) live in the design system; extend those tokens and primitives. Overlays (dialogs, palette, context menus, notices) stay opaque in Glass. Tooltips stay native title hints.
- CSS rules live in cascade layers; `css:check` inspects every block. CodeMirror's injected theme is the scoped exception. Accessibility fallbacks live in `foundations.css`'s a11y layer and keep precedence. See [the cascade layer](../reference/styling.md#the-cascade-layer).
- Destination tabs use the primary underline; scopes and filters use secondary pills ([view control levels](../reference/styling.md#view-control-levels)). Controls keep their size on press (fill feedback, no scaling).

## Themes, texture and motion

- Interface offers Texture (Full / Quiet / Off) and Ambient motion, independent and both beneath the accessibility gates. Keep the old `experimentalVisuals` storage key: saved Off stays Off for motion; missing texture defaults to Full.
- The lazy controller in `ExperimentalVisuals.tsx` owns `experimentalActive` and the matching root attribute; every experiment uses that combined focus/motion/a11y policy. Only decorative pixels move.
- App-side motion readers share `motion.ts`: either app Reduced or system Reduce Motion suppresses motion. Boot content is opaque from its first frame.
- Text size has four choices shared by bootstrap, store and Settings. Custom text derives an AA contrast floor; theme files stay unchanged. Custom-theme reads reserve selection ownership before awaiting so older results are rejected. See [Appearance preferences](../reference/styling.md#appearance-preferences).
- The flow SVG golden pins the stock palette and is shared with the Rust export validator.

## Icons

- Icon names derive from the fresh-node factories in `icons.tsx`; shared meanings live in `iconMeaning.ts`.
- Custom icons ([Custom icons](../CUSTOM_ICONS.md)): read failures disable Save, conflicts keep the draft for an explicit replacement, and `ICON_DAMAGED` stays separate from ordinary I/O. Icon data stays outside the vault and signed bundle; existing custom records are never migrated or replaced. The app icon drives `HollowMark`, the favicon and runtime menu-bar/window artwork; Fold is the stock identity.
- The doorway silhouette is Hollow's identity; see [BRAND_DOORWAY](../BRAND_DOORWAY.md).

## Empty views

`EmptyArt` draws static stipple under the shared texture policy, behind its own Suspense/error boundary, with reserved dimensions and no observer or animation. Optional artwork requires complete, unfiltered coverage; words, actions and the icon fallback always stay. See [Empty views](../EMPTY_STATES.md) and [Doorway landscape](../DOORWAY_LANDSCAPE.md).
