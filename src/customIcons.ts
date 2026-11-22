/** Upload an icon (overhaul H1, 2026-11-12): the checks a picked file passes
 *  before the backend's own (`custom_icons.rs`), and the 512 px render that
 *  is saved. Replaces the icon maker. No network. */
export type IconTarget = 'app' | 'mainAgent';
/** Uploads are PNG, JPEG or SVG; older records and bundled presets may also hold ICNS originals. */
export interface IconOriginal { format: 'png' | 'jpeg' | 'svg' | 'icns'; bytes: string }
export interface CustomIcon { version: number; revision: string; preset?: string; original: IconOriginal; editor: unknown; pngs: Record<number, string> }
export interface PreparedIcon { url: string; width: number; height: number }
export const ICON_SIZES = [16, 32, 64, 128, 512] as const;
export const MAX_UPLOAD_BYTES = 8 * 1024 * 1024;
/** What every save from the dialog records as its editor state: the whole picture, fitted. */
export const UPLOAD_EDITOR = { mode: 'upload', fit: 'fit' } as const;
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_SIGNATURE = [0xff, 0xd8, 0xff];
export type UploadFormat = 'png' | 'jpeg' | 'svg';

/** The upload's format by name. JPEG is taken too (lane S 11-13): the backend's
 *  `prepare` already decodes it with the same 4096 px limits as PNG and hands back a
 *  PNG preview, so nothing new runs; refusing it only confused people with photos. */
export function uploadFormat(name: string): UploadFormat {
  const suffix = name.split('.').pop()?.toLowerCase();
  if (suffix === 'png' || suffix === 'svg') return suffix;
  if (suffix === 'jpg' || suffix === 'jpeg') return 'jpeg';
  throw new Error('Choose a PNG, JPEG or SVG file.');
}

/** The file's type from its bytes: a PNG or JPEG signature, or text that is an SVG document. */
export function checkUploadBytes(format: UploadFormat, data: Uint8Array): void {
  if (format === 'png') {
    if (!PNG_SIGNATURE.every((b, i) => data[i] === b)) throw new Error('This file is not a PNG image.');
    return;
  }
  if (format === 'jpeg') {
    if (!JPEG_SIGNATURE.every((b, i) => data[i] === b)) throw new Error('This file is not a JPEG image.');
    return;
  }
  let text: string;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(data); }
  catch { throw new Error('This SVG is not UTF-8 text.'); }
  if (!/<svg[\s>]/.test(text)) throw new Error('This file is not an SVG image.');
}

function base64(data: Uint8Array): string {
  let binary = '';
  for (let at = 0; at < data.length; at += 8192) binary += String.fromCharCode(...data.subarray(at, at + 8192));
  return btoa(binary);
}

/** A picked file as an upload: PNG, JPEG or SVG, at most 8 MiB, its bytes checked. Never reads an oversized file. */
export async function fileOriginal(file: File): Promise<IconOriginal> {
  if (!file.size || file.size > MAX_UPLOAD_BYTES) throw new Error('Choose an image no larger than 8 MiB.');
  const format = uploadFormat(file.name);
  const data = new Uint8Array(await file.arrayBuffer());
  checkUploadBytes(format, data);
  return { format, bytes: base64(data) };
}

/** A generated SVG document as an upload. */
export function svgOriginal(svg: string): IconOriginal {
  const data = new TextEncoder().encode(svg);
  if (data.length > MAX_UPLOAD_BYTES) throw new Error('This logo is larger than 8 MiB.');
  checkUploadBytes('svg', data);
  return { format: 'svg', bytes: base64(data) };
}

export function loadIconImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve,reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('This image could not be drawn. Try exporting it as PNG.'));
    image.src = url;
  });
}

/** The picture's own size. `prepare_custom_icon` returns it (an SVG's from its width and height, else its
 *  viewBox); the image element's natural size is only a fallback, because a browser gives an SVG with a viewBox
 *  and no width or height a default size (300 x 150, or 0), which drew such an icon squashed (lane S 11-13). */
export interface IconSize { width: number; height: number }

/** Draw `source` fitted whole inside a transparent square (never cropped), at `size`'s aspect when given.
 *  SVG is rasterized here, after the backend's static-SVG check. */
export function drawUpload(canvas: HTMLCanvasElement, source: HTMLImageElement, size?: IconSize): void {
  const c = canvas.getContext('2d');
  if (!c) throw new Error('Image preview is unavailable.');
  c.clearRect(0,0,canvas.width,canvas.height);
  const sw = size && size.width > 0 && size.height > 0 ? size.width : source.naturalWidth;
  const sh = size && size.width > 0 && size.height > 0 ? size.height : source.naturalHeight;
  const scale = Math.min(canvas.width/sw, canvas.height/sh);
  const w = sw*scale, h = sh*scale;
  c.drawImage(source,(canvas.width-w)/2,(canvas.height-h)/2,w,h);
}

/** The 512 px PNG (base64) the backend saves and scales down. */
export function renderUpload(source: HTMLImageElement, size?: IconSize): string {
  const canvas = document.createElement('canvas'); canvas.width = 512; canvas.height = 512;
  drawUpload(canvas, source, size);
  return canvas.toDataURL('image/png').split(',')[1];
}

/** The icon dialog's stale-result guard (lane S 11-13). Every upload, pick and generate takes a ticket;
 *  a result lands only while its ticket is the newest of its kind. Generate also retires any pending
 *  prepare, like an upload or pick does, and closing retires both, so an older result never lands after a
 *  newer action. */
export function createIconTickets() {
  let prepare = 0, generate = 0;
  return {
    /** An upload or a pick. */
    prepare: () => ++prepare,
    /** A generation: older generations and any pending prepare are stale. */
    generate: () => { prepare++; return ++generate; },
    /** Closing the dialog: everything pending is stale. */
    close: () => { prepare++; generate++; },
    isPrepare: (id: number) => id === prepare,
    isGenerate: (id: number) => id === generate,
  };
}
