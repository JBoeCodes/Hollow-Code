import { describe, expect, it } from 'vitest';
import { rowMarks, unreadCount, type RowFacts } from './rowStatus';

const base: RowFacts = { state: 'idle', ask: null, planReady: false, unread: 0, pr: null, current: false };
const ids = (f: Partial<RowFacts>) => rowMarks({ ...base, ...f }).map(m => m.id);

describe('row status marks', () => {
  it('shows nothing for a quiet chat', () => { expect(ids({})).toEqual([]); });
  it('names the ask: approval versus input, only while the chat is waiting', () => {
    expect(ids({ state: 'waiting', ask: 'approval' })).toEqual(['approval']);
    expect(ids({ state: 'waiting', ask: 'input' })).toEqual(['input']);
    expect(ids({ state: 'running', ask: 'approval' })).toEqual([]);
    expect(ids({ state: 'waiting', ask: null })).toEqual([]);
  });
  it('orders ask, plan, unread, pull request', () => {
    expect(ids({ state: 'waiting', ask: 'input', planReady: true, unread: 3, pr: { number: 7, state: 'open' } })).toEqual(['input', 'plan', 'unread', 'pr']);
  });
  it('hides plan and unread on the chat on screen, keeps the ask and the pull request', () => {
    expect(ids({ current: true, state: 'waiting', ask: 'approval', planReady: true, unread: 2, pr: { number: 1, state: 'merged' } })).toEqual(['approval', 'pr']);
  });
  it('words the unread count and the pull request state', () => {
    expect(rowMarks({ ...base, unread: 1 })[0].label).toBe('1 new message');
    expect(rowMarks({ ...base, unread: 4 })[0].label).toBe('4 new messages');
    const pr = rowMarks({ ...base, pr: { number: 12, state: 'merged' } })[0];
    expect([pr.text, pr.label, pr.tone]).toEqual(['#12', 'Pull request #12 merged', 'merged']);
    expect(rowMarks({ ...base, pr: { number: 5, state: null } })[0].label).toBe('Pull request #5');
  });
  it('counts unread only for a chat that was opened before', () => {
    expect(unreadCount(null, 9)).toBe(0);
    expect(unreadCount(4, 4)).toBe(0);
    expect(unreadCount(4, 7)).toBe(3);
    expect(unreadCount(9, 4)).toBe(0);
  });
});
