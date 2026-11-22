import { describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
// @ts-ignore: an untyped path import of the browser build (see its case below)
import * as browserStore from '../node_modules/solid-js/store/dist/store.js';
import { createTranscript } from './chatTranscript';
import type { ChatTurn } from './chatTurnTypes';

// Moved from store/chatEngine.test.ts when the Main agent drawer's chat
// engine was removed (run 15 P3); the transcript is the chat's own.
const createBrowserStore = (browserStore as typeof import('solid-js/store')).createStore;

describe('createTranscript (moved out of runtime.ts, behaviour pinned)', () => {
  it('appends text to the open assistant turn until a boundary starts a new one', () => {
    const [turns, setTurns] = createStore<ChatTurn[]>([]);
    const t = createTranscript(setTurns, () => 'run-1', () => turns);
    t.appendText('one');
    t.appendText('two');
    expect(turns.map(x => x.text)).toEqual(['one\n\ntwo']);
    t.markBoundary();
    t.appendText('three');
    expect(turns.map(x => [x.text, x.runId])).toEqual([['one\n\ntwo', 'run-1'], ['three', 'run-1']]);
  });

  it('streams deltas into a partial turn and finalize settles it and running tools', () => {
    const [turns, setTurns] = createStore<ChatTurn[]>([]);
    const t = createTranscript(setTurns, () => undefined, () => turns);
    t.appendDelta('Hel');
    t.appendDelta('lo');
    t.appendToolUse('Read', { file_path: '/a' }, 'tu-1');
    expect(turns[0]).toMatchObject({ text: 'Hello', partial: true, tools: [{ name: 'Read', status: 'running', id: 'tu-1' }] });
    t.finalize();
    expect(turns[0].partial).toBe(false);
    expect(turns[0].tools[0].status).toBe('done');
  });

  it('keeps a subagent call\'s parent id on its row and none on a main-thread call (AG7)', () => {
    const [turns, setTurns] = createStore<ChatTurn[]>([]);
    const t = createTranscript(setTurns, () => 'run-1', () => turns);
    t.appendToolUse('Task', {}, 'tu-task');
    t.appendToolUse('Read', {}, 'tu-sub', undefined, 'tu-task');
    expect(turns[0].tools[0]).not.toHaveProperty('parentToolUseId');
    expect(turns[0].tools[1]).toMatchObject({ id: 'tu-sub', parentToolUseId: 'tu-task' });
  });

  it('a streamed delta touches only the reply it extends, not the rest of the conversation (run 15 S7)', () => {
    // In the app (Solid's browser store), a one-argument
    // setTurns(produce(...)) on an array store unwraps every turn
    // (Object.getOwnPropertyDescriptors per object) on each call: a long chat
    // paid that once per streamed delta. Vitest resolves the server build,
    // whose plain-object store never walks, so this case loads the browser one.
    const history: ChatTurn[] = Array.from({ length: 300 }, (_, i) => ({ role: i % 2 ? 'assistant' : 'user', text: `turn ${i}`, tools: [{ name: 'Read', input: { file_path: `/f${i}` }, id: `t${i}`, status: 'done' }] }));
    const [turns, setTurns] = createBrowserStore<ChatTurn[]>(history);
    const t = createTranscript(setTurns, () => 'run-9', () => turns);
    t.appendDelta('Hel');
    expect(turns).toHaveLength(301);
    expect(turns[300]).toMatchObject({ role: 'assistant', text: 'Hel', partial: true, runId: 'run-9' });
    const walk = vi.spyOn(Object, 'getOwnPropertyDescriptors');
    try {
      for (const d of ['lo', ', ', 'world']) t.appendDelta(d);
      expect(walk.mock.calls.length).toBeLessThan(10);
    } finally { walk.mockRestore(); }
    expect(turns[300].text).toBe('Hello, world');
    expect(turns).toHaveLength(301);
    // A boundary still starts a new bubble, and a finished reply is never extended.
    t.markBoundary();
    t.appendDelta('Next');
    expect(turns.slice(-2).map(x => [x.text, x.partial])).toEqual([['Hello, world', true], ['Next', true]]);
    t.finalize();
    t.appendDelta('After');
    expect(turns.slice(-2).map(x => [x.text, x.partial])).toEqual([['Next', false], ['After', true]]);
  });

  it('tool events update only their target in a long browser-store conversation', () => {
    const history: ChatTurn[] = Array.from({ length: 1000 }, (_, i) => ({
      role: 'assistant', text: `history ${i}`, tools: [{ name: 'Read', id: `old-${i}`, status: 'done' }],
    }));
    const [turns, setTurns] = createBrowserStore<ChatTurn[]>(history);
    const t = createTranscript(setTurns, () => 'current', () => turns);
    const walk = vi.spyOn(Object, 'getOwnPropertyDescriptors');
    try {
      t.markBoundary();
      t.appendToolUse('Read', {}, 'first');
      t.appendToolUse('Search', {}, 'second');
      t.applyToolResult('second', 'second finished first', false);
      t.markBoundary();
      t.appendDelta('Answer');
      t.applyToolResult('first', 'first finished later', true);
      t.applyToolImages('first', ['data:image/png;base64,x']);
      t.toggleToolExpanded(1000, 0);
      t.appendText('complete');
      expect(walk.mock.calls.length).toBeLessThan(50);
      t.finalize();
    } finally { walk.mockRestore(); }
    expect(turns).toHaveLength(1002);
    expect(turns[999].text).toBe('history 999');
    expect(turns[1000].tools).toMatchObject([
      { id: 'first', result: 'first finished later', status: 'error', expanded: true, images: ['data:image/png;base64,x'] },
      { id: 'second', result: 'second finished first', status: 'done' },
    ]);
    expect(turns[1001]).toMatchObject({ text: 'Answer\n\ncomplete', partial: false, runId: 'current' });
  });

  it('matches a tool result by id, or the newest running tool without one, and attaches images', () => {
    const [turns, setTurns] = createStore<ChatTurn[]>([]);
    const t = createTranscript(setTurns, () => undefined, () => turns);
    t.appendToolUse('A', {}, 'a');
    t.appendToolUse('B', {});
    t.applyToolResult('a', 'ok', false);
    t.applyToolResult('', 'boom', true);
    t.applyToolImages('a', ['data:image/png;base64,x']);
    expect(turns[0].tools.map(x => [x.name, x.status, x.result, x.images?.length ?? 0])).toEqual([['A', 'done', 'ok', 1], ['B', 'error', 'boom', 0]]);
  });

  it('puts a receipt on the run’s assistant turn, or on a bare one when the run wrote nothing', () => {
    const [turns, setTurns] = createStore<ChatTurn[]>([{ role: 'user', text: 'q', tools: [] }]);
    const t = createTranscript(setTurns, () => 'r', () => turns);
    const receipt = { status: 'success' } as unknown as Parameters<typeof t.applyReceipt>[0];
    t.applyReceipt(receipt, 'r', 1);
    expect(turns[1]).toMatchObject({ role: 'assistant', text: '', runId: 'r' });
    // A later run's prose turn takes its own receipt; no new bubble.
    t.markBoundary();
    t.appendText('answer');
    expect(turns).toHaveLength(3);
    t.applyReceipt(receipt, 'r', 2);
    expect(turns).toHaveLength(3);
    expect(turns[2].receipt).toBeDefined();
  });
});


it('replacing a receipt drops old optional fields without changing its run identity', () => {
  const [turns, setTurns] = createBrowserStore<ChatTurn[]>([{ role: 'assistant', text: 'Done', tools: [], runId: 'original' }]);
  const t = createTranscript(setTurns, () => 'new', () => turns);
  const receipt: Parameters<typeof t.applyReceipt>[0] = { status: 'verified', ordering: 'known', files: [], checks: [], failures: [], eventCount: 0 };
  t.applyReceipt({ ...receipt, status: 'failed', unattributedFileCount: 2 }, 'new', 0);
  t.applyReceipt(receipt, 'new', 0);
  expect(turns[0].receipt).toEqual(receipt);
  expect(turns[0].receipt).not.toHaveProperty('unattributedFileCount');
  expect(turns[0].runId).toBe('original');
});
