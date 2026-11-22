/** Repo ideas 2026-10-27 #4: contrast measured on the rendered pixels, for
 *  the design check's unresolved texts (text over a gradient, an image, a
 *  pattern or a texture, which a style read can't score). The canvas is
 *  drawn four times offscreen (Rust `canvas_measure`), animations frozen and
 *  only the text's own fill changed (`-webkit-text-fill-color`, so anything
 *  painted in `currentColor` stays put): as it is (`shown`), with its text
 *  transparent (`hidden`: the ground under it), and with its text black and
 *  white. Where black and white differ is the text's coverage, found apart
 *  from its colour, so glyphs that vanish into their ground still count.
 *  Each well-covered pixel's ratio is WCAG's between `shown` and `hidden`
 *  there, and the text scores the lower quartile of those: a quarter of its
 *  glyphs below the need fails it. Fewer than `MIN_GLYPH_PIXELS` covered
 *  pixels can't be scored and stay unresolved. Pure, so it's unit-tested;
 *  capturing and cropping the pictures is `measurePicture.ts`. */

/** RGBA pixels, row by row (ImageData's shape): a crop of one text's box. */
export interface Pixels { width: number; height: number; data: Uint8ClampedArray }
/** The four pictures' crops of one box, all the same size. */
export interface BoxCrops { shown: Pixels; hidden: Pixels; black: Pixels; white: Pixels }
export type PixelVerdict =
  | { kind: 'measured'; ratio: number; need: 3 | 4.5; pixels: number }
  | { kind: 'unresolved'; why: 'few-pixels' | 'outside' | 'too-large' };

/** Fewer well-covered glyph pixels than this can't be scored. */
export const MIN_GLYPH_PIXELS = 8;
/** A pixel is a glyph's when black and white text differ by this share of
 *  full range there (anti-aliased edges below it blend text and ground). */
const COVERED = 0.75;

function luminance(r: number, g: number, b: number): number {
  const lin = (c: number) => { const v = c / 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast between two sRGB colours. */
export function contrastRatio(a: readonly number[], b: readonly number[]): number {
  const [l1, l2] = [luminance(a[0], a[1], a[2]), luminance(b[0], b[1], b[2])];
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

/** Score one text from its box's four crops. */
export function scoreBox(c: BoxCrops, need: 3 | 4.5): PixelVerdict {
  const { width, height } = c.shown;
  if ([c.hidden, c.black, c.white].some(p => p.width !== width || p.height !== height) || !width || !height) return { kind: 'unresolved', why: 'outside' };
  const ratios: number[] = [];
  for (let i = 0; i < width * height * 4; i += 4) {
    const cover = Math.min(c.white.data[i] - c.black.data[i], c.white.data[i + 1] - c.black.data[i + 1], c.white.data[i + 2] - c.black.data[i + 2]) / 255;
    if (cover < COVERED) continue;
    const s = c.shown.data, h = c.hidden.data;
    const [l1, l2] = [luminance(s[i], s[i + 1], s[i + 2]), luminance(h[i], h[i + 1], h[i + 2])];
    ratios.push((Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05));
  }
  if (ratios.length < MIN_GLYPH_PIXELS) return { kind: 'unresolved', why: 'few-pixels' };
  ratios.sort((a, b) => a - b);
  const quartile = ratios[Math.floor((ratios.length - 1) / 4)];
  return { kind: 'measured', ratio: Math.round(quartile * 100) / 100, need, pixels: ratios.length };
}

/** A box `[x, y, w, h]` in CSS px as device-pixel bounds inside an image of
 *  `width` × `height`, or null when nothing of it is inside. */
export function deviceBounds(box: readonly [number, number, number, number], scale: number, width: number, height: number): { x0: number; y0: number; x1: number; y1: number } | null {
  if (!(scale > 0)) return null;
  const x0 = Math.max(0, Math.floor(box[0] * scale)), y0 = Math.max(0, Math.floor(box[1] * scale));
  const x1 = Math.min(width, Math.ceil((box[0] + box[2]) * scale)), y1 = Math.min(height, Math.ceil((box[1] + box[3]) * scale));
  return x1 > x0 && y1 > y0 ? { x0, y0, x1, y1 } : null;
}

/** The pixels of `bounds` cut out of a whole picture. */
export function crop(picture: Pixels, b: { x0: number; y0: number; x1: number; y1: number }): Pixels {
  const width = b.x1 - b.x0, height = b.y1 - b.y0, data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) data.set(picture.data.subarray(((b.y0 + y) * picture.width + b.x0) * 4, ((b.y0 + y) * picture.width + b.x1) * 4), y * width * 4);
  return { width, height, data };
}
