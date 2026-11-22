import { describe, expect, it } from 'vitest';
import { FOLDER_DROP, MAX_IMPORT_BYTES, bytesToBase64, captureDrop, fileSource, formatBytes, isFileDrag, suggestedRename, type EntryLike, type FileLike } from './skillImport';

const text = (s: string) => new TextEncoder().encode(s);
const decode = (b64: string) => new TextDecoder().decode(Uint8Array.from(atob(b64), c => c.charCodeAt(0)));

function fakeFile(name: string, content: string | Uint8Array, size?: number): FileLike {
  const bytes = typeof content === 'string' ? text(content) : content;
  return { name, size: size ?? bytes.length, arrayBuffer: async () => bytes.slice().buffer };
}

describe('skill import sources', () => {
  it('base64-encodes large inputs in chunks without losing bytes', () => {
    const bytes = new Uint8Array(200_000).map((_, i) => i % 256);
    const b64 = bytesToBase64(bytes);
    expect(Uint8Array.from(atob(b64), c => c.charCodeAt(0))).toEqual(bytes);
  });

  it('a dropped file becomes its bytes under its name', async () => {
    const source = await fileSource(fakeFile('pdf-tools.skill', 'PK\u0003\u0004…'));
    expect(source.kind).toBe('bytes');
    if (source.kind !== 'bytes') return;
    expect(source.name).toBe('pdf-tools.skill');
    expect(decode(source.data)).toBe('PK\u0003\u0004…');
  });

  it('refuses a file over the cap before reading it', async () => {
    const huge: FileLike = { name: 'big.zip', size: MAX_IMPORT_BYTES + 1, arrayBuffer: () => { throw new Error('read'); } };
    await expect(fileSource(huge)).rejects.toThrow('larger than 50 MB');
  });

  it('captures a drop synchronously and refuses more than one item', () => {
    const file = fakeFile('SKILL.md', 'x');
    const item = (entry: EntryLike | null) => ({ kind: 'file', getAsFile: () => file, webkitGetAsEntry: () => entry });
    const one = { types: ['Files'], items: [item(null)] } as unknown as DataTransfer;
    expect(isFileDrag(one)).toBe(true);
    expect(captureDrop(one)).toEqual({ file });
    // A folder is refused: WebKit would read through links inside it.
    const folder = { types: ['Files'], items: [item({ isDirectory: true })] } as unknown as DataTransfer;
    expect(captureDrop(folder)).toEqual({ error: FOLDER_DROP });
    const plain = { types: ['Files'], items: [item({ isDirectory: false })] } as unknown as DataTransfer;
    expect(captureDrop(plain)).toEqual({ file });
    const two = { types: ['Files'], items: [item(null), item(null)] } as unknown as DataTransfer;
    expect(captureDrop(two)).toEqual({ error: expect.stringContaining('one skill at a time') });
    const session = { types: ['application/x-hollow-session'], items: [{ kind: 'string' }] } as unknown as DataTransfer;
    expect(isFileDrag(session)).toBe(false);
    expect(captureDrop(session)).toBeNull();
  });

  it('formats sizes', () => {
    expect(formatBytes(12)).toBe('12 B');
    expect(formatBytes(3482)).toBe('3.4 KB');
    expect(formatBytes(1_258_291)).toBe('1.2 MB');
  });

  it('reads the suggested name out of a refusal and nothing else', () => {
    const refusal = 'SKILL.md\'s name "stitch::generate-design" can only use letters, numbers, dashes and underscores. Import it as "stitch-generate-design" instead (pass rename_to "stitch-generate-design"); nothing was renamed or installed.';
    expect(suggestedRename(refusal)).toBe('stitch-generate-design');
    expect(suggestedRename('SKILL.md needs a `description`.')).toBeNull();
    expect(suggestedRename('Import it as "../escape" instead')).toBeNull();
    expect(suggestedRename(null)).toBeNull();
  });
});
