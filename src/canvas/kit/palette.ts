/** The kit's chart palette (kit@1), following the `dataviz` skill's
 *  validated reference instance.
 *
 *  Categorical (series identity): eight hues in a FIXED order (the order is
 *  the colour-blind-safety mechanism), assigned in sequence and never cycled.
 *  The dark column is the same hues stepped for a dark surface. `kit.css`
 *  declares them as `--hc-s1`…`--hc-s8` (light on `:root`, dark under
 *  `:root[data-hc-scheme=dark]`); an author can override any slot. Which
 *  column applies follows the host theme: the kit reads the live
 *  `--app-surface` token (`schemeOf`) and sets `data-hc-scheme`, again on
 *  every `hollow-theme` message. Chart chrome (text, axes, grid, the surface
 *  gap) reads the `--app-*` tokens directly.
 *
 *  Validated with the skill's `validate_palette.js` against Hollow's own
 *  surfaces (light `--surface-card` #ffffff, dark #232120): every hard gate
 *  passes in both modes; three light slots sit under 3:1 contrast, so every
 *  chart ships the relief channels (legend, direct labels, a table view).
 *  `src/canvas/palette.test.ts` re-checks contrast and the CSS copy. */

export const SERIES_LIGHT = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#4a3aa7', '#e34948'] as const;
export const SERIES_DARK = ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9', '#e66767'] as const;
/** Series past this many are never drawn in a new colour (table only). */
export const MAX_SERIES = 8;

/** Marker shapes per series slot, so a line's identity never rests on colour:
 *  the legend shows the same glyph the plot draws. */
export const MARKER_GLYPHS = ['●', '■', '▲', '◆', '▼', '⬟', '⬢', '◀'] as const;

export type Rgb = [number, number, number];

/** `#rrggbb` (or `#rrggbbaa`) or `rgb()/rgba()`, the forms a canvas
 *  normalises any CSS colour to. Null for anything else. */
export function parseColor(c: string): Rgb | null {
  const m = /^#([0-9a-f]{6})|^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(c.trim());
  if (!m) return null;
  return m[1] ? [0, 2, 4].map(i => parseInt(m[1].slice(i, i + 2), 16)) as Rgb : [Number(m[2]), Number(m[3]), Number(m[4])];
}

/** WCAG relative luminance. */
export function luminance([r, g, b]: Rgb): number {
  const f = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Which palette column suits a surface colour. */
export function schemeOf(surface: string): 'light' | 'dark' {
  const rgb = parseColor(surface);
  return rgb && luminance(rgb) < 0.2 ? 'dark' : 'light';
}
