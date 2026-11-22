import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parseSegments } from '../chatSegments';
import type { Canvas, CanvasRecorded } from '../generated/ipc';
import {
  CanvasHeads, DEFAULT_CANVAS_HEIGHT, canvasFixRequest, canvasKey, canvasStateSaver, chatCanvasView, forgetRecordedCanvases,
  headOf, htmlFenceMode, innerOf, resolveCanvas, saveCanvasStateCas, settleCanvasBody, sha256Hex, versionLabel,
  type CanvasStateCalls, type CanvasStoreCalls, type ChatCanvasScope,
} from './chatCanvas';

const BODY = '<hc-canvas id="release-status" title="Release status" height="320">\n  <hc-stats><hc-stat label="Bed" value="29"></hc-stat></hc-stats>\n</hc-canvas>';
const REPLY = `Here is where the release stands.\n\n\`\`\`hollow-canvas\n${BODY}\n\`\`\`\n\nAll green.`;

/** The canvas segment's view at every streaming flush of REPLY. */
function viewsWhileStreaming(reply: string) {
  const states: string[] = [];
  for (let n = 1; n <= reply.length; n++) {
    const seg = parseSegments(reply.slice(0, n)).find(s => s.kind === 'canvas');
    if (!seg || seg.kind !== 'canvas') continue;
    const v = chatCanvasView(seg.code, seg.closed);
    if (states[states.length - 1] !== v.state) states.push(v.state);
  }
  return states;
}

describe('a canvas while its reply streams', () => {
  it('is pending, then a skeleton once the root tag closes, then ready when the fence ends', () => {
    expect(viewsWhileStreaming(REPLY)).toEqual(['pending', 'drawing', 'ready']);
  });

  it('the skeleton names the title and reserves the root\'s height, or a default', () => {
    const drawing = chatCanvasView('<hc-canvas id="a" title="Plan &amp; risks" height="480px"><p>', false);
    expect(drawing).toEqual({ state: 'drawing', root: { id: 'a', title: 'Plan & risks', ask: false, height: 480 }, reserve: 480 });
    const plain = chatCanvasView('<hc-canvas id="a" title="A">', false);
    expect(plain.state === 'drawing' && plain.reserve).toBe(DEFAULT_CANVAS_HEIGHT);
    // Not yet: the start tag is still arriving.
    expect(chatCanvasView('<hc-canvas id="a" title="A', false)).toEqual({ state: 'pending' });
  });

  it('a body at exactly 128 KB never reads too-large while its closing fence arrives', () => {
    const at = `<hc-canvas id="big" title="Big">${'a'.repeat(131028)}</hc-canvas>`;
    const reply = `\`\`\`hollow-canvas\n${at}\n\`\``;
    const seg = parseSegments(reply)[0];
    expect(seg.kind === 'canvas' && chatCanvasView(seg.code, seg.closed).state).toBe('drawing');
  });

  it('a body at exactly 128 KB never reads too-large once the newline after its root arrives', () => {
    const at = `<hc-canvas id="big" title="Big">${'a'.repeat(131028)}</hc-canvas>`;
    for (const tail of ['\n', '\n  ', '\n\t']) {
      const seg = parseSegments(`\`\`\`hollow-canvas\n${at}${tail}`)[0];
      expect(seg.kind === 'canvas' && chatCanvasView(seg.code, seg.closed).state).toBe('drawing');
    }
  });

  it('a broken fence is invalid with its reason and as much of the root as parses', () => {
    const v = chatCanvasView('<hc-canvas data-id="x" id="Bad Id" title="Release">\n</hc-canvas>', true);
    expect(v).toMatchObject({ state: 'invalid', code: 'bad-id', root: { id: 'Bad Id', title: 'Release' } });
    expect(chatCanvasView('<div>no root</div>', true)).toMatchObject({ state: 'invalid', code: 'no-root', root: null });
    expect(chatCanvasView('<hc-canvas id="a" title="A"><p>', true)).toMatchObject({ state: 'invalid', code: 'unclosed', root: { id: 'a', title: 'A' } });
  });

  it('the ready inner is what the frame renders', () => {
    expect(innerOf(BODY, 'fallback')).toBe('<div data-hc-texture-root texture="dither" style="display:contents">\n  <hc-stats><hc-stat label="Bed" value="29"></hc-stat></hc-stats>\n</div>');
    expect(innerOf('not a canvas', 'fallback')).toBe('fallback');
  });
});

/** The canvas segment of a reply that has stopped (`settled`), as ChatCanvas reads it. */
function settledView(reply: string) {
  const seg = parseSegments(reply).find(s => s.kind === 'canvas');
  if (!seg || seg.kind !== 'canvas') throw new Error('no canvas segment');
  const body = settleCanvasBody(seg.code, seg.closed, true);
  return { body, view: chatCanvasView(body.code, body.closed) };
}

describe('a canvas whose reply settled before its fence closed', () => {
  // Stop mid-canvas, an agent error, or a provider that drops the closing
  // fence: the reply is finished, so the fence is read as ended and a
  // truncated canvas shows its source and Ask to fix, never a skeleton.
  it('is broken when the reply stopped inside the canvas', () => {
    const { view } = settledView('Status:\n\n```hollow-canvas\n<hc-canvas id="release-status" title="Release status">\n<p>29 che');
    expect(view).toMatchObject({ state: 'invalid', code: 'unclosed', root: { id: 'release-status', title: 'Release status' } });
  });

  it('is broken when the reply stopped inside the root tag or before any body', () => {
    expect(settledView('```hollow-canvas\n<hc-canvas id="release-status" ti').view).toMatchObject({ state: 'invalid', code: 'unclosed' });
    expect(settledView('```hollow-canvas\n<hc-can').view).toMatchObject({ state: 'invalid', code: 'no-root' });
    expect(settledView('Here:\n\n```hollow-canv').view).toMatchObject({ state: 'invalid', code: 'no-root', root: null });
  });

  it('is ready when the canvas is whole and only the closing fence is missing or half typed', () => {
    for (const tail of ['', '\n', '\n``', '\n  ~']) {
      const { body, view } = settledView(`\`\`\`hollow-canvas\n${BODY}${tail}`);
      expect(view.state).toBe('ready');
      // The body the store would save is the canvas, not the half-typed fence.
      expect(body).toEqual({ code: BODY, closed: true });
    }
  });

  it('changes nothing while the reply streams or once the fence closed', () => {
    expect(settleCanvasBody('<hc-canvas id="a" title="A"><p>\n``', false, false)).toEqual({ code: '<hc-canvas id="a" title="A"><p>\n``', closed: false });
    expect(settleCanvasBody(`${BODY}\n`, true, true)).toEqual({ code: `${BODY}\n`, closed: true });
    expect(settleCanvasBody(`${BODY}\n`, true, false)).toEqual({ code: `${BODY}\n`, closed: true });
  });
});

describe('Ask to fix', () => {
  it('drafts a short request that quotes the error and names the canvas', () => {
    const text = canvasFixRequest({ id: 'release-status', title: 'Release status' }, 'ReferenceError: data is not defined');
    expect(text).toBe('The canvas "Release status" (id release-status) is broken: "ReferenceError: data is not defined". Please send a corrected hollow-canvas fence with the same id.');
  });

  it('works without a root, and keeps a long error to one bounded line', () => {
    const text = canvasFixRequest(null, `line one\n${'x'.repeat(1000)}`);
    expect(text.startsWith('The hollow-canvas fence in your last reply is broken: "line one xxx')).toBe(true);
    expect(text).not.toContain('\n');
    expect(text.length).toBeLessThan(420);
    expect(text.endsWith('Please send a corrected hollow-canvas fence.')).toBe(true);
  });
});

describe('the default mode', () => {
  it('opens html fences in Canvas in quick and workspace chats, Offline elsewhere', () => {
    expect(htmlFenceMode(true)).toBe('canvas');
    expect(htmlFenceMode(false)).toBe('offline');
  });
});

/** A fake store: counts calls and keeps one canvas per id, like the real one. */
function fakeStore(opts: { recordError?: string; changedOnce?: boolean; retired?: boolean } = {}) {
  const calls = { read: 0, record: 0 };
  const turnIds: (string | null | undefined)[] = [];
  const saved = new Map<string, Canvas>();
  let changedOnce = !!opts.changedOnce;
  const store: CanvasStoreCalls = {
    async readCanvas(_scope, id) { calls.read++; return saved.get(id) ?? null; },
    async recordCanvas(_scope, body, expected, recordOpts): Promise<CanvasRecorded> {
      calls.record++;
      turnIds.push(recordOpts?.turnId);
      if (opts.recordError) throw opts.recordError;
      if (changedOnce) { changedOnce = false; throw new Error('CANVAS_CHANGED: someone else saved'); }
      const sha = await sha256Hex(body);
      const id = /id="([^"]+)"/.exec(body)![1];
      const prior = saved.get(id);
      if (opts.retired && prior) return { outcome: 'retired', matchedRevision: 1, canvas: prior };
      const kept = prior?.revisions.find(r => r.fenceSha256 === sha);
      if (prior && kept) return { outcome: 'unchanged', matchedRevision: kept.revision, canvas: prior };
      if (expected !== (prior?.sha256 ?? null)) throw new Error('CANVAS_CHANGED: stale');
      const revision = (prior?.revision ?? 0) + 1;
      const canvas = {
        id, revision, fenceSha256: sha, html: body, sha256: `tok-${revision}`,
        revisions: [...(prior?.revisions ?? []), { revision, fenceSha256: sha }],
      } as unknown as Canvas;
      saved.set(id, canvas);
      return { outcome: prior ? 'revised' : 'created', matchedRevision: revision, canvas };
    },
  };
  return { store, calls, saved, turnIds };
}

const WS: ChatCanvasScope = { workspaceId: 'release', sessionKey: 's-1' };
const v2 = BODY.replace('29', '31');

describe('saving a chat canvas', () => {
  beforeEach(() => forgetRecordedCanvases());

  it('records once when the fence finishes during a live turn, and a remount reuses that record', async () => {
    const { store, calls } = fakeStore();
    const first = await resolveCanvas(store, WS, 'release-status', BODY, { live: true, kit: 'kit@1' });
    expect(first).toEqual({ body: BODY, revision: 1, latest: 1, saved: true, note: null, canvas: expect.objectContaining({ revision: 1, sha256: 'tok-1' }) });
    expect(calls.record).toBe(1);
    // The row remounts as the turn settles: same store, same body, no new call.
    await resolveCanvas(store, WS, 'release-status', BODY, { live: true });
    expect(calls.record).toBe(1);
  });

  it('names the turn the fence streamed in, so a curated save binds to that turn (C6 K1 review)', async () => {
    const { store, turnIds } = fakeStore();
    await resolveCanvas(store, WS, 'release-status', BODY, { live: true, kit: 'kit@1', turnId: 'run-n1' });
    await resolveCanvas(store, WS, 'release-status', v2, { live: true });
    expect(turnIds).toEqual(['run-n1', null]);
  });

  it('a re-render or a reopened conversation only reads', async () => {
    const { store, calls } = fakeStore();
    await resolveCanvas(store, WS, 'release-status', BODY, { live: true });
    await resolveCanvas(store, WS, 'release-status', v2, { live: true });
    const reads = calls.read;
    // The older fence renders from the transcript with its own number; the
    // newer one from the store's current revision.
    expect(await resolveCanvas(store, WS, 'release-status', BODY, { live: false })).toEqual({ body: BODY, revision: 1, latest: 2, saved: true, note: null, canvas: expect.objectContaining({ revision: 2 }) });
    expect(await resolveCanvas(store, WS, 'release-status', v2, { live: false })).toEqual({ body: v2, revision: 2, latest: 2, saved: true, note: null, canvas: expect.objectContaining({ revision: 2 }) });
    // A canvas the store never saw renders from the transcript, unsaved.
    expect(await resolveCanvas(store, WS, 'other', BODY.replace('release-status', 'other'), { live: false })).toMatchObject({ saved: false, revision: null });
    expect(calls.record).toBe(2);
    expect(calls.read).toBe(reads + 3);
  });

  it('a canvas saved before row escaping (overnight A2) still finds its revision from the bytes as written', async () => {
    const { store, calls } = fakeStore();
    const written = BODY.replace('<hc-stats>', '<hc-bars label="B">\nUnder <5: 12\n</hc-bars>\n<hc-stats>');
    await resolveCanvas(store, WS, 'release-status', written, { live: true });
    const escaped = written.replace('Under <5', 'Under &lt;5');
    expect(await resolveCanvas(store, WS, 'release-status', escaped, { live: false })).toMatchObject({ saved: false, revision: null });
    // It still draws the repaired bytes.
    expect(await resolveCanvas(store, WS, 'release-status', escaped, { live: false, written })).toMatchObject({ saved: true, revision: 1, body: escaped });
    expect(calls.record).toBe(1);
  });

  it('a live fence the turn\'s end already saved as written (question cards, 2026-10-14) is not saved again', async () => {
    const { store, calls } = fakeStore();
    const written = BODY.replace('<hc-stats>', '<hc-bars label="B">\nUnder <5: 12\n</hc-bars>\n<hc-stats>');
    // `record_reply` saved the fence as the agent wrote it before this window recorded.
    await resolveCanvas(store, WS, 'release-status', written, { live: true });
    forgetRecordedCanvases();
    const escaped = written.replace('Under <5', 'Under &lt;5');
    expect(await resolveCanvas(store, WS, 'release-status', escaped, { live: true, written })).toMatchObject({ saved: true, revision: 1, latest: 1, body: escaped });
    expect(calls.record).toBe(1);
  });

  it('a body the store retired renders from the transcript and is not saved', async () => {
    const { store } = fakeStore({ retired: true });
    await resolveCanvas(store, WS, 'release-status', BODY, { live: true });
    forgetRecordedCanvases();
    const old = await resolveCanvas(store, WS, 'release-status', v2, { live: true });
    expect(old).toMatchObject({ body: v2, saved: false, revision: 1 });
    expect(old.note).toMatch(/no longer kept/);
  });

  it('CANVAS_RETIRED (a pruned conversation) renders from the transcript and is not saved', async () => {
    const { store } = fakeStore({ recordError: 'CANVAS_RETIRED: pruned' });
    const r = await resolveCanvas(store, { workspaceId: null, sessionKey: 's-old' }, 'release-status', BODY, { live: true });
    expect(r).toMatchObject({ body: BODY, saved: false, revision: null });
    expect(r.note).toMatch(/cleared/);
  });

  it('a racing save is re-read and retried once', async () => {
    const { store, calls } = fakeStore({ changedOnce: true });
    const r = await resolveCanvas(store, WS, 'release-status', BODY, { live: true });
    expect(r.saved).toBe(true);
    expect(calls.record).toBe(2);
    expect(calls.read).toBe(2);
  });

  it('any other refusal renders the fence and says it was not saved', async () => {
    const { store } = fakeStore({ recordError: 'CANVAS_INVALID: The canvas id must be 1–64 lowercase letters' });
    const r = await resolveCanvas(store, WS, 'release-status', BODY, { live: true });
    expect(r).toMatchObject({ body: BODY, saved: false });
    expect(r.note).toBe('Not saved: The canvas id must be 1–64 lowercase letters');
  });

  it('a conversation with no store yet saves nothing', async () => {
    const { store, calls } = fakeStore();
    expect(await resolveCanvas(store, { workspaceId: null, sessionKey: null }, 'release-status', BODY, { live: true })).toMatchObject({ saved: false });
    expect(await resolveCanvas(store, null, 'release-status', BODY, { live: true })).toMatchObject({ saved: false });
    expect(calls).toEqual({ read: 0, record: 0 });
  });
});

/** A canvas as the store returns it, for the head and state tests. */
function storedCanvas(over: Partial<Canvas> = {}): Canvas {
  return {
    scope: 'workspace', workspaceId: 'release', sessionKey: 's-1', id: 'release-status', title: 'Release status', kit: 'kit@1',
    session: 's-1', fenceSha256: 'f2', createdAt: '2026-10-05T10:00:00Z', updatedAt: '2026-10-05T10:05:00Z', revision: 2,
    ask: false, height: null, html: BODY, sha256: 'tok-a', path: null, file: null, htmlChanged: false,
    state: null, stateSavedAt: null,
    revisions: [
      { revision: 1, fenceSha256: 'f1', title: 'Release status', savedAt: '2026-10-05T10:00:00Z', ask: false, height: null, session: 's-1', turnId: null },
      { revision: 2, fenceSha256: 'f2', title: 'Release status', savedAt: '2026-10-05T10:05:00Z', ask: false, height: null, session: 's-1', turnId: null },
    ],
    ...over,
  };
}

describe('canvas heads (the newest this window knows)', () => {
  const key = canvasKey(WS, 'release-status');

  it('keys by store and id', () => {
    expect(canvasKey(WS, 'a')).not.toBe(canvasKey({ workspaceId: 'release', sessionKey: null }, 'a'));
    expect(canvasKey(WS, 'a')).not.toBe(canvasKey(WS, 'b'));
  });

  it('a later revision or state save replaces the head; an older note never rolls it back', () => {
    const heads = new CanvasHeads();
    let told = 0;
    heads.subscribe(() => told++);
    heads.note(key, headOf(storedCanvas()));
    expect(heads.get(key)).toMatchObject({ revision: 2, token: 'tok-a', state: null });
    // A cached record or a stale read of revision 1 is ignored.
    heads.note(key, headOf(storedCanvas({ revision: 1, sha256: 'tok-old' })));
    expect(heads.get(key)?.token).toBe('tok-a');
    // A state saved later on the same revision replaces it, an earlier one doesn't.
    heads.note(key, headOf(storedCanvas({ sha256: 'tok-b', state: { tab: 'Table' }, stateSavedAt: '2026-10-05T10:06:00Z' })));
    heads.note(key, headOf(storedCanvas({ sha256: 'tok-c', state: { tab: 'Chart' }, stateSavedAt: '2026-10-05T10:05:30Z' })));
    expect(heads.get(key)).toMatchObject({ token: 'tok-b', state: { tab: 'Table' } });
    heads.note(key, headOf(storedCanvas({ revision: 3, sha256: 'tok-d', state: { tab: 'Table' }, stateSavedAt: '2026-10-05T10:06:00Z' })));
    expect(heads.get(key)?.revision).toBe(3);
    expect(told).toBe(3);
  });

  it('a stored state that is not a small plain object reads as none', () => {
    expect(headOf(storedCanvas({ state: [1, 2] })).state).toBeNull();
    expect(headOf(storedCanvas({ state: 'x' })).state).toBeNull();
  });
});

/** A fake store for state saves: one canvas, a sidecar token that changes on every write. */
function stateStore(opts: { canvas?: Canvas | null; changeBefore?: (c: Canvas) => void } = {}) {
  let canvas: Canvas | null = opts.canvas === undefined ? storedCanvas() : opts.canvas;
  let n = 0;
  const calls = { read: 0, save: 0, record: 0 };
  let changeBefore = opts.changeBefore;
  const store: CanvasStateCalls = {
    async readCanvas() { calls.read++; return canvas ? { ...canvas } : null; },
    async recordCanvas() { calls.record++; throw new Error('not used'); },
    async saveCanvasState(_scope, _id, state, expected) {
      calls.save++;
      if (!canvas) throw new Error('This canvas no longer exists.');
      if (changeBefore) { changeBefore(canvas); changeBefore = undefined; }
      if (expected !== canvas.sha256) throw new Error('CANVAS_CHANGED: the canvas changed since it was read');
      n++;
      canvas = { ...canvas, state, stateSavedAt: new Date(Date.UTC(2026, 9, 5, 11, 0, n)).toISOString(), sha256: `tok-s${n}` };
      return { sha256: canvas.sha256, stateSavedAt: canvas.stateSavedAt, revision: canvas.revision };
    },
  };
  return { store, calls, current: () => canvas };
}

describe('saving a canvas\'s state (compare-and-swap)', () => {
  const key = canvasKey(WS, 'release-status');
  const at = Date.UTC(2026, 9, 5, 12);

  it('saves with the head\'s token and moves the head to the new token', async () => {
    const heads = new CanvasHeads();
    const { store, calls, current } = stateStore();
    expect(await saveCanvasStateCas(store, heads, WS, 'release-status', { 'hc:hc-tabs:0': 'Table' }, at)).toBe('saved');
    expect(calls).toMatchObject({ read: 1, save: 1 });
    expect(current()?.state).toEqual({ 'hc:hc-tabs:0': 'Table' });
    expect(heads.get(key)).toMatchObject({ token: 'tok-s1', state: { 'hc:hc-tabs:0': 'Table' } });
    // The same state again writes nothing; a new one uses the new token.
    expect(await saveCanvasStateCas(store, heads, WS, 'release-status', { 'hc:hc-tabs:0': 'Table' }, at)).toBe('unchanged');
    expect(await saveCanvasStateCas(store, heads, WS, 'release-status', { 'hc:hc-tabs:0': 'Chart' }, at)).toBe('saved');
    expect(calls).toMatchObject({ read: 1, save: 2 });
  });

  it('a conflict re-reads; the newer write wins', async () => {
    // Another window saved a state after this change: keep theirs.
    const later = stateStore({ changeBefore: c => Object.assign(c, { sha256: 'tok-other', state: { tab: 'Theirs' }, stateSavedAt: new Date(at + 5_000).toISOString() }) });
    const heads = new CanvasHeads();
    expect(await saveCanvasStateCas(later.store, heads, WS, 'release-status', { tab: 'Mine' }, at)).toBe('kept-newer');
    expect(later.current()?.state).toEqual({ tab: 'Theirs' });
    expect(heads.get(key)).toMatchObject({ token: 'tok-other', state: { tab: 'Theirs' } });
    expect(later.calls.save).toBe(1);

    // Something older raced it (a new revision, an earlier state): retry once with the fresh token.
    const older = stateStore({ changeBefore: c => Object.assign(c, { sha256: 'tok-rev3', revision: 3, stateSavedAt: new Date(at - 60_000).toISOString() }) });
    const heads2 = new CanvasHeads();
    expect(await saveCanvasStateCas(older.store, heads2, WS, 'release-status', { tab: 'Mine' }, at)).toBe('saved');
    expect(older.current()).toMatchObject({ revision: 3, state: { tab: 'Mine' } });
    expect(older.calls.save).toBe(2);
  });

  it('does not apply an old frame edit to a new revision discovered during CAS retry', async () => {
    const changed = stateStore({ changeBefore: c => Object.assign(c, { sha256: 'new-revision', revision: 3 }) });
    const heads = new CanvasHeads();
    const revision = changed.current()!.revision;
    expect(await saveCanvasStateCas(changed.store, heads, WS, 'release-status', { tab: 'Old frame' }, at, revision)).toBe('kept-newer');
    expect(changed.calls.save).toBe(1);
    expect(changed.current()?.state).not.toEqual({ tab: 'Old frame' });
  });

  it('a canvas the store doesn\'t have, or any other refusal, saves nothing', async () => {
    const gone = stateStore({ canvas: null });
    expect(await saveCanvasStateCas(gone.store, new CanvasHeads(), WS, 'release-status', { a: 1 }, at)).toBe('failed');
    expect(gone.calls.save).toBe(0);
    const heads = new CanvasHeads();
    heads.note(key, headOf(storedCanvas()));
    const refused: CanvasStateCalls = { ...stateStore().store, saveCanvasState: async () => { throw new Error('CANVAS_INVALID: The UI state is over 16 KB.'); } };
    expect(await saveCanvasStateCas(refused, heads, WS, 'release-status', { a: 1 }, at)).toBe('failed');
  });
});

describe('the state saver (debounce)', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('retains a rejected save for an explicit retry with its original change time', async () => {
    const save = vi.fn().mockRejectedValueOnce(new Error('disk unavailable')).mockResolvedValue(undefined);
    const saver = canvasStateSaver(save);
    saver.push({ tab: 'Lanes' }, 123);
    await saver.flush();
    await saver.flush();
    expect(save.mock.calls).toEqual([[{ tab: 'Lanes' }, 123], [{ tab: 'Lanes' }, 123]]);
  });

  it('retains a failed CAS result and never restores it over a newer pending edit', async () => {
    let finish!: (result: string) => void;
    const save = vi.fn().mockImplementationOnce(() => new Promise<string>(resolve => { finish = resolve; })).mockResolvedValue('saved');
    const saver = canvasStateSaver(save);
    saver.push({ tab: 'Old' }, 1);
    const first = saver.flush();
    await Promise.resolve();
    saver.push({ tab: 'New' }, 2);
    finish('failed');
    await first;
    await saver.flush();
    expect(save.mock.calls).toEqual([[{ tab: 'Old' }, 1], [{ tab: 'New' }, 2]]);
    await saver.flush();
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('retries a resolved failed CAS result instead of dropping the edit', async () => {
    const save = vi.fn().mockResolvedValueOnce('failed').mockResolvedValue('saved');
    const saver = canvasStateSaver(save);
    saver.push({ open: true }, 42);
    await saver.flush();
    await saver.flush();
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('saves only the last state of a burst, one save at a time', async () => {
    const saves: unknown[] = [];
    const saver = canvasStateSaver(async (state) => { saves.push(state); }, 400);
    saver.push({ tab: 'A' });
    saver.push({ tab: 'B' });
    await vi.advanceTimersByTimeAsync(399);
    expect(saves).toEqual([]);
    saver.push({ tab: 'C' });
    await vi.advanceTimersByTimeAsync(400);
    expect(saves).toEqual([{ tab: 'C' }]);
  });

  it('flush saves what is pending now (unmount), and nothing twice', async () => {
    const saves: unknown[] = [];
    const saver = canvasStateSaver(async (state) => { saves.push(state); }, 400);
    saver.push({ open: true });
    await saver.flush();
    await vi.advanceTimersByTimeAsync(1000);
    await saver.flush();
    expect(saves).toEqual([{ open: true }]);
  });
});

describe('the version label', () => {
  it('names the shown revision, and the current one when it is not', () => {
    expect(versionLabel(3, 3)).toBe('v3');
    expect(versionLabel(1, 3)).toBe('v1 of 3');
    expect(versionLabel(1, null)).toBe('v1');
  });
});
