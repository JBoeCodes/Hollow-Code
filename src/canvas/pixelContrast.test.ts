import { describe, expect, it } from 'vitest';
import { MIN_GLYPH_PIXELS, contrastRatio, crop, deviceBounds, scoreBox, type BoxCrops, type Pixels } from './pixelContrast';

/** The four pictures of one box: a ground, and text of `ink` where `glyph`. */
function crops(width: number, height: number, ground: (x: number, y: number) => number[], ink: number[], glyph: (x: number, y: number) => boolean): BoxCrops {
  const make = (paint: (x: number, y: number) => number[]): Pixels => {
    const data = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) data.set([...paint(x, y), 255], (y * width + x) * 4);
    return { width, height, data };
  };
  return {
    shown: make((x, y) => (glyph(x, y) ? ink : ground(x, y))),
    hidden: make(ground),
    black: make((x, y) => (glyph(x, y) ? [0, 0, 0] : ground(x, y))),
    white: make((x, y) => (glyph(x, y) ? [255, 255, 255] : ground(x, y))),
  };
}
const strokes = (x: number, y: number) => y >= 6 && y < 14 && x % 3 === 0;

describe('contrast on the rendered pixels (repo ideas 2026-10-27 #4)', () => {
  it('scores the glyphs against what lies under each of them', () => {
    // White text over a light grey dither (200/220): it fails.
    const faint = scoreBox(crops(40, 20, (x, y) => ((x + y) % 2 ? [220, 220, 220] : [200, 200, 200]), [255, 255, 255], strokes), 4.5);
    expect(faint.kind === 'measured' && faint.ratio).toBeLessThan(1.7);
    // Black text on the same ground reads well.
    const dark = scoreBox(crops(40, 20, () => [220, 220, 220], [0, 0, 0], strokes), 4.5);
    expect(dark).toMatchObject({ kind: 'measured', need: 4.5 });
    expect(dark.kind === 'measured' && dark.ratio).toBeGreaterThan(10);
  });

  it('counts glyphs that vanish into their ground: half-unreadable text fails', () => {
    // White text over a ground that is white on the left and black on the
    // right: the left half can't be seen, so the text must not pass.
    const split = scoreBox(crops(40, 20, x => (x < 20 ? [255, 255, 255] : [0, 0, 0]), [255, 255, 255], strokes), 4.5);
    expect(split.kind).toBe('measured');
    expect(split.kind === 'measured' && split.ratio).toBeLessThan(1.1);
    // A small dark patch under a quarter or less of it doesn't sink it.
    const mostly = scoreBox(crops(40, 20, x => (x >= 33 ? [255, 255, 255] : [0, 0, 0]), [255, 255, 255], strokes), 4.5);
    expect(mostly.kind === 'measured' && mostly.ratio).toBeGreaterThan(20);
  });

  it('leaves too little text, or crops that disagree, unresolved, never guessed', () => {
    const tiny = scoreBox(crops(20, 20, () => [255, 255, 255], [0, 0, 0], (x, y) => x === 10 && y < MIN_GLYPH_PIXELS - 2), 4.5);
    expect(tiny).toEqual({ kind: 'unresolved', why: 'few-pixels' });
    const c = crops(10, 10, () => [0, 0, 0], [255, 255, 255], () => true);
    expect(scoreBox({ ...c, hidden: { width: 9, height: 10, data: new Uint8ClampedArray(9 * 10 * 4) } }, 4.5)).toEqual({ kind: 'unresolved', why: 'outside' });
  });

  it('maps a CSS box to device pixels inside the picture and cuts it out', () => {
    expect(deviceBounds([10, 20, 30, 5], 2, 1000, 1000)).toEqual({ x0: 20, y0: 40, x1: 80, y1: 50 });
    expect(deviceBounds([490, 0, 30, 5], 2, 1000, 1000)).toEqual({ x0: 980, y0: 0, x1: 1000, y1: 10 });
    expect(deviceBounds([600, 0, 30, 5], 2, 1000, 1000)).toBeNull();
    expect(deviceBounds([0, 0, 30, 5], 0, 1000, 1000)).toBeNull();
    const picture: Pixels = { width: 4, height: 3, data: Uint8ClampedArray.from({ length: 48 }, (_, i) => i) };
    const cut = crop(picture, { x0: 1, y0: 1, x1: 3, y1: 3 });
    expect([cut.width, cut.height]).toEqual([2, 2]);
    expect(Array.from(cut.data.subarray(0, 4))).toEqual([20, 21, 22, 23]);
    expect(Array.from(cut.data.subarray(8, 12))).toEqual([36, 37, 38, 39]);
  });

  it('uses the WCAG ratio', () => {
    expect(contrastRatio([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 5);
    expect(contrastRatio([118, 118, 118], [255, 255, 255])).toBeCloseTo(4.54, 1);
  });
});
