import { describe, expect, it } from 'vitest';
import { clearDraft, discardDraft, holdDraftOpen, listDrafts, onDraftDiscarded, openDraftPaths, readDraft, writeDraft } from './fileDrafts';

function mem(): Storage {
  const m = new Map<string, string>();
  return { getItem: k => m.get(k) ?? null, setItem: (k, v) => { m.set(k, String(v)); }, removeItem: k => { m.delete(k); }, key: i => [...m.keys()][i] ?? null, get length() { return m.size; }, clear: () => m.clear() };
}

describe('file drafts survive a restart (next-level P08)', () => {
  it('writes, reads back and lists drafts with their baseline', () => {
    const s = mem();
    expect(writeDraft('workspaces/w/outputs/a.md', 'mine', 'base', s, new Date('2026-10-01T12:00:00Z'))).toBe(true);
    expect(readDraft('workspaces/w/outputs/a.md', s, mem())).toEqual({ text: 'mine', baseline: 'base', savedAt: '2026-10-01T12:00:00.000Z' });
    writeDraft('b.md', 'same', 'same', s);
    expect(listDrafts(s).map(d => d.path)).toEqual(['workspaces/w/outputs/a.md']);
    clearDraft('workspaces/w/outputs/a.md', s, mem());
    expect(readDraft('workspaces/w/outputs/a.md', s, mem())).toBeNull();
  });
  it('migrates an older session-only draft so the next launch finds it', () => {
    const s = mem(), legacy = mem();
    legacy.setItem('hollow.file.notes/x.md', JSON.stringify({ text: 'draft', baseline: 'orig' }));
    expect(readDraft('notes/x.md', s, legacy)?.text).toBe('draft');
    expect(legacy.getItem('hollow.file.notes/x.md')).toBeNull();
    expect(readDraft('notes/x.md', s, mem())?.baseline).toBe('orig');
  });
  it('reports a full store instead of pretending the draft is safe', () => {
    const full = { ...mem(), setItem: () => { throw new Error('QuotaExceeded'); } } as unknown as Storage;
    expect(writeDraft('a.md', 'x', 'y', full)).toBe(false);
  });
});

describe('drafts open in an editor, and Review’s Discard (P1-14)', () => {
  it('holds a path open while any editor shows it, and releases each hold once', () => {
    const first = holdDraftOpen('notes/a.md');
    const second = holdDraftOpen('notes/a.md');
    expect(openDraftPaths().has('notes/a.md')).toBe(true);
    first();
    first();
    expect(openDraftPaths().has('notes/a.md')).toBe(true);
    second();
    expect(openDraftPaths().has('notes/a.md')).toBe(false);
  });

  it('discards from storage and every copy, returning the draft for Undo', () => {
    const s = mem();
    writeDraft('notes/a.md', 'mine', 'base', s, new Date('2026-10-24T12:00:00Z'));
    const told: string[] = [];
    const stop = onDraftDiscarded(path => told.push(path));
    expect(discardDraft('notes/a.md', s, mem())).toEqual({ text: 'mine', baseline: 'base', savedAt: '2026-10-24T12:00:00.000Z' });
    expect(readDraft('notes/a.md', s, mem())).toBeNull();
    expect(told).toEqual(['notes/a.md']);
    stop();
    expect(discardDraft('notes/a.md', s, mem())).toBeNull();
    expect(told).toEqual(['notes/a.md']);
  });
});
