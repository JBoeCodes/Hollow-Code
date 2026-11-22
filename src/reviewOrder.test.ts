import { describe, expect, it } from 'vitest';
import { REVIEW_VIEWS } from './store/destinations';
import { groupWaiting, waitingWorkspaces } from './waitingOnYou';
import type { NeedsYouItem } from './workspaceTypes';

// P1-13: Review says how it orders rows, puts the asks that unblock a running
// chat first within asks, and can be narrowed to one workspace.
const item = (over: Partial<NeedsYouItem> & Pick<NeedsYouItem, 'id' | 'kind'>): NeedsYouItem => ({
  sourceId: over.id, title: over.id, detail: null, state: 'pending', at: null,
  workspaceId: null, sessionKey: null, path: null, category: null, link: null, ...over,
});

describe('Review order', () => {
  it('lists Waiting as All, Decisions, Results and Failures', () => {
    expect(REVIEW_VIEWS.map(v => [v.id, v.label])).toEqual([['all', 'All'], ['decisions', 'Decisions'], ['results', 'Results'], ['failures', 'Failures']]);
  });
  it('All waiting explains failures-first groups and running-chat priority within asks', () => {
    expect(REVIEW_VIEWS.find(v => v.id === 'all')?.hint).toBe('Failures first; newest first within groups; running chats lead within approvals and questions');
  });

  it('asks whose chat is running come first, the rest keep their order', () => {
    const items = [
      item({ id: 'q:old', kind: 'question', sessionKey: 's-idle' }),
      item({ id: 'a:busy', kind: 'approval', sessionKey: 's-busy' }),
      item({ id: 'q:none', kind: 'question' }),
      item({ id: 'q:busy', kind: 'question', sessionKey: 's-busy' }),
    ];
    const asks = groupWaiting(items, [], null, { busy: new Set(['s-busy']) }).find(g => g.id === 'asks');
    expect(asks?.rows.map(r => r.id)).toEqual(['a:busy', 'q:busy', 'q:old', 'q:none']);
  });

  it('with no running chat the order is the query order', () => {
    const items = [item({ id: 'q:1', kind: 'question', sessionKey: 's1' }), item({ id: 'q:2', kind: 'question', sessionKey: 's2' })];
    expect(groupWaiting(items, []).find(g => g.id === 'asks')?.rows.map(r => r.id)).toEqual(['q:1', 'q:2']);
  });

  it('a workspace filter keeps that workspace\'s rows, and drafts only without one', () => {
    const items = [
      item({ id: 'r:a', kind: 'result', workspaceId: 'w-a' }),
      item({ id: 'f:b', kind: 'failure', workspaceId: 'w-b' }),
      item({ id: 'q:a', kind: 'question', workspaceId: 'w-a' }),
    ];
    const drafts = [{ path: 'notes/x.md', savedAt: '2026-10-24T06:00:00Z' }];
    const rows = (workspace: string | null) => groupWaiting(items, drafts, null, { workspace }).flatMap(g => g.rows.map(r => r.id));
    expect(rows('w-a')).toEqual(['q:a', 'r:a']);
    expect(rows('w-b')).toEqual(['f:b']);
    expect(rows(null)).toEqual(['f:b', 'q:a', 'r:a', 'draft:notes/x.md']);
  });

  it('the filter lists each workspace with its count, largest first', () => {
    const items = [
      item({ id: 'r:a', kind: 'result', workspaceId: 'w-a' }),
      item({ id: 'f:b', kind: 'failure', workspaceId: 'w-b' }),
      item({ id: 'q:a', kind: 'question', workspaceId: 'w-a' }),
      item({ id: 'c:none', kind: 'conflict' }),
    ];
    expect(waitingWorkspaces(items)).toEqual([{ id: 'w-a', count: 2 }, { id: 'w-b', count: 1 }]);
  });
});
