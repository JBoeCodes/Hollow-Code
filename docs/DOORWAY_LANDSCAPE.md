# Doorway landscape

## Behavior

Populated Home (`StudioHome`) shows a 64px identity above its glance tiles and
return cards; Ask follows those cards. Empty Home retains the larger artwork
above Ask anything. `styles/home.css` owns the unlayered populated override.
Empty full-size `ChatBody` shows it above the existing heading and starter prompts,
including workspace and Chats. The artwork disappears with the empty state
when a message exists, never covers reading surfaces and is omitted in compact chats.
No navigation, send, draft or session behavior changes.

`DoorwayLandscape.tsx` and `styles/doorway-landscape.css` own both placements.
They lazy-load `ThresholdArt.tsx`, an original deterministic folded doorway in
glyphs. It is decorative (`aria-hidden`) and ignores pointer input. Reserved
responsive height avoids layout jumps; shorter windows cap it. Shared tokens
give it the actual theme color. Accessibility gates replace glyphs with a plain
doorway, including system colors under forced colors. It has no animation,
canvas, shader, timer, network fetch or new dependency. See
[Design system 3.0](DESIGN_SYSTEM.md#identity-and-texture).

## Artwork provenance

The current field is original code from the 2026-10-26 Hollow dither study,
implemented in `thresholdField.ts`; no third-party renderer or artwork is used.
The previous supplied landscape remains unchanged in source, with provenance
retained below. It is no longer imported by DoorwayLandscape.

- Source: the owner's first image attached to the Hollow dithering discussion,
  2026-10-07 (`paste-18916-1791413771495755000-0.png`), selected in the proposal
  accepted by “let's build it”.
- Master kept unchanged at `src/assets/brand/hollow-doorway-landscape.png` (not bundled since
  five asks B4, 2026-10-10). The former bundled `hollow-doorway-landscape.jpg` has the same 2172 × 724
  pixels re-encoded by `sips -s format jpeg -s formatOptions 90` (810,169 bytes, −75%). At 3×
  zoom the dither is indistinguishable from the master. AVIF/WebP were ruled out: the app
  supports macOS 11, and WebKit only decodes AVIF from macOS 13. `check-bundle-budget.mjs`
  caps every bundled image.
- Dimensions: 2172 × 724; bytes: 3,190,854.
- SHA-256: `faf7602ecfac866e8bc1907894289700b66a337e031d86740d0e694d095467dd`.
- Dithering is already baked into this supplied artwork. No runtime conversion or
  synthetic recreation is involved; retain this source attribution if replaced.

## Historical landscape verification (2026-10-07)

Typecheck, all 1,412 Vitest tests, production frontend build, CSS-token checks,
bundle budget and plan/version checks passed. Playwright browser-mock screenshots
cover Home and empty chat, light/dark, custom/glass CSS and a 390px-wide chat.
A local interaction check confirms a decoded image, decorative semantics, no
ongoing animation, no horizontal overflow at 390px, forced-color suppression and
artwork removal after sending a mock message. Browser glass checks are not native
macOS vibrancy verification. Full existing smoke: 638 passed / 160 skipped / 0 failed (9.5 minutes), on a private port.
Skipped checks retain their original suite conditions and are not counted as passes.

Not installed or verified in the native candidate bed as part of this change.
