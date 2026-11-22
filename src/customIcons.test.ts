import { describe, it, expect } from 'vitest';
import type { ComponentProps } from 'solid-js';
import type { Icon } from './icons';
import { MAX_UPLOAD_BYTES, UPLOAD_EDITOR, checkUploadBytes, createIconTickets, drawUpload, fileOriginal, svgOriginal, uploadFormat } from './customIcons';

// Compile-only acceptance: misspelled glyphs must fail before rendering blank.
// @ts-expect-error image is not an icon name; use fileImage for file artwork.
const unknownIcon: ComponentProps<typeof Icon>['name'] = 'image';
void unknownIcon;

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13]);
const JPEG = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0, 16, 0x4a, 0x46, 0x49, 0x46, 0, 1]);
const SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#7050ac"/></svg>';
const file = (name: string, data: Uint8Array | string, size?: number) => {
  const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
  let read = false;
  const f = { name, size: size ?? bytes.length, arrayBuffer: () => { read = true; return Promise.resolve(bytes.slice().buffer); } } as unknown as File;
  return { f, wasRead: () => read };
};

describe('Upload an icon (overhaul H1)', () => {
  it('takes PNG, JPEG or SVG, by name, and says so for anything else', () => {
    expect(uploadFormat('Art.PNG')).toBe('png');
    expect(uploadFormat('logo.svg')).toBe('svg');
    expect(uploadFormat('photo.JPG')).toBe('jpeg');
    expect(uploadFormat('photo.jpeg')).toBe('jpeg');
    for (const name of ['a.gif', 'a.icns', 'a.html', 'noext']) expect(() => uploadFormat(name)).toThrow('Choose a PNG, JPEG or SVG file.');
  });

  it('a JPEG is checked by its signature and sent as a jpeg original (lane S 11-13)', async () => {
    expect(() => checkUploadBytes('jpeg', JPEG)).not.toThrow();
    expect(() => checkUploadBytes('jpeg', PNG)).toThrow('not a JPEG');
    await expect(fileOriginal(file('renamed.jpg', PNG).f)).rejects.toThrow('not a JPEG');
    expect(await fileOriginal(file('photo.jpg', JPEG).f)).toEqual({ format: 'jpeg', bytes: btoa(String.fromCharCode(...JPEG)) });
  });

  it('refuses an empty or oversized file before reading its bytes', async () => {
    const big = file('large.png', PNG, MAX_UPLOAD_BYTES + 1);
    await expect(fileOriginal(big.f)).rejects.toThrow('8 MiB');
    expect(big.wasRead()).toBe(false);
    const empty = file('empty.svg', '', 0);
    await expect(fileOriginal(empty.f)).rejects.toThrow('8 MiB');
    expect(empty.wasRead()).toBe(false);
  });

  it('checks the bytes match the type: a PNG signature, an SVG document', async () => {
    expect(() => checkUploadBytes('png', PNG)).not.toThrow();
    expect(() => checkUploadBytes('png', new TextEncoder().encode(SVG))).toThrow('not a PNG');
    expect(() => checkUploadBytes('svg', PNG)).toThrow();
    expect(() => checkUploadBytes('svg', new TextEncoder().encode('<html><body/></html>'))).toThrow('not an SVG');
    await expect(fileOriginal(file('renamed.png', SVG).f)).rejects.toThrow('not a PNG');
  });

  it('returns the original as base64 for the backend check and save', async () => {
    const png = await fileOriginal(file('a.png', PNG).f);
    expect(png).toEqual({ format: 'png', bytes: btoa(String.fromCharCode(...PNG)) });
    const svg = await fileOriginal(file('a.svg', SVG).f);
    expect(svg.format).toBe('svg');
    expect(atob(svg.bytes)).toBe(SVG);
  });

  it('a generated SVG becomes the same kind of original, UTF-8 kept', () => {
    const text = SVG.replace('</svg>', '<title>Café</title></svg>');
    const o = svgOriginal(text);
    expect(o.format).toBe('svg');
    expect(new TextDecoder().decode(Uint8Array.from(atob(o.bytes), c => c.charCodeAt(0)))).toBe(text);
    expect(() => svgOriginal('not an svg')).toThrow();
  });

  it('every save records the fitted whole picture as its editor state', () => {
    expect(UPLOAD_EDITOR).toEqual({ mode: 'upload', fit: 'fit' });
  });
});

describe('the fitted render and the stale-result guard (lane S 11-13)', () => {
  const canvas = () => {
    const calls: number[][] = [];
    const c = { clearRect: () => {}, drawImage: (_s: unknown, ...a: number[]) => { calls.push(a); } };
    return { el: { width: 512, height: 512, getContext: () => c } as unknown as HTMLCanvasElement, calls };
  };
  // What a browser reports for an SVG with a viewBox and no width or height: a default 2:1 size
  // (300 x 150 in practice; 256 x 128 here so the arithmetic is exact).
  const viewBoxOnly = { naturalWidth: 256, naturalHeight: 128 } as HTMLImageElement;

  it('sizes from the prepared width and height, not the image element', () => {
    const wide = canvas();
    drawUpload(wide.el, viewBoxOnly);
    expect(wide.calls).toEqual([[0, 128, 512, 256]]);
    // prepare read a square viewBox: drawn square, filling the canvas.
    const square = canvas();
    drawUpload(square.el, viewBoxOnly, { width: 64, height: 64 });
    expect(square.calls).toEqual([[0, 0, 512, 512]]);
    const tall = canvas();
    drawUpload(tall.el, viewBoxOnly, { width: 32, height: 64 });
    expect(tall.calls).toEqual([[128, 0, 256, 512]]);
    // An unusable size falls back to the element's own.
    const zero = canvas();
    drawUpload(zero.el, viewBoxOnly, { width: 0, height: 64 });
    expect(zero.calls).toEqual([[0, 128, 512, 256]]);
  });

  it('generate retires a pending prepare, like an upload or pick', () => {
    const t = createIconTickets();
    const pick = t.prepare();
    expect(t.isPrepare(pick)).toBe(true);
    const gen = t.generate();
    expect(t.isPrepare(pick)).toBe(false);
    expect(t.isGenerate(gen)).toBe(true);
    // A newer generation retires the older one; a later pick does not retire the generation.
    const newer = t.generate();
    expect(t.isGenerate(gen)).toBe(false);
    const upload = t.prepare();
    expect(t.isGenerate(newer)).toBe(true);
    expect(t.isPrepare(upload)).toBe(true);
    // Closing retires everything pending.
    t.close();
    expect(t.isGenerate(newer)).toBe(false);
    expect(t.isPrepare(upload)).toBe(false);
  });
});
