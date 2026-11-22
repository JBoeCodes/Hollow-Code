import { describe, expect, it } from 'vitest';
import { EditorState, type TransactionSpec } from '@codemirror/state';
import { carrier, inlineAi } from './editor/inlineAi';
import { MAX_SELECTION, acceptSpec, reanchor, replacementText, requestFor, setSuggestion, slashItems, slashStart, suggestionField, type Suggestion } from './inlineAiModel';

const state = (doc: string) => EditorState.create({ doc, extensions: [suggestionField] });
const suggestion = (over: Partial<Suggestion>): Suggestion => ({ id: 1, from: 0, to: 0, action: 'rewrite', status: 'ready', text: '', request: { action: 'rewrite', text: '' }, ...over });

describe('suggestion state', () => {
  it('follows its text through an edit before it, and goes when its text is deleted', () => {
    let s = state('one two three');
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 4, to: 7, text: 'TWO' })) }).state;
    s = s.update({ changes: { from: 0, insert: '>> ' } }).state;
    expect(s.field(suggestionField)).toMatchObject({ from: 7, to: 10 });
    s = s.update({ changes: { from: 6, to: 11 } }).state;
    expect(s.field(suggestionField)).toBeNull();
  });

  it('accepting replaces the text in one undoable step; nothing before it is ready', () => {
    let s = state('teh plan is good');
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 0, to: 8, status: 'pending' })) }).state;
    expect(acceptSpec(s)).toBeNull();
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 0, to: 8, text: 'the plan' })) }).state;
    s = s.update(acceptSpec(s)!).state;
    expect(s.doc.toString()).toBe('the plan is good');
    expect(s.field(suggestionField)).toBeNull();
    expect(s.selection.main.head).toBe(8);
  });

  it('an insertion at the cursor stays put when text is typed after it', () => {
    let s = state('ab');
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 1, to: 1, action: 'ask', text: 'X' })) }).state;
    s = s.update({ changes: { from: 2, insert: 'c' } }).state;
    expect(s.field(suggestionField)).toMatchObject({ from: 1, to: 1 });
    s = s.update(acceptSpec(s)!).state;
    expect(s.doc.toString()).toBe('aXbc');
  });
});

describe('review fixes', () => {
  it('keeps the selection’s own newline and indent around the reply', () => {
    expect(replacementText('  - teh item\n', '- the item')).toBe('  - the item\n');
    let s = state('line one\nnext line');
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 0, to: 9, text: 'Line one, better.' })) }).state;
    s = s.update(acceptSpec(s)!).state;
    expect(s.doc.toString()).toBe('Line one, better.\nnext line');
  });

  it('drops the suggestion when its own text is edited (the answer would be stale)', () => {
    let s = state('alpha beta gamma');
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 6, to: 10, status: 'pending' })) }).state;
    s = s.update({ changes: { from: 7, to: 8, insert: 'E' } }).state;
    expect(s.field(suggestionField)).toBeNull();
    s = s.update({ effects: setSuggestion.of(suggestion({ from: 6, to: 10, status: 'pending' })) }).state;
    s = s.update({ changes: { from: 8, insert: 'x' } }).state;
    expect(s.field(suggestionField)).toBeNull();
  });

  it('never cuts an emoji in half for context', () => {
    // 600 back from SEL lands on the emoji's second half.
    const doc = `${'a'.repeat(5)}😀${'b'.repeat(599)}SEL`;
    const s = state(doc);
    const from = doc.indexOf('SEL');
    const r = requestFor(s, from, from + 3, 'rewrite');
    if ('error' in r) throw new Error('unexpected');
    const first = r.context!.charCodeAt(0);
    expect(first >= 0xdc00 && first <= 0xdfff).toBe(false);
    expect(() => JSON.parse(JSON.stringify(r))).not.toThrow();
  });
});

describe('requestFor', () => {
  it('carries the text, the instruction and bounded context', () => {
    const s = state(`${'a'.repeat(2000)}MIDDLE${'b'.repeat(2000)}`);
    const r = requestFor(s, 2000, 2006, 'translate', 'French');
    expect(r).toMatchObject({ action: 'translate', text: 'MIDDLE', instruction: 'French' });
    if ('error' in r) throw new Error('unexpected');
    expect(r.context!.length).toBeLessThan(1300);
    expect(r.context).toContain('[…]');
  });
  it('refuses a selection over the limit', () => {
    const s = state('x'.repeat(MAX_SELECTION + 1));
    expect(requestFor(s, 0, MAX_SELECTION + 1, 'rewrite')).toMatchObject({ error: expect.stringContaining('too long') });
  });
});

describe('slash menu', () => {
  it('opens only for /word at the start of a line', () => {
    expect(slashStart('/')).toBe(0);
    expect(slashStart('  /tab')).toBe(2);
    expect(slashStart('see /tab')).toBeNull();
    expect(slashStart('/two words')).toBeNull();
  });
  it('offers task, table, callout, date, link and ask', () => {
    const items = slashItems('2026-10-11');
    expect(items.map(i => i.id)).toEqual(['task', 'table', 'callout', 'date', 'link', 'ask']);
    expect(items.find(i => i.id === 'date')!.insert).toBe('2026-10-11');
    expect(items.find(i => i.id === 'task')!.insert).toBe('- [ ] ');
    expect(items.find(i => i.id === 'ask')!.insert).toBeUndefined();
  });
});

describe('reanchor after the file changed on disk (overnight S3)', () => {
  const old = 'Intro line.\nThe plan ships Friday.\nOutro.';
  const from = old.indexOf('The plan');
  const s = suggestion({ from, to: from + 'The plan ships Friday.'.length, status: 'pending', requestId: 'r1' });
  it('finds the selected text where it moved, keeping the request', () => {
    const next = `A new first line.\n${old}`;
    const moved = reanchor(s, old, next)!;
    expect(next.slice(moved.from, moved.to)).toBe('The plan ships Friday.');
    expect(moved.requestId).toBe('r1');
    expect(moved.status).toBe('pending');
  });
  it('picks the occurrence nearest where it was', () => {
    const twice = 'The plan ships Friday.\n' + old;
    const moved = reanchor(s, old, twice)!;
    // Copies at 0 and 35; it was at 12, so the first.
    expect(moved.from).toBe(0);
    const tail = old + '\nThe plan ships Friday.';
    expect(reanchor(s, old, tail)!.from).toBe(from);
  });
  it('is gone (null) when its text changed', () => {
    expect(reanchor(s, old, old.replace('Friday', 'Monday'))).toBeNull();
  });
  it('an insertion follows the text around the cursor', () => {
    const at = old.indexOf('\nOutro');
    const ins = suggestion({ from: at, to: at, action: 'ask' });
    const next = `Top.\n${old}`;
    const moved = reanchor(ins, old, next)!;
    expect(moved.from).toBe(next.indexOf('\nOutro'));
    expect(moved.to).toBe(moved.from);
    expect(reanchor(ins, old, 'Something else entirely.')).toBeNull();
  });
});

describe('a pending request never leaves silently (overnight S3 review)', () => {
  const doc = 'Intro.\nThe plan ships Friday.\nOutro.';
  const from = doc.indexOf('The plan');
  const pending = suggestion({ id: 7, from, to: from + 'The plan ships Friday.'.length, status: 'pending', requestId: 'req-7' });
  const setup = (key: string) => {
    const cancelled: string[] = [];
    const host = { run: async () => '', cancel: (id: string) => { cancelled.push(id); }, ask: async () => null, today: () => '2026-10-13', key };
    let st = EditorState.create({ doc, extensions: [inlineAi(host)] });
    st = st.update({ effects: setSuggestion.of(pending) }).state;
    const view = { state: st, dispatch(spec: TransactionSpec) { view.state = view.state.update(spec).state; } };
    return { cancelled, view, host };
  };
  it('cancels the call when a whole-document replace maps the suggestion away, and says it expired', async () => {
    const { cancelled, view } = setup('w1|a.md');
    const plugin = carrier(view);
    const start = view.state;
    const tr = start.update({ changes: { from: 0, to: start.doc.length, insert: 'Rewritten elsewhere.' } });
    view.state = tr.state;
    expect(view.state.field(suggestionField)).toBeNull();
    plugin.update({ startState: start, state: tr.state, transactions: [tr] });
    expect(cancelled).toEqual(['req-7']);
    await Promise.resolve();
    expect(view.state.field(suggestionField)?.status).toBe('expired');
    plugin.destroy();
  });
  it('a pending suggestion carried on destroy is not cancelled, and re-attaches', async () => {
    const { cancelled, view } = setup('w2|a.md');
    carrier(view).destroy();
    expect(cancelled).toEqual([]);
    const next = setup('w2|a.md').view;
    next.state = next.state.update({ effects: setSuggestion.of(null) }).state;
    carrier(next);
    await Promise.resolve();
    expect(next.state.field(suggestionField)?.requestId).toBe('req-7');
    expect(cancelled).toEqual([]);
  });
});
