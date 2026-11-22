import { describe, expect, it } from 'vitest';
import { readDraft, writeDraft, clearDraft } from './fileDrafts';
import { afterReread, rereadGate, saveSnapshot, unreadableReason, type CasResult } from './fileSave';

function mem(): Storage {
  const m = new Map<string, string>();
  return { getItem: k => m.get(k) ?? null, setItem: (k, v) => { m.set(k, String(v)); }, removeItem: k => { m.delete(k); }, key: i => [...m.keys()][i] ?? null, get length() { return m.size; }, clear: () => m.clear() };
}

/** A compare-and-swap save the test resolves by hand, so it can type while
 *  the save is in flight. */
function slowDisk(initial: string) {
  let disk = initial;
  const calls: { value: string; base: string }[] = [];
  let release: () => void = () => {};
  const cas = (_path: string, value: string, base: string) => {
    calls.push({ value, base });
    return new Promise<CasResult>(done => {
      release = () => {
        if (base !== disk) { done({ conflict: true, note: { content: disk } }); return; }
        disk = value; done({ conflict: false, note: { content: value } });
      };
    });
  };
  return { cas, calls, disk: () => disk, land: () => release(), edit: (t: string) => { disk = t; } };
}

/** The editor's draft state for one path, as WorkFileEditor keeps it. */
type Live = { text: string; typed: (t: string) => void };
function editor(path: string, store: Storage, text: string): Live {
  const state: Live = { text, typed: (t: string) => { state.text = t; writeDraft(path, t, 'base', store); } };
  return state;
}

const deps = (path: string, store: Storage, cas: ReturnType<typeof slowDisk>['cas'], live: { text: string }) => ({
  cas,
  latest: () => live.text,
  keep: (text: string, baseline: string) => writeDraft(path, text, baseline, store),
  forget: () => clearDraft(path, store, mem()),
});

describe('keystrokes typed during a save are kept (review A, slice 4 item 1d)', () => {
  const path = 'workspaces/w/outputs/report.md';

  it('saves the snapshot and keeps text typed while the save was in flight as a draft', async () => {
    const store = mem(), disk = slowDisk('base');
    const live = editor(path, store, 'base + one');
    const pending = saveSnapshot(path, live.text, 'base', deps(path, store, disk.cas, live), { keepTyped: true });
    live.typed('base + one + two'); // typed after the snapshot, before the save lands
    disk.land();
    const outcome = await pending;
    expect(disk.disk()).toBe('base + one'); // the save wrote the snapshot it took
    expect(outcome).toEqual({ kind: 'saved', baseline: 'base + one', text: 'base + one + two' });
    // Still dirty, and the stored draft survives a restart against the new baseline.
    expect(readDraft(path, store, mem())).toMatchObject({ text: 'base + one + two', baseline: 'base + one' });
  });

  it('the next save writes the typed text against the version the first save wrote', async () => {
    const store = mem(), disk = slowDisk('base');
    const live = editor(path, store, 'v1');
    const first = saveSnapshot(path, live.text, 'base', deps(path, store, disk.cas, live), { keepTyped: true });
    live.typed('v2');
    disk.land();
    const settled = await first;
    expect(settled.kind).toBe('saved');
    if (settled.kind !== 'saved') return;
    const second = saveSnapshot(path, settled.text, settled.baseline, deps(path, store, disk.cas, live), { keepTyped: true });
    disk.land();
    expect(await second).toEqual({ kind: 'saved', baseline: 'v2', text: 'v2' });
    expect(disk.calls).toEqual([{ value: 'v1', base: 'base' }, { value: 'v2', base: 'v1' }]);
    expect(disk.disk()).toBe('v2');
    expect(readDraft(path, store, mem())).toBeNull(); // nothing left unsaved
  });

  it('a save with no typing in flight clears the draft', async () => {
    const store = mem(), disk = slowDisk('base');
    const live = editor(path, store, 'edited');
    writeDraft(path, 'edited', 'base', store);
    const pending = saveSnapshot(path, live.text, 'base', deps(path, store, disk.cas, live), { keepTyped: true });
    disk.land();
    expect(await pending).toEqual({ kind: 'saved', baseline: 'edited', text: 'edited' });
    expect(readDraft(path, store, mem())).toBeNull();
  });

  it('a conflict offers the newest draft, typing in flight included', async () => {
    const store = mem(), disk = slowDisk('base');
    const live = editor(path, store, 'mine');
    const pending = saveSnapshot(path, live.text, 'base', deps(path, store, disk.cas, live), { keepTyped: true });
    disk.edit('theirs');
    live.typed('mine, more');
    disk.land();
    expect(await pending).toEqual({ kind: 'conflict', disk: 'theirs', draft: 'mine, more' });
    expect(disk.disk()).toBe('theirs');
    expect(readDraft(path, store, mem())?.text).toBe('mine, more');
  });

  it('a conflict choice (keepTyped off) saves exactly the chosen text and clears the draft', async () => {
    const store = mem(), disk = slowDisk('theirs');
    writeDraft(path, 'mine', 'base', store);
    const live = { text: 'mine' }; // the old draft is not newer typing
    const pending = saveSnapshot(path, 'merged', 'theirs', deps(path, store, disk.cas, live), { keepTyped: false });
    disk.land();
    expect(await pending).toEqual({ kind: 'saved', baseline: 'merged', text: 'merged' });
    expect(readDraft(path, store, mem())).toBeNull();
  });
});

describe('a conflict whose file on disk can’t be read (concurrency PR 2 delta review 3)', () => {
  const unreadable = (_path: string, _value: string, _base: string): Promise<CasResult> =>
    Promise.resolve({ conflict: true, note: { content: '', revision: 'unreadable: Permission denied (os error 13)' } });

  it('is its own outcome: the draft is kept and nothing claims the disk is empty', async () => {
    const store = mem();
    const live = { text: 'my draft' };
    const out = await saveSnapshot('w/a.md', 'my draft', 'base', { ...deps('w/a.md', store, unreadable, live) }, { keepTyped: false });
    expect(out).toEqual({ kind: 'unreadable', reason: 'Permission denied (os error 13)', draft: 'my draft' });
    expect(readDraft('w/a.md', store)?.text).toBe('my draft');
    expect(readDraft('w/a.md', store)?.baseline).toBe('base');
  });

  it('offers disk adoption or a merge only after a successful reread', () => {
    expect(afterReread({ ok: false, error: 'Permission denied' }, 'base', 'mine')).toEqual({ kind: 'unreadable', reason: 'Permission denied' });
    expect(afterReread({ ok: true, content: 'theirs' }, 'base', 'mine')).toEqual({ kind: 'conflict', base: 'base', disk: 'theirs', draft: 'mine' });
    expect(afterReread({ ok: true, content: 'base' }, 'base', 'mine')).toEqual({ kind: 'draft', baseline: 'base', text: 'mine' });
  });

  it('knows the sentinel', () => {
    expect(unreadableReason({ content: '', revision: 'unreadable: EACCES' })).toBe('EACCES');
    expect(unreadableReason({ content: '', revision: 'e3b0c442' })).toBeNull();
    expect(unreadableReason({ content: 'x' })).toBeNull();
  });
});

describe('Read the file again runs one read at a time (delta review 4)', () => {
  it('refuses a second read while one is pending and drops a response for a state that moved on', () => {
    const gate = rereadGate<{ draft: string }>();
    const first = { draft: 'a' };
    const token = gate.start(first);
    expect(token).not.toBeNull();
    expect(gate.start(first), 'a second click while pending').toBeNull();
    expect(gate.pending()).toBe(true);
    // The state the read was for is gone (a newer conflict, a reload, typing after recovery).
    expect(gate.finish(token!, { draft: 'a' })).toBe(false);
    expect(gate.pending()).toBe(false);
    const again = gate.start(first);
    expect(gate.finish(again!, first)).toBe(true);
    expect(gate.finish(again!, first), 'a token settles once').toBe(false);
  });
});
