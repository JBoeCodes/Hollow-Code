import { describe, expect, it, vi } from 'vitest';
import { FOLDER_REFUSED, carriesFiles, dragLabel, isFolderLike, splitDrop, takeComposerDrop } from './composerDrop';

const file = (name: string, type: string, body = 'x') => new File(body ? [body] : [], name, { type });
function drop(files: File[]) {
  return { dataTransfer: { files } as unknown as DataTransfer, preventDefault: vi.fn() };
}

describe('the composer drop target (P0-07, C5)', () => {
  it('attaches a file that is not an image: no default action, nothing refused', () => {
    const pdf = file('report.pdf', 'application/pdf');
    const e = drop([pdf]);
    const attach = vi.fn(); const refuse = vi.fn();
    expect(takeComposerDrop(e, { attach, refuse })).toBe(true);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(attach).toHaveBeenCalledWith(pdf, false);
    expect(refuse).not.toHaveBeenCalled();
  });

  it('asks the entry API which item is a folder where the webview has it', () => {
    const empty = file('LICENSE', '', '');
    const designs = file('Designs', '', '');
    const item = (f: File, isDirectory: boolean | null) => ({ kind: 'file', getAsFile: () => f, webkitGetAsEntry: () => (isDirectory == null ? null : { isDirectory }) });
    // An empty file with no extension is a file when its entry says so.
    expect(splitDrop({ items: [item(empty, false), item(designs, true)] })).toEqual({ files: [{ file: empty, known: true }], folders: ['Designs'] });
    // No entry (a file a page made): the guess decides, and it isn't known.
    expect(splitDrop({ items: [item(empty, null)] })).toEqual({ files: [], folders: ['LICENSE'] });
    const pdf = file('a.pdf', 'application/pdf');
    expect(splitDrop({ items: [item(pdf, null), { kind: 'string', getAsFile: () => null }] })).toEqual({ files: [{ file: pdf, known: false }], folders: [] });
  });

  it('cancels the default of a drop whose files cannot be read, attaching nothing', () => {
    const e = { dataTransfer: { items: [{ kind: 'file', getAsFile: () => null }] }, preventDefault: vi.fn() };
    const attach = vi.fn(); const refuse = vi.fn();
    expect(takeComposerDrop(e, { attach, refuse })).toBe(true);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(attach).not.toHaveBeenCalled();
    expect(refuse).not.toHaveBeenCalled();
  });

  it('attaches every file of a mixed drop and refuses its folders once', () => {
    const png = file('shot.png', 'image/png');
    const log = file('run.log', 'text/plain');
    const untyped = file('notes.md', '');
    const e = drop([png, log, untyped, file('Designs', '', ''), file('Old', '', '')]);
    const attach = vi.fn(); const refuse = vi.fn();
    takeComposerDrop(e, { attach, refuse });
    expect(e.preventDefault).toHaveBeenCalledTimes(1);
    expect(attach.mock.calls.map(c => c[0])).toEqual([png, log, untyped]);
    expect(refuse).toHaveBeenCalledTimes(1);
    expect(refuse).toHaveBeenCalledWith(['Designs', 'Old']);
  });

  it('reads a folder as no type and no bytes, never a typed or non-empty file', () => {
    expect(isFolderLike({ type: '', size: 0 })).toBe(true);
    expect(isFolderLike({ type: 'text/plain', size: 0 })).toBe(false);
    expect(isFolderLike({ type: '', size: 12 })).toBe(false);
  });

  it('leaves a drop with no files alone (text, links)', () => {
    const e = drop([]);
    const refuse = vi.fn();
    expect(takeComposerDrop(e, { attach: vi.fn(), refuse })).toBe(false);
    expect(e.preventDefault).not.toHaveBeenCalled();
    expect(refuse).not.toHaveBeenCalled();
  });

  it('labels the target for any dragged file, and nothing else', () => {
    const item = (kind: string, type: string) => ({ kind, type });
    expect(dragLabel([item('file', 'image/png')])).toBe('Drop files to attach');
    expect(dragLabel([item('file', 'application/pdf')])).toBe('Drop files to attach');
    expect(dragLabel([item('file', '')])).toBe('Drop files to attach');
    expect(dragLabel([item('string', 'text/plain')])).toBeNull();
    expect(dragLabel([])).toBeNull();
  });

  it('says where a folder goes instead', () => {
    expect(FOLDER_REFUSED).toBe("Folders don't attach to a message. Attach the files inside it, or add the folder in Sources.");
  });

  // 2026-11-04: WebKit (the installed app) may list no items while a file
  // is dragged; `types` says "Files" in every engine.
  it('knows a file drag by its types alone, as WebKit gives it', () => {
    expect(carriesFiles({ types: ['Files'], items: [] })).toBe(true);
    expect(carriesFiles({ types: [], items: [{ kind: 'file' }] })).toBe(true);
    expect(carriesFiles({ types: [], files: [file('a.pdf', 'application/pdf')] })).toBe(true);
    // A conversation, task or link drag carries none.
    expect(carriesFiles({ types: ['application/x-hollow-task', 'text/plain'], items: [{ kind: 'string' }] })).toBe(false);
    expect(carriesFiles(null)).toBe(false);
  });

  it('takes a drop that says Files in its types even before items list it', () => {
    const pdf = file('report.pdf', 'application/pdf');
    const e = { dataTransfer: { types: ['Files'], files: [pdf] } as unknown as DataTransfer, preventDefault: vi.fn() };
    const attach = vi.fn();
    expect(takeComposerDrop(e, { attach, refuse: vi.fn() })).toBe(true);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(attach).toHaveBeenCalledWith(pdf, false);
  });
});
