/** Repo ideas 2026-10-27 #4: Measure on the picture, the browser half. Asks
 *  Rust for the four offscreen pictures of a reply canvas (`canvas_measure`:
 *  drawn as its frame drew it; text shown, hidden, black and white), then
 *  decodes them one at a time, keeps only each unresolved text's box and
 *  scores it (`pixelContrast.ts`). A picture past `MEASURE_MAX_PIXELS` is
 *  refused before it is decoded. Only on the person's click; nothing is
 *  saved. */
import { api } from '../api';
import type { CanvasState, DesignMessage, TextBox } from '../canvasBridge';
import { exportPalette } from './exportLook';
import { crop, deviceBounds, scoreBox, type BoxCrops, type Pixels, type PixelVerdict } from './pixelContrast';

/** The most device pixels one picture may have (Rust's
 *  `MEASURE_MAX_PIXELS`): about 48 MB once decoded. */
export const MEASURE_MAX_PIXELS = 12_000_000;
/** The most device pixels all boxes may cover together: each is cut out of
 *  all four pictures, so about 32 MB of crops at most. Boxes past it stay
 *  unresolved ("too large"). */
export const CROP_MAX_PIXELS = 2_000_000;

/** A PNG's size from its header, before anything is decoded. */
function pngSize(bytes: Uint8Array): { width: number; height: number } {
  if (bytes.length < 24 || bytes[0] !== 0x89 || bytes[1] !== 0x50) throw new Error('The picture is not a PNG.');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

/** Decode one picture and cut every box out of it; the whole picture is
 *  dropped before the next is decoded. */
async function cropsOf(base64: string, boxes: ReadonlyArray<[number, TextBox]>, scale: number): Promise<Map<number, Pixels>> {
  const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
  const { width, height } = pngSize(bytes);
  if (!width || !height || width * height > MEASURE_MAX_PIXELS) throw new Error('The canvas is too large to measure on the picture.');
  const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
  try {
    const canvas = document.createElement('canvas');
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const g = canvas.getContext('2d', { willReadFrequently: true });
    if (!g) throw new Error('The picture could not be read.');
    g.drawImage(bitmap, 0, 0);
    const image = g.getImageData(0, 0, bitmap.width, bitmap.height);
    const whole: Pixels = { width: image.width, height: image.height, data: image.data };
    const out = new Map<number, Pixels>();
    for (const [i, box] of boxes) {
      const bounds = deviceBounds(box, scale, whole.width, whole.height);
      if (bounds) out.set(i, crop(whole, bounds));
    }
    canvas.width = canvas.height = 0;
    return out;
  } finally { bitmap.close(); }
}

/** One verdict per unresolved (`texture-text`) item with a box, by its index
 *  in the report. `themeName` is the workspace's canvas look, as for Ask to
 *  fix. */
export async function measureOnPicture(source: string, state: CanvasState | null, report: DesignMessage, themeName?: string | null): Promise<Map<number, PixelVerdict>> {
  const boxes: Array<[number, TextBox]> = [];
  report.items.forEach((item, i) => { if (item.rule === 'texture-text' && item.box) boxes.push([i, item.box]); });
  const verdicts = new Map<number, PixelVerdict>();
  if (!boxes.length) return verdicts;
  const { scheme, theme } = await exportPalette(themeName);
  const pictures = await api.canvasMeasure({ source, state, scheme, theme, width: report.width, texture: report.texture });
  // Keep boxes in report order while their device area fits the crop
  // budget; the rest stay unresolved, too large to cut out.
  const { width, height } = pngSize(Uint8Array.from(atob(pictures.shown.slice(0, 44)), c => c.charCodeAt(0)));
  let area = 0;
  const kept: Array<[number, TextBox]> = [];
  for (const [i, box] of boxes) {
    const b = deviceBounds(box, pictures.scale, width, height);
    const size = b ? (b.x1 - b.x0) * (b.y1 - b.y0) : 0;
    if (b && area + size <= CROP_MAX_PIXELS) { area += size; kept.push([i, box]); }
    else verdicts.set(i, { kind: 'unresolved', why: b ? 'too-large' : 'outside' });
  }
  boxes.splice(0, boxes.length, ...kept);
  const passes: Array<keyof BoxCrops> = ['shown', 'hidden', 'black', 'white'];
  const cut: Partial<Record<keyof BoxCrops, Map<number, Pixels>>> = {};
  for (const pass of passes) cut[pass] = await cropsOf(pictures[pass], boxes, pictures.scale);
  for (const [i] of boxes) {
    const need = (report.items[i] as { need: 3 | 4.5 }).need;
    const [shown, hidden, black, white] = passes.map(p => cut[p]!.get(i));
    verdicts.set(i, shown && hidden && black && white ? scoreBox({ shown, hidden, black, white }, need) : { kind: 'unresolved', why: 'outside' });
  }
  return verdicts;
}
