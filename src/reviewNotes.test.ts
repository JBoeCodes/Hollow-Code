import { describe, expect, it } from 'vitest';
import {
  BATCH_MAX_CHARS, MAX_NOTES_PER_FILE, MAX_NOTE_FILES, QUOTE_CHARS, REVIEW_NOTES_KEY,
  composeBatch, loadReviewNotes, makeNote, markSent, parseReviewNotes, pendingNotes, reanchor, saveReviewNotes, sameNotes, sendAgain, setResolved, splitLines, withNotes,
  type ReviewNote,
} from './reviewNotes';

const PLAN = ['# Plan', '', '## Goals', 'Ship the notes view', 'Keep it small', '', '## Risks', 'Long files', 'Stale lines'].join('\n');
const noteOn = (text: string, line: number, note = 'fix this', at = 1, id = `n${line}-${at}`) => makeNote('/v/plan.md', splitLines(text), line, note, at, id);

describe('splitLines', () => {
  it('drops one trailing newline and reads CRLF', () => {
    expect(splitLines('a\nb\n')).toEqual(['a', 'b']);
    expect(splitLines('a\r\nb')).toEqual(['a', 'b']);
    expect(splitLines('')).toEqual(['']);
  });
});

describe('makeNote', () => {
  it('keeps the trimmed line and its neighbours', () => {
    const n = noteOn(PLAN, 4, '  Say more  ');
    expect(n).toMatchObject({ line: 4, anchorText: 'Ship the notes view', before: '## Goals', after: 'Keep it small', text: 'Say more', resolved: false, lost: false });
  });
  it('is empty at the edges of the file', () => {
    expect(noteOn(PLAN, 1)).toMatchObject({ before: '', after: '' });
    expect(noteOn(PLAN, 9)).toMatchObject({ after: '' });
  });
});

describe('reanchor', () => {
  it('leaves an untouched file alone (same objects)', () => {
    const notes = [noteOn(PLAN, 4)];
    expect(sameNotes(reanchor(notes, PLAN), notes)).toBe(true);
  });
  it('follows a line when lines are inserted above it', () => {
    const [n] = reanchor([noteOn(PLAN, 4)], `Intro one\nIntro two\n${PLAN}`);
    expect(n).toMatchObject({ line: 6, lost: false });
  });
  it('follows a line when lines above it are deleted', () => {
    const [n] = reanchor([noteOn(PLAN, 8)], PLAN.split('\n').slice(3).join('\n'));
    expect(n).toMatchObject({ line: 5, lost: false, anchorText: 'Long files' });
  });
  it('follows a line that moved with both of its neighbours elsewhere in the file', () => {
    const moved = ['# Plan', '## Risks', 'Long files', 'Stale lines', '', '## Goals', 'Ship the notes view', 'Keep it small'].join('\n');
    expect(reanchor([noteOn(PLAN, 4)], moved)[0]).toMatchObject({ line: 7, lost: false });
  });
  it('marks a line edited in place as lost, keeping its old number and words', () => {
    const edited = PLAN.replace('Ship the notes view', 'Ship the review view');
    const [n] = reanchor([noteOn(PLAN, 4)], edited);
    expect(n).toMatchObject({ line: 4, lost: true, anchorText: 'Ship the notes view' });
  });
  it('marks a deleted line as lost, and finds it again when it comes back', () => {
    const without = PLAN.split('\n').filter(l => l !== 'Long files').join('\n');
    const lost = reanchor([noteOn(PLAN, 8)], without);
    expect(lost[0]).toMatchObject({ line: 8, lost: true });
    expect(reanchor(lost, PLAN)[0]).toMatchObject({ line: 8, lost: false });
  });
  it('chooses the nearest of several identical lines', () => {
    const text = ['- item', 'a', '- item', 'b', '- item', 'c', '- item'].join('\n');
    // Neighbours differ for each; the note was on line 5 ("b" above, "c" below).
    const n = noteOn(text, 5);
    expect(reanchor([n], `new\n${text}`)[0].line).toBe(6);
    // Neighbours changed on both sides: the nearest twin to the old line wins.
    const rewritten = ['- item', 'x', '- item', 'y', '- item', 'z', '- item'].join('\n');
    expect(reanchor([n], rewritten)[0]).toMatchObject({ line: 5, lost: false });
    const shifted = ['p', 'q', '- item', 'x', '- item', 'y', '- item'].join('\n');
    expect(reanchor([noteOn(text, 5)], shifted)[0].line).toBe(5);
  });
  it('prefers the twin whose neighbours match over the one on the same line', () => {
    const text = ['alpha', 'same', 'beta', 'gamma', 'same', 'delta'].join('\n');
    const n = noteOn(text, 2);
    const after = ['alpha', 'x', 'same', 'y', 'gamma', 'same', 'delta'].join('\n');
    // Line 2 is now "x", so "same" is on lines 3 and 6; neither neighbours match line 3 fully: nearest wins.
    expect(reanchor([n], after)[0].line).toBe(3);
    const twin = ['q', 'same', 'x', 'alpha', 'same', 'beta'].join('\n');
    // Line 2 still reads "same" but both neighbours changed, while line 5 has both intact.
    expect(reanchor([n], twin)[0].line).toBe(5);
  });
  it('does not follow a bare blank line without a matching neighbour', () => {
    const n = noteOn(PLAN, 2);
    expect(reanchor([n], '# Other\n\nNothing\nelse\n\nhere')[0]).toMatchObject({ lost: true, line: 2 });
    expect(reanchor([n], `top\n${PLAN}`)[0]).toMatchObject({ lost: false, line: 3 });
  });
  it('updates the neighbours it remembers after an edit beside the line', () => {
    const [n] = reanchor([noteOn(PLAN, 4)], PLAN.replace('Keep it small', 'Keep it tiny'));
    expect(n).toMatchObject({ line: 4, lost: false, after: 'Keep it tiny' });
  });
});

describe('composeBatch', () => {
  it('makes one message from every unresolved note, in file order', () => {
    const notes = [noteOn(PLAN, 8, 'Say how long'), noteOn(PLAN, 4, 'Be concrete\nlist the steps')];
    const batch = composeBatch('/v/plan.md', notes);
    expect(batch.included).toHaveLength(2);
    expect(batch.omitted).toBe(0);
    expect(batch.text).toBe([
      'I reviewed /v/plan.md and left 2 notes on it. Each one names the line and what it said.',
      '',
      'Line 4: "Ship the notes view"\n  Be concrete\n  list the steps',
      '',
      'Line 8: "Long files"\n  Say how long',
      '',
      'Please make the changes and send back a revised version of the file.',
    ].join('\n'));
  });
  it('never sends a resolved note, and sends nothing when none is left', () => {
    const done = { ...noteOn(PLAN, 4), resolved: true };
    const open = noteOn(PLAN, 8);
    const batch = composeBatch('/v/plan.md', [done, open]);
    expect(batch.text).toContain('one note');
    expect(batch.text).not.toContain('Ship the notes view');
    expect(composeBatch('/v/plan.md', [done])).toEqual({ text: '', included: [], omitted: 0 });
  });
  it('quotes a long line cut to 120 characters on one line', () => {
    const long = `${'word '.repeat(60)}\tend`;
    const text = composeBatch('/v/a.md', [noteOn(long, 1)]).text;
    const quoted = /Line 1: "(.*)"/.exec(text)![1];
    expect(quoted.endsWith('…')).toBe(true);
    expect(quoted.length).toBe(QUOTE_CHARS + 1);
  });
  it('says when a quoted line may have moved', () => {
    const lost = { ...noteOn(PLAN, 4), lost: true };
    expect(composeBatch('/v/plan.md', [lost]).text).toContain('Line 4 (it may have moved or changed since): "Ship the notes view"');
  });
  it('keeps to the cap and counts what it left out', () => {
    const notes = Array.from({ length: 30 }, (_, i) => noteOn(PLAN, 4, `${i} ${'x'.repeat(1000)}`, i, `id${i}`));
    const batch = composeBatch('/v/plan.md', notes);
    expect(batch.text.length).toBeLessThanOrEqual(BATCH_MAX_CHARS);
    expect(batch.included.length).toBeGreaterThan(0);
    expect(batch.included.length + batch.omitted).toBe(30);
    expect(batch.omitted).toBeGreaterThan(0);
    expect(batch.text).toContain(`left ${batch.included.length} notes`);
  });
  it('always sends the first note even when it alone passes a tiny cap', () => {
    const batch = composeBatch('/v/plan.md', [noteOn(PLAN, 4, 'x'.repeat(500))], 100);
    expect(batch.included).toHaveLength(1);
    expect(batch.text.length).toBeLessThanOrEqual(100);
  });
});

describe('send state', () => {
  const a = noteOn(PLAN, 4, 'one', 1, 'a');
  const b = noteOn(PLAN, 8, 'two', 2, 'b');
  it('a send marks its notes, and the next send carries only the rest', () => {
    const sent = markSent([a, b], new Set(['a']), 9);
    expect(sent[0].sentAt).toBe(9);
    expect(pendingNotes(sent).map(n => n.id)).toEqual(['b']);
  });
  it('resolved notes are not pending; reopening one makes it pending again', () => {
    const done = setResolved(markSent([a, b], new Set(['a', 'b']), 9), 'a', true);
    expect(pendingNotes(done)).toEqual([]);
    const back = setResolved(done, 'a', false);
    expect(back[0]).toMatchObject({ resolved: false });
    expect(back[0].sentAt).toBeUndefined();
    expect(pendingNotes(back).map(n => n.id)).toEqual(['a']);
  });
  it('send again returns a sent note to the next send', () => {
    const sent = markSent([a], new Set(['a']), 9);
    expect(pendingNotes(sendAgain(sent, 'a')).map(n => n.id)).toEqual(['a']);
  });
});

describe('storage', () => {
  const memory = () => { const m = new Map<string, string>(); return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m }; };

  it('round-trips a file’s notes under the versioned key', () => {
    const s = memory();
    const notes = [noteOn(PLAN, 4), { ...noteOn(PLAN, 8), resolved: true, sentAt: 5 }];
    saveReviewNotes('/v/plan.md', notes, 10, s);
    expect(REVIEW_NOTES_KEY).toBe('hollow.reviewNotes.v1');
    expect(s.m.has(REVIEW_NOTES_KEY)).toBe(true);
    expect(loadReviewNotes('/v/plan.md', s)).toEqual(notes);
    expect(loadReviewNotes('/v/other.md', s)).toEqual([]);
  });
  it('removes a file once its last note is deleted', () => {
    const s = memory();
    saveReviewNotes('/v/plan.md', [noteOn(PLAN, 4)], 10, s);
    saveReviewNotes('/v/plan.md', [], 11, s);
    expect(parseReviewNotes(s.getItem(REVIEW_NOTES_KEY)).files).toEqual([]);
  });
  it('tolerates corrupt storage', () => {
    for (const raw of [null, '', 'not json', '42', '"x"', '{}', '{"files":5}', '{"files":[null,3,{"path":"/a"},{"path":"/b","notes":"x"}]}']) {
      expect(parseReviewNotes(raw)).toEqual({ files: [] });
    }
  });
  it('drops single bad notes and keeps the good ones', () => {
    const good = noteOn(PLAN, 4);
    const raw = JSON.stringify({ files: [{ path: '/a', updatedAt: 1, notes: [good, null, { id: 'x', line: 0, text: 'no' }, { id: 'y', line: 3, text: '   ' }, { id: 'z', line: 2.7, text: 'ok', anchorText: 5, resolved: 'yes' }] }] });
    const notes = parseReviewNotes(raw).files[0].notes;
    expect(notes.map(n => n.id)).toEqual([good.id, 'z']);
    expect(notes[1]).toMatchObject({ line: 2, anchorText: '', resolved: false, path: '/a' });
  });
  it('survives a storage that throws', () => {
    const broken = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('full'); } };
    expect(loadReviewNotes('/a', broken)).toEqual([]);
    expect(() => saveReviewNotes('/a', [noteOn(PLAN, 4)], 1, broken)).not.toThrow();
  });
  it('keeps the newest notes per file and the newest files', () => {
    const many: ReviewNote[] = Array.from({ length: MAX_NOTES_PER_FILE + 5 }, (_, i) => noteOn(PLAN, 4, `n${i}`, i, `id${i}`));
    const kept = withNotes({ files: [] }, '/v/a.md', many, 1).files[0].notes;
    expect(kept).toHaveLength(MAX_NOTES_PER_FILE);
    expect(kept.some(n => n.id === 'id0')).toBe(false);
    expect(kept.some(n => n.id === `id${MAX_NOTES_PER_FILE + 4}`)).toBe(true);
    let store = { files: [] as ReturnType<typeof withNotes>['files'] };
    for (let i = 0; i < MAX_NOTE_FILES + 3; i++) store = withNotes(store, `/v/${i}.md`, [noteOn(PLAN, 4)], i + 1);
    expect(store.files).toHaveLength(MAX_NOTE_FILES);
    expect(store.files.some(f => f.path === '/v/0.md')).toBe(false);
    expect(store.files.some(f => f.path === `/v/${MAX_NOTE_FILES + 2}.md`)).toBe(true);
  });
});
