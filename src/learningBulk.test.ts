import { describe, expect, it } from 'vitest';
import type { LearningBatchOutcome } from './generated/ipc';
import type { ReviewInboxItem } from './types';
import {
  BATCH_MAX, REJECT_REASON_MAX, batchMessage, failureDetail, failures, isBulkLearning, keepPending, mayRepeat, pendingLearningIds, rejectReason,
  selectAllIds, selectAllState, toggleId, undoLastLabel, undoMessage, batchAfterUndo, nothingLeftToUndo, offScreenAccept, autoSaveUndoBatch, learningsWaiting, opensInLearnings,
  editedWords, pinEditVersion, versionForSave,
} from './learningBulk';

function item(id: string, over: Partial<ReviewInboxItem> = {}): ReviewInboxItem {
  return {
    id, kind: 'learning', status: 'pending', createdAt: '2026-10-11T09:00:00Z', updatedAt: '2026-10-11T09:00:00Z', fingerprint: id,
    payload: { subject: id, statement: `${id} statement` }, risk: 'local_write', source: 'chat', sourcePath: null, snoozedUntil: null, error: null,
    ...over,
  };
}

const outcome = (action: string, results: Array<[string, string | null]>): LearningBatchOutcome => ({
  batchId: results.some(([, error]) => !error) ? 'learning-batch-1' : null,
  action,
  items: results.map(([id, error]) => ({ id, ok: !error, status: error ? null : action === 'accept' ? 'accepted' : 'rejected', error })),
});

describe('bulk learning review', () => {
  it('selects only pending learnings, across the whole queue', () => {
    const items = [
      item('a'),
      item('b', { status: 'accepted' }),
      item('c', { kind: 'commitment' }),
      item('d', { status: 'snoozed' }),
      item('e', { risk: 'external' as ReviewInboxItem['risk'] }),
      item('f'),
    ];
    expect(pendingLearningIds(items)).toEqual(['a', 'f']);
    expect(pendingLearningIds(undefined)).toEqual([]);
    expect(isBulkLearning(undefined)).toBe(false);
  });

  it('toggles, prunes to what is still pending, and reads Select all', () => {
    let selected = toggleId(new Set(), 'a');
    selected = toggleId(selected, 'b');
    expect([...selected]).toEqual(['a', 'b']);
    expect([...toggleId(selected, 'a')]).toEqual(['b']);
    const pending = ['a', 'b', 'c'];
    expect(selectAllState(new Set(), pending)).toBe('none');
    expect(selectAllState(selected, pending)).toBe('some');
    expect(selectAllState(new Set(pending), pending)).toBe('all');
    // A learning resolved since (here b) leaves the selection.
    expect([...keepPending(new Set(['a', 'b', 'x']), ['a', 'c'])]).toEqual(['a']);
    expect(selectAllState(new Set(['x']), [])).toBe('none');
  });

  it('caps Select all at one batch, and still reads it as all', () => {
    const many = Array.from({ length: BATCH_MAX + 20 }, (_, i) => `id-${i}`);
    const all = selectAllIds(many);
    expect(all.size).toBe(BATCH_MAX);
    expect(all.has('id-0') && !all.has(`id-${BATCH_MAX}`)).toBe(true);
    expect(selectAllState(all, many)).toBe('all');
    expect(selectAllIds(['a', 'b']).size).toBe(2);
  });

  it('counts the selected learnings the duplicate check flagged, keeping them selectable', () => {
    const items = [
      item('a', { payload: { subject: 'a', statement: 's', possibleDuplicate: 'Always use Outlook.' } }),
      item('b'),
      item('c', { payload: { subject: 'c', statement: 's', possibleDuplicate: '  ' } }),
      item('d', { status: 'accepted', payload: { subject: 'd', statement: 's', possibleDuplicate: 'x' } }),
    ];
    expect(pendingLearningIds(items)).toEqual(['a', 'b', 'c']);
    expect(mayRepeat(items, new Set(['a', 'b', 'c', 'd']))).toBe(1);
    expect(mayRepeat(items, new Set(['b']))).toBe(0);
    expect(mayRepeat(undefined, new Set(['a']))).toBe(0);
  });

  it('names failures by subject in the details, never by review id', () => {
    const partial = outcome('accept', [['review-1', null], ['review-2', 'gone'], ['review-3', 'stale']]);
    const titles: Record<string, string> = { 'review-2': 'Email platform' };
    expect(failureDetail(partial, id => titles[id])).toBe('Email platform: gone\nA learning: stale');
    expect(failureDetail(outcome('accept', [['review-1', null]]), () => 'x')).toBeUndefined();
  });

  it('trims the reason, sends blank as none and refuses one too long', () => {
    expect(rejectReason('  too generic ')).toEqual({ reason: 'too generic', problem: null });
    expect(rejectReason('   ')).toEqual({ reason: undefined, problem: null });
    expect(rejectReason('é'.repeat(REJECT_REASON_MAX)).problem).toBeNull();
    expect(rejectReason('x'.repeat(REJECT_REASON_MAX + 1)).problem).toMatch(/500/);
  });

  it('names partial failures instead of reading a batch as all-or-nothing', () => {
    expect(batchMessage(outcome('accept', [['a', null], ['b', null]]))).toEqual({ text: 'Accepted 2 learnings', failed: false });
    expect(batchMessage(outcome('reject', [['a', null]])).text).toBe('Rejected one learning');
    const partial = outcome('accept', [['a', null], ['b', 'learning target does not exist: people/J.md'], ['c', 'review item c is rejected']]);
    expect(failures(partial).map(f => f.id)).toEqual(['b', 'c']);
    expect(batchMessage(partial)).toEqual({
      text: '× Accepted 1 of 3 learnings; 2 couldn’t be: learning target does not exist: people/J.md (and 1 more)',
      failed: true,
    });
    expect(batchMessage(outcome('reject', [['a', 'gone']])).text).toBe('× Couldn’t reject one learning: gone');
  });

  it('says what Undo put back, and what it left', () => {
    expect(undoMessage(outcome('undo', [['a', null], ['b', null]])).text).toBe('Undone: 2 learnings back in Needs you');
    const left = undoMessage(outcome('undo', [['a', null], ['b', 'learnings/x.md isn’t the note this proposal wrote; left in place']]));
    expect(left.failed).toBe(true);
    expect(left.text).toMatch(/^× Undid 1 of 2; 1 stayed as they were: learnings/);
  });
});

describe('after an Undo of the last batch (A06 review)', () => {
  const batch = { id: 'review-batch-1', action: 'reject' as const, count: 2, snackbar: 's_1' };
  it('forgets a batch that went back whole, and keeps the ones a partial Undo left', () => {
    expect(batchAfterUndo(batch, outcome('undo', [['a', null], ['b', null]]))).toBeNull();
    expect(batchAfterUndo(batch, outcome('undo', [['a', null], ['b', 'changed since']]))).toEqual({ ...batch, count: 1, stalled: false });
  });
  it('lets go after a second Undo in a row that put nothing back', () => {
    const stuck = outcome('undo', [['b', 'isn’t the note this proposal wrote; left in place']]);
    const once = batchAfterUndo({ ...batch, count: 1 }, stuck);
    expect(once).toEqual({ ...batch, count: 1, stalled: true });
    expect(batchAfterUndo(once!, stuck)).toBeNull();
  });
  it('lets go only when nothing is left to undo; any other error keeps the button', () => {
    expect(nothingLeftToUndo('nothing to undo: bulk review review-batch-1 was already undone')).toBe(true);
    expect(nothingLeftToUndo(new Error('nothing to undo: bulk review x was already undone'))).toBe(true);
    expect(nothingLeftToUndo('mock undo_learning_batch failure')).toBe(false);
    expect(nothingLeftToUndo(new Error('disk full'))).toBe(false);
  });
});

describe('the pending signal outside Learnings (J7 review)', () => {
  it('counts pending learnings for the Library tab, and none when there are none', () => {
    const items = [item('a'), item('b'), item('c', { status: 'accepted' }), item('d', { kind: 'commitment' }), item('e', { status: 'snoozed' })];
    expect(learningsWaiting(items, false)).toBe(2);
    expect(learningsWaiting([item('c', { status: 'rejected' })], false)).toBeUndefined();
    expect(learningsWaiting([], false)).toBeUndefined();
  });
  it('shows no number while the queue is unread or its read failed', () => {
    expect(learningsWaiting(undefined, false)).toBeUndefined();
    expect(learningsWaiting([item('a')], true)).toBeUndefined();
  });
  it('routes a learning row to Library Learnings and every other row as before', () => {
    expect(opensInLearnings({ kind: 'review', category: 'learning' })).toBe(true);
    expect(opensInLearnings({ kind: 'review', category: 'commitment' })).toBe(false);
    expect(opensInLearnings({ kind: 'failure', category: 'learning' })).toBe(false);
    expect(opensInLearnings({ kind: 'review', category: null })).toBe(false);
  });
});

describe('undoLastLabel', () => {
  it('names the batch: how many, and what was done to them', () => {
    expect(undoLastLabel('reject', 2)).toBe('Undo last batch (2 learnings rejected)');
    expect(undoLastLabel('accept', 12)).toBe('Undo last batch (12 learnings accepted)');
  });
  it('says one learning in the singular', () => {
    expect(undoLastLabel('reject', 1)).toBe('Undo last batch (1 learning rejected)');
    expect(undoLastLabel('accept', 1)).toBe('Undo last batch (1 learning accepted)');
  });
});

describe('autoSaveUndoBatch (review 2026-11-08)', () => {
  it('offers a per-item Undo only for that item\'s own automatic save', () => {
    expect(autoSaveUndoBatch('review-1', 'learning-batch-auto-review-1')).toBe('learning-batch-auto-review-1');
    // A bulk Accept's batch holds other learnings: no per-card Undo.
    expect(autoSaveUndoBatch('review-1', 'learning-batch-0123456789abcdef0123')).toBeNull();
    // Another item's automatic batch is not this card's.
    expect(autoSaveUndoBatch('review-1', 'learning-batch-auto-review-2')).toBeNull();
    expect(autoSaveUndoBatch('review-1', null)).toBeNull();
    expect(autoSaveUndoBatch('review-1', undefined)).toBeNull();
  });
});

describe('editedWords (PR #1239 review)', () => {
  // A row's Save is a one-item batch with these words, so an edited Save
  // has the same Undo as any other (a reworded saved line gets its words back).
  it('sends the edit only when it changed the words, case included', () => {
    expect(editedWords(null, 'Use redis.')).toBeUndefined();
    expect(editedWords('Use redis.', 'Use redis.')).toBeUndefined();
    expect(editedWords('Use Redis.', 'Use redis.')).toBe('Use Redis.');
    expect(editedWords('Use Valkey.', 'Use redis.')).toBe('Use Valkey.');
  });
});

describe('offScreenAccept (P0-10)', () => {
  const subject = (id: string) => ({ a: 'Release checks', b: 'Weekly status' } as Record<string, string>)[id];
  it('asks nothing when every selected learning is on screen', () => {
    expect(offScreenAccept(['a', 'b'], new Set(['a', 'b', 'c']), subject)).toBeNull();
  });
  it('names the selected learnings that are not on screen before saving', () => {
    const ask = offScreenAccept(['a', 'b', 'z'], new Set(['a']), subject)!;
    expect(ask.title).toBe('Accept 3 learnings?');
    expect(ask.body).toBe('2 of them aren’t on screen: “Weekly status”, “z”.');
    expect(ask.confirmLabel).toBe('Accept 3');
  });
  it('lists at most five, then a count', () => {
    const ids = ['1', '2', '3', '4', '5', '6', '7'];
    expect(offScreenAccept(ids, new Set(), () => undefined)!.body).toMatch(/“5”, and 2 more\.$/);
  });
});

// ME1: a row's edit draft lives in a module store that outlives a reload; the
// item under it is swapped for the refreshed proposal mid-edit. The Save must
// send the version the edit began at, not the refreshed one.
describe('the version a Save sends', () => {
  it('stays the one the edit began at when the item refreshes mid-edit', () => {
    let shown = 'v-1';
    let draft: { edited: string | null; editedAt?: string } = { edited: null };
    const type = (words: string) => { draft = { edited: words, editedAt: pinEditVersion(draft.editedAt, shown) }; };
    type('Use Outlook');
    shown = 'v-2'; // a reload swaps in the refreshed proposal
    type('Use Outlook for mail');
    expect(draft.editedAt).toBe('v-1');
    expect(versionForSave(draft.editedAt, shown)).toBe('v-1');
  });

  it('is the version shown now when nothing was edited', () => {
    expect(versionForSave(undefined, 'v-2')).toBe('v-2');
    expect(pinEditVersion(undefined, 'v-3')).toBe('v-3');
  });
});
