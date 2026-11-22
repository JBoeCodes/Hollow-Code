import { describe, expect, it } from 'vitest';
import { canVote, replyFinishedAt, replyRowButtons, replyTime, voteErrorText, voteInput, voteSavedText } from './replyActions';
import { fileChips } from './turnFooter';
import type { ExecutionReceipt } from './executionReceipt';

describe('the row under a reply (overnight 11-12, C2)', () => {
  it('orders its buttons: Copy, the thumbs, Read aloud, Retry, More', () => {
    expect(replyRowButtons({ canVote: true, canRead: true, canRetry: true, hasMore: true }))
      .toEqual(['copy', 'useful', 'notUseful', 'read', 'retry', 'more']);
    expect(replyRowButtons({ canVote: false, canRead: false, canRetry: false, hasMore: false })).toEqual(['copy']);
    expect(replyRowButtons({ canVote: true, canRead: false, canRetry: false, hasMore: true })).toEqual(['copy', 'useful', 'notUseful', 'more']);
  });

  it('offers a vote only on a settled reply with words and an exact run id', () => {
    const reply = { role: 'assistant', text: 'Done.', runId: 'r1' };
    expect(canVote(reply, null)).toBe(true);
    expect(canVote(reply, 'r2')).toBe(true);
    expect(canVote(reply, 'r1')).toBe(false);
    expect(canVote({ ...reply, partial: true }, null)).toBe(false);
    expect(canVote({ ...reply, text: '  ' }, null)).toBe(false);
    expect(canVote({ ...reply, runId: undefined }, null)).toBe(false);
    expect(canVote({ ...reply, role: 'user' }, null)).toBe(false);
  });

  it('says when it finished: the time today, the weekday this week, else the date', () => {
    const now = new Date(2026, 10, 11, 22, 43).getTime(); // Wednesday
    const at = (d: number, h: number, m: number, y = 2026, mo = 10) => new Date(y, mo, d, h, m).toISOString();
    expect(replyTime(at(11, 9, 5), now, 'en-US')).toBe('9:05 AM');
    expect(replyTime(at(8, 23, 27), now, 'en-US')).toBe('Sunday 11:27 PM');
    expect(replyTime(at(10, 0, 1), now, 'en-US')).toBe('Tuesday 12:01 AM');
    // Seven days back would repeat today's weekday: a date instead.
    expect(replyTime(at(4, 18, 0), now, 'en-US')).toBe('Nov 4, 6:00 PM');
    expect(replyTime(at(5, 8, 0), now, 'en-US')).toBe('Thursday 8:00 AM');
    expect(replyTime(at(20, 8, 0, 2025, 11), now, 'en-US')).toBe('Dec 20, 2025, 8:00 AM');
    expect(replyTime(undefined, now)).toBeNull();
    expect(replyTime('not a time', now)).toBeNull();
  });

  it('takes the receipt’s finish, else the reply’s own time, so every finished reply has one (C8)', () => {
    const done = '2026-11-12T07:00:00Z', ended = '2026-11-12T07:00:02Z';
    expect(replyFinishedAt({ finishedAt: done }, ended)).toBe(done);
    // No receipt (an emitted or reloaded reply without one): the turn's end.
    expect(replyFinishedAt(undefined, ended)).toBe(ended);
    expect(replyFinishedAt(null, ended)).toBe(ended);
    expect(replyFinishedAt({}, ended)).toBe(ended);
    expect(replyFinishedAt({ finishedAt: 'garbage' }, ended)).toBe(ended);
    expect(replyFinishedAt(undefined, undefined)).toBeUndefined();
    expect(replyFinishedAt({ finishedAt: '' }, 'not a time')).toBeUndefined();
  });

  it('saves a reason only with Not useful, trimmed and capped', () => {
    expect(voteInput('r1', 'useful', 'great')).toEqual({ runId: 'r1', vote: 'useful', reason: null });
    expect(voteInput('r1', 'notUseful')).toEqual({ runId: 'r1', vote: 'notUseful', reason: null });
    expect(voteInput('r1', 'notUseful', '  wrong file  ')).toEqual({ runId: 'r1', vote: 'notUseful', reason: 'wrong file' });
    expect(voteInput('r1', 'notUseful', 'x'.repeat(3000)).reason).toHaveLength(2000);
  });

  it('words a saved vote and a refused one plainly', () => {
    expect(voteSavedText('useful')).toBe('Thanks: marked useful');
    expect(voteSavedText('notUseful')).toBe('Saved: not useful');
    expect(voteSavedText('notUseful', true)).toBe('Saved: not useful, with your reason');
    expect(voteErrorText('FEEDBACK_NO_TURN: no turn')).toBe('Not saved: this reply has no saved turn record.');
    expect(voteErrorText('FEEDBACK_TURN_RUNNING: busy')).toMatch(/still being written/);
    expect(voteErrorText('FEEDBACK_INVALID: reason too long')).toBe('Not saved: reason too long');
    expect(voteErrorText(new Error('disk full'))).toBe('Not saved: disk full');
  });
});

describe('the chips above a reply’s Worked for line (C2)', () => {
  const receipt = (paths: string[]): ExecutionReceipt => ({
    files: paths.map(path => ({ path, changeType: 'modify', attribution: 'confirmed' })),
  }) as unknown as ExecutionReceipt;

  it('shows three files as three chips, like his reference, and folds a longer list', () => {
    const three = fileChips(receipt(['notes/brief.md', 'notes/findings.md', 'notes/handoff.md']));
    expect(three.shown.map(c => c.name)).toEqual(['brief.md', 'findings.md', 'handoff.md']);
    expect(three.more).toBeNull();
    const five = fileChips(receipt(['a.md', 'b.md', 'c.md', 'd.md', 'e.md']));
    expect(five.shown.map(c => c.name)).toEqual(['a.md', 'b.md']);
    expect(five.more?.count).toBe(3);
  });

  it('leaves out another chat’s files and has nothing without a receipt', () => {
    const r = { files: [
      { path: 'mine.md', changeType: 'create', attribution: 'confirmed' },
      { path: 'theirs.md', changeType: 'modify', attribution: 'shared' },
    ] } as unknown as ExecutionReceipt;
    expect(fileChips(r).shown.map(c => c.name)).toEqual(['mine.md']);
    expect(fileChips(undefined).total).toBe(0);
  });
});
