# Studio doorway

2026-09-27. The doorway silhouette is Hollow's identity. The user rejected the
h monogram and requested an evolution of the old logo that fits the Studio UI.
The transparent lilac frame, teal reflection and warm threshold replace the old
amber plastic finish and dark tile in the in-app rail and home. The macOS bundle
uses the same approved mark. Original native assets remain available in Git history.

Asset: `src/assets/brand/hollow-doorway-studio-v2.png`. Generated with the built-in
image-generation tool, copied unchanged with its alpha channel. It stays the master for the native icons below. The rail draws the mark at 18–22 px, so
`HollowMark.tsx` imports `hollow-mark-96.png`, a 96 × 96 copy made with `sips -Z 96`
(7.6 KB instead of 688 KB, and a 36 KB decoded bitmap instead of 6.3 MB; five asks B4); no generated asset is loaded from outside the repository.
Reference images: existing `src-tauri/icons/icon.png` and the local Studio browser
preview. This is a raster asset, not a claimed vector master.

## Generation prompt

Use case: style-transfer / logo-brand.
Asset type: Hollow desktop app's redesigned doorway logo, production transparent PNG.
Input image 1 is the existing logo to evolve. Input image 2 is the current studio interface for palette/style reference only; do not reproduce any UI.
Primary request: Improve the original glowing arched doorway so it belongs naturally in this refined warm-charcoal, luminous-lilac studio. Preserve the iconic tall rounded-top doorway and solid horizontal threshold, centered frontal symmetry. This is a doorway, never a letter h, n, or U.
Replace the bulky dark rounded-square app-icon backing entirely with transparency. Render just one clean substantial sculpted arch and threshold: satin lilac ceramic/glass material with a restrained pale lavender edge light (#c4acf4), a tiny pale teal inner reflection, softly lit warm charcoal inset opening, subtle depth and a restrained warm pearl light at the threshold. The interior reads as a passage into possibility. A refined continuous silhouette with a generously rounded semicircular top, vertical sides and a solid base. Premium, tactile, approachable; clean enough to read at 32 pixels. Simplify the old amber glow and heavy plastic bevels. No ornate layering.
Composition: square canvas, centered doorway occupying about 78% of canvas height, about 55% canvas width. Keep a small even safety margin. Transparent outside the doorway, actual alpha, no backdrop, no floor, no cast shadow outside silhouette.
Constraints: one finished logo only; no text, monograms, wordmark, letters, grid, alternatives, orbit lines, stars, sparkles, lens flares, perspective tilt, mockup, UI, border, or watermark. Preserve arched doorway identity while modernizing material and color.

## Native icon derivatives

Generate with `npm run tauri -- icon src/assets/brand/hollow-doorway-studio-v2.png
--output <scratch-directory>`, then copy only the five configured bundle assets:
`32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, and `icon.png` into
`src-tauri/icons/`. The CLI resizes/encodes the approved artwork; it does not
redesign it. The legacy `source.png` is not the current master. Release source
provenance includes `src-tauri/icons/` so an icon-only change requires a rebuild.

## Compact working-session mark (2026-10-01)

At the rail's 14px size, `RunningGlyph` in `Rail.tsx` uses a 1.4-unit
outline of the rounded arch and threshold, drawn directly in a 16-unit SVG.
It folds into the row's existing geometric provider mark and back; this is
a transform/opacity transition, not a path interpolation or official model logo.
The full Studio artwork remains the rail/home/bundle identity. System and
app reduced motion keep the complete line doorway still.
