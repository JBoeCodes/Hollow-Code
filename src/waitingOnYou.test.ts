import { describe, expect, it, vi } from 'vitest';
import {
  DRAFT_IDLE_MS, WAITING_FILTER_PAGE, WAITING_GROUPS, WAITING_PAGE, askCanvasOf, countedDrafts, draftsNote, groupWaiting, inWaitingFilter, moreCount, openWaitingRow, reviewCardView,
  reviewRowLabel, unavailableNotice, waitingBadgeOf, waitingCount, waitingDisabledReason, waitingEmptyState, waitingQueryCategories, waitingQueryKinds, waitingTarget, type WaitingOpeners, type WaitingRow,
} from './waitingOnYou';
import { createNeedsYouList } from './needsYou';
import type { NeedsYouItem, NeedsYouPage } from './workspaceTypes';

const item = (over: Partial<NeedsYouItem> & Pick<NeedsYouItem, 'id' | 'kind'>): NeedsYouItem => ({
  sourceId: over.id.split(':').slice(1).join(':'), title: over.id, detail: null, state: 'pending', at: null,
  workspaceId: null, sessionKey: null, path: null, category: null, link: null, ...over,
});

const approval = item({ id: 'approval:apr-1', kind: 'approval', title: 'Quit Safari', sessionKey: 's-old', workspaceId: 'alpha' });
const question = item({ id: 'question:q-1', kind: 'question', title: 'Which venue?', sessionKey: 's-q' });
const result = item({ id: 'result:workspaces/alpha/outputs/r.md', kind: 'result', title: 'Report', state: 'ready', workspaceId: 'alpha', path: 'workspaces/alpha/outputs/r.md' });
const learning = item({ id: 'review:rev-7', kind: 'review', title: 'Release verification', category: 'learning' });
const task = item({ id: 'review:rev-8', kind: 'review', title: 'Draft the rollout checklist', category: 'agent_task' });
const pause = item({ id: 'review:rev-9', kind: 'review', title: 'Budget pause', category: 'budget_pause' });
const failure = item({ id: 'failure:n-3', kind: 'failure', title: 'Morning brief failed', state: 'unread', link: 'workflow:morning-brief' });
const conflict = item({ id: 'conflict:page-1:status', kind: 'conflict', sourceId: 'page-1', title: 'Ship it', state: 'conflict', category: 'task', path: 'tasks/ship-it.md' });
const offer = item({ id: 'interrupted:2026-10-10T01:00:00.000Z', kind: 'interrupted', title: 'Resume 2 interrupted chats' });
const copies = item({ id: 'copies:vault', kind: 'copies', sourceId: 'vault', title: '2 conflict copies in the vault' });
const syncClash = item({ id: 'clash:vault:library/plan.md', kind: 'clash', sourceId: 'vault:library/plan.md', title: 'Two Macs changed plan.md' });
const memory = item({ id: 'lesson_memory:PER-4:m4.md', kind: 'lesson_memory', sourceId: 'PER-4:m4.md', title: 'Delete the memory m4?', detail: 'PER-4 is Done', category: 'PER-4' });
const draft = { path: 'workspaces/alpha/outputs/slides.md', savedAt: '2026-10-03T09:00:00Z' };
const canvas = item({ id: 'canvas:s-ask/release-choices@2', kind: 'canvas', sourceId: 'release-choices@2', title: 'Release choices', sessionKey: 's-ask', workspaceId: 'alpha', path: 'workspaces/alpha/outputs/canvases/release-choices.canvas.html' });

describe('groupWaiting (decision 7: one surface, grouped by kind)', () => {
  it('groups every kind in the fixed order, keeping the query order inside each group, with drafts merged last', () => {
    const groups = groupWaiting([failure, task, approval, result, conflict, question, learning, offer, copies, syncClash, memory], [draft]);
    expect(groups.map(g => g.id)).toEqual(['failures', 'interrupted', 'asks', 'results', 'reviews', 'conflicts', 'clashes', 'copies', 'memories', 'drafts']);
    expect(groups.map(g => g.label)).toEqual(WAITING_GROUPS.map(g => g.label));
    expect(groups[2].rows.map(r => r.id)).toEqual(['approval:apr-1', 'question:q-1']);
    expect(groups[4].rows.map(r => r.id)).toEqual(['review:rev-8', 'review:rev-7']);
    expect(groups[9].rows).toEqual([{ kind: 'draft', id: `draft:${draft.path}`, draft }]);
  });

  it('leaves out empty groups', () => {
    expect(groupWaiting([result], []).map(g => g.id)).toEqual(['results']);
    expect(groupWaiting([], [])).toEqual([]);
  });

  it("filtered to a page's review kinds, shows only those review items and no drafts", () => {
    const groups = groupWaiting([approval, learning, task, pause, failure], [draft], 'tasks');
    expect(groups.map(g => g.id)).toEqual(['reviews']);
    expect(groups[0].rows.map(r => r.id)).toEqual(['review:rev-8']);
    expect(groupWaiting([learning, task, pause], [], 'budgets')[0].rows.map(r => r.id)).toEqual(['review:rev-9']);
    expect(inWaitingFilter(approval, null)).toBe(true);
    expect(inWaitingFilter(learning, 'learning')).toBe(true);
    expect(inWaitingFilter(learning, 'automations')).toBe(false);
    expect(waitingQueryKinds(null)).toBeUndefined();
    expect(waitingQueryKinds('learning')).toEqual(['review']);
  });
});

describe('waitingTarget / openWaitingRow (every row opens by its exact id)', () => {
  const row = (i: NeedsYouItem): WaitingRow => ({ kind: 'item', id: i.id, item: i });
  const spies = () => ({
    session: vi.fn(), canvas: vi.fn(), result: vi.fn(), review: vi.fn(), failure: vi.fn(), task: vi.fn(), workspace: vi.fn(), note: vi.fn(), resume: vi.fn(), copies: vi.fn(), clashes: vi.fn(), lessonMemory: vi.fn(),
  }) satisfies WaitingOpeners;

  it('opens the interrupted-chats item as the resume offer, first of every group (overnight A1)', () => {
    const open = spies();
    expect(openWaitingRow(row(offer), open)).toBe(true);
    expect(open.resume).toHaveBeenCalledTimes(1);
    expect(open.session).not.toHaveBeenCalled();
    expect(groupWaiting([approval, offer], []).map(g => g.id)).toEqual(['interrupted', 'asks']);
  });

  it('opens the conflict-copies item as its dialog, in its own group (overnight A10)', () => {
    const open = spies();
    expect(openWaitingRow(row(copies), open)).toBe(true);
    expect(open.copies).toHaveBeenCalledTimes(1);
    expect(open.note).not.toHaveBeenCalled();
    expect(groupWaiting([copies, approval], []).map(g => g.id)).toEqual(['asks', 'copies']);
  });

  it('opens a context sync clash as the clash cards, before conflict copies (CX6c)', () => {
    const open = spies();
    const clash = item({ id: 'clash:personal:me/Identity.md', kind: 'clash', sourceId: 'personal:me/Identity.md', title: 'Two Macs changed Identity.md', path: 'me/Identity.md', category: 'context-sync' });
    expect(openWaitingRow(row(clash), open)).toBe(true);
    expect(open.clashes).toHaveBeenCalledTimes(1);
    expect(open.note).not.toHaveBeenCalled();
    expect(groupWaiting([copies, clash], []).map(g => g.id)).toEqual(['clashes', 'copies']);
  });

  it('opens a lesson memory offer as Delete or Keep by its own id, in its own group (build run L2)', () => {
    const open = spies();
    expect(openWaitingRow(row(memory), open)).toBe(true);
    expect(open.lessonMemory).toHaveBeenCalledWith('PER-4:m4.md', 'Delete the memory m4?', 'PER-4 is Done');
    expect(open.note).not.toHaveBeenCalled();
    expect(groupWaiting([memory, copies], []).map(g => g.id)).toEqual(['copies', 'memories']);
  });

  it('opens each kind through its own opener with the id its store knows', () => {
    const open = spies();
    for (const i of [approval, question, result, learning, failure, conflict]) expect(openWaitingRow(row(i), open)).toBe(true);
    expect(openWaitingRow({ kind: 'draft', id: 'draft:x', draft }, open)).toBe(true);
    expect(open.session.mock.calls).toEqual([['s-old'], ['s-q']]);
    expect(open.result).toHaveBeenCalledWith('alpha', 'workspaces/alpha/outputs/r.md');
    // The review id, not the row id or the title.
    expect(open.review).toHaveBeenCalledWith('rev-7');
    expect(open.failure).toHaveBeenCalledWith('n-3', 'workflow:morning-brief');
    expect(open.task).toHaveBeenCalledWith('tasks/ship-it.md');
    expect(open.note).toHaveBeenCalledWith(draft.path);
    expect(open.workspace).not.toHaveBeenCalled();
  });

  it('opens a project conflict in its workspace, and one with neither in its file', () => {
    expect(waitingTarget(row({ ...conflict, category: 'project', workspaceId: 'notion-p1' }))).toEqual({ kind: 'workspace', id: 'notion-p1' });
    expect(waitingTarget(row({ ...conflict, category: 'project', workspaceId: null }))).toEqual({ kind: 'note', path: 'tasks/ship-it.md' });
  });

  it('runs nothing for a row that names no place, and says why', () => {
    const open = spies();
    const unbound = row({ ...approval, sessionKey: null });
    const stale = row({ ...question, state: 'stale' });
    expect(openWaitingRow(unbound, open)).toBe(false);
    expect(openWaitingRow(stale, open)).toBe(false);
    expect(Object.values(open).every(fn => fn.mock.calls.length === 0)).toBe(true);
    expect(waitingDisabledReason(unbound)).toMatch(/answer it on its card/);
    expect(waitingDisabledReason(stale)).toMatch(/can no longer be answered/);
    expect(waitingDisabledReason(row(result))).toBeNull();
  });
});

describe('ask-canvases (Canvas C4 A3: waiting like a question card, reopened by exact id)', () => {
  const row = (i: NeedsYouItem): WaitingRow => ({ kind: 'item', id: i.id, item: i });
  const spies = () => ({
    session: vi.fn(), canvas: vi.fn(), result: vi.fn(), review: vi.fn(), failure: vi.fn(), task: vi.fn(), workspace: vi.fn(), note: vi.fn(), resume: vi.fn(), copies: vi.fn(), clashes: vi.fn(), lessonMemory: vi.fn(),
  }) satisfies WaitingOpeners;

  it('groups an ask-canvas with the approvals and questions, in the query order', () => {
    const groups = groupWaiting([failure, question, canvas, approval], []);
    expect(groups.map(g => g.id)).toEqual(['failures', 'asks']);
    expect(groups[1].rows.map(r => r.id)).toEqual(['question:q-1', 'canvas:s-ask/release-choices@2', 'approval:apr-1']);
    expect(groupWaiting([canvas], [], 'tasks')).toEqual([]);
  });

  it('opens its conversation at that canvas revision, by the conversation key and the canvas id', () => {
    expect(askCanvasOf(canvas)).toEqual({ canvasId: 'release-choices', revision: 2 });
    expect(waitingTarget(row(canvas))).toEqual({ kind: 'canvas', key: 's-ask', canvasId: 'release-choices', revision: 2 });
    const open = spies();
    expect(openWaitingRow(row(canvas), open)).toBe(true);
    expect(open.canvas).toHaveBeenCalledWith('s-ask', 'release-choices', 2);
    expect(open.session).not.toHaveBeenCalled();
    expect(waitingDisabledReason(row(canvas))).toBeNull();
  });

  it('names no place without a conversation or an exact canvas revision, and says why', () => {
    const open = spies();
    for (const bad of [
      { ...canvas, sessionKey: null },
      { ...canvas, sourceId: 'release-choices' },
      { ...canvas, sourceId: 'Release@2' },
      { ...canvas, sourceId: 'release-choices@0' },
      { ...canvas, sourceId: 'release-choices@2x' },
    ]) {
      expect(waitingTarget(row(bad)), bad.sourceId).toBeNull();
      expect(openWaitingRow(row(bad), open)).toBe(false);
      expect(waitingDisabledReason(row(bad))).toMatch(/canvas/);
    }
    expect(Object.values(open).every(fn => fn.mock.calls.length === 0)).toBe(true);
  });

  it('counts toward the bell like any other waiting item', () => {
    expect(waitingCount(2, [canvas, question])).toBe(2);
  });
});

describe('unavailableNotice (a partial list never reads as complete)', () => {
  it('names every source the query could not read', () => {
    expect(unavailableNotice([])).toBeNull();
    expect(unavailableNotice(['questions'])).toBe('Couldn’t read questions, so anything waiting there isn’t listed.');
    expect(unavailableNotice(['questions', 'results in Alpha', 'Notion sync'])).toBe('Couldn’t read questions, results in Alpha and Notion sync, so anything waiting there isn’t listed.');
  });
});

describe('paging and the count', () => {
  const fake = (all: NeedsYouItem[]) => async (cursor: string | null, limit: number): Promise<NeedsYouPage> => {
    const start = cursor ? all.findIndex(q => q.id === cursor) + 1 : 0;
    const items = all.slice(start, start + limit);
    return { items, total: all.length, nextCursor: start + items.length < all.length ? items[items.length - 1].id : null, unavailable: [] };
  };

  it('pages with Show more across groups until every item is grouped', async () => {
    const all = Array.from({ length: 40 }, (_, i) => item({ id: i % 2 ? `review:r-${i}` : `failure:f-${i}`, kind: i % 2 ? 'review' : 'failure', category: 'learning' }));
    const list = createNeedsYouList(fake(all), WAITING_PAGE);
    await list.load();
    expect(list.items()).toHaveLength(30);
    expect(moreCount(list.total(), list.items().length, WAITING_PAGE)).toBe(10);
    await list.load(true);
    const groups = groupWaiting(list.items(), []);
    expect(groups.map(g => [g.id, g.rows.length])).toEqual([['failures', 20], ['reviews', 20]]);
    expect(list.next()).toBeNull();
    expect(moreCount(list.total(), list.items().length, WAITING_PAGE)).toBe(0);
  });

  it("filtered to a page, asks the query for that page's review kinds, so older ones are never hidden behind other reviews", async () => {
    expect(waitingQueryCategories(null)).toBeUndefined();
    expect(waitingQueryCategories('budgets')).toEqual(['budget_pause']);
    expect(waitingQueryCategories('tasks')).toEqual(['heartbeat_task', 'meeting_task', 'agent_task', 'commitment', 'companion_conflict', 'companion_old_op']);
    // 250 newer learning reviews, then one budget pause: more than one query page.
    const all = [
      ...Array.from({ length: 250 }, (_, i) => item({ id: `review:l-${String(i).padStart(3, '0')}`, kind: 'review', category: 'learning' })),
      item({ id: 'review:pause-old', kind: 'review', category: 'budget_pause' }),
    ];
    const asked: { kinds?: readonly string[]; categories?: readonly string[] }[] = [];
    // The query's filter (needs_you::list_matching): kinds and categories apply before paging.
    const query = (kinds?: readonly string[], categories?: readonly string[]) => {
      asked.push({ kinds, categories });
      return fake(all.filter(i => (!kinds || kinds.includes(i.kind)) && (!categories || categories.includes(i.category ?? ''))));
    };
    const list = createNeedsYouList((cursor, limit) => query(waitingQueryKinds('budgets'), waitingQueryCategories('budgets'))(cursor, limit), WAITING_FILTER_PAGE);
    await list.load();
    expect(asked[0]).toEqual({ kinds: ['review'], categories: ['budget_pause'] });
    expect(groupWaiting(list.items(), [], 'budgets').flatMap(g => g.rows.map(r => r.id))).toEqual(['review:pause-old']);
    expect(list.total()).toBe(1);
    expect(list.next()).toBeNull();
    expect(moreCount(list.total(), list.items().length, WAITING_FILTER_PAGE)).toBe(0);
  });

  it('counts everything waiting, less questions that can no longer be answered; drafts are named apart (D3)', () => {
    expect(waitingCount(7, [question, { ...question, id: 'question:q-2', state: 'stale' }])).toBe(6);
    expect(waitingCount(0, [])).toBe(0);
  });
});

describe('reviewCardView (a review card the snapshot lacks yet)', () => {
  const base = { found: false, loading: false, failed: false, reread: false };

  it('re-reads the snapshot once before saying an item the query lists is gone', () => {
    // Listed by the query, missing from an older snapshot: re-read, never "gone" yet.
    expect(reviewCardView(base)).toBe('reread');
    expect(reviewCardView({ ...base, reread: true, loading: true })).toBe('loading');
    // The re-read found it: the card opens.
    expect(reviewCardView({ ...base, reread: true, found: true })).toBe('card');
    // A read that worked and still lacks it: no longer waiting.
    expect(reviewCardView({ ...base, reread: true })).toBe('gone');
  });

  it("says the card couldn't be loaded when the read failed, not that the item is gone", () => {
    expect(reviewCardView({ ...base, failed: true })).toBe('reread');
    expect(reviewCardView({ ...base, failed: true, reread: true })).toBe('unreadable');
    // Last good answer still has it: the card shows.
    expect(reviewCardView({ ...base, failed: true, found: true })).toBe('card');
  });
});

describe('waitingEmptyState (no false "nothing waiting")', () => {
  it('says nothing is waiting only after an answer landed and the last load worked', () => {
    expect(waitingEmptyState({ shown: 0, more: false, loaded: false, failed: false })).toBe('loading');
    expect(waitingEmptyState({ shown: 0, more: false, loaded: true, failed: false })).toBe('empty');
    expect(waitingEmptyState({ shown: 0, more: false, loaded: true, failed: true })).toBe('none');
    expect(waitingEmptyState({ shown: 0, more: false, loaded: false, failed: true })).toBe('none');
    expect(waitingEmptyState({ shown: 0, more: true, loaded: true, failed: false })).toBe('list');
    expect(waitingEmptyState({ shown: 2, more: false, loaded: false, failed: true })).toBe('list');
  });

  it('a list is loaded only by an answer, stays loaded through a failed reload, and reset() unloads it', async () => {
    let fail = false;
    const list = createNeedsYouList(async () => {
      if (fail) throw new Error('vault unreadable');
      return { items: [question], total: 1, nextCursor: null, unavailable: [] };
    }, WAITING_PAGE);
    expect(list.loaded()).toBe(false);
    fail = true;
    await list.load();
    expect(list.loaded()).toBe(false);
    expect(list.failed()).toBe(true);
    fail = false;
    await list.load();
    expect(list.loaded()).toBe(true);
    expect(list.items()).toHaveLength(1);
    fail = true;
    await list.load();
    expect(list.loaded()).toBe(true);
    expect(list.failed()).toBe(true);
    list.reset();
    expect([list.loaded(), list.failed(), list.items().length, list.total(), list.next()]).toEqual([false, false, 0, 0, null]);
  });

  it('reset() drops an answer still in flight for the old question', async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const list = createNeedsYouList(async () => {
      await gate;
      return { items: [question], total: 1, nextCursor: null, unavailable: [] };
    }, WAITING_PAGE);
    const inFlight = list.load();
    list.reset();
    release();
    await inFlight;
    expect(list.loaded()).toBe(false);
    expect(list.items()).toEqual([]);
  });
});

describe('the Review row’s count (P1-14, D3)', () => {
  const now = Date.parse('2026-10-24T12:00:00Z');
  const at = (minutesAgo: number) => new Date(now - minutesAgo * 60_000).toISOString();

  it('counts a draft only when no editor has it open and it has idled ten minutes', () => {
    const typing = { path: 'notes/typing.md', savedAt: at(0) };
    const paused = { path: 'notes/paused.md', savedAt: at(9) };
    const idle = { path: 'notes/idle.md', savedAt: at(10) };
    const openIdle = { path: 'notes/open.md', savedAt: at(60) };
    // Recovered after a restart with no time: nothing is typing it.
    const restored = { path: 'notes/restored.md', savedAt: '' };
    expect(DRAFT_IDLE_MS).toBe(10 * 60_000);
    expect(countedDrafts([typing, paused, idle, openIdle, restored], new Set(['notes/open.md']), now).map(d => d.path))
      .toEqual(['notes/idle.md', 'notes/restored.md']);
    expect(countedDrafts([idle], new Set(), now - 1)).toEqual([]);
  });

  it('is unknown before the first answer and after a failed read, else a number without drafts', () => {
    const page = { total: 3, items: [question, { ...question, id: 'question:q-2', state: 'stale' }] };
    expect(waitingBadgeOf({ ...page, loaded: false, failed: false }, [draft])).toEqual({ count: null, drafts: 1 });
    expect(waitingBadgeOf({ ...page, loaded: true, failed: true }, [])).toEqual({ count: null, drafts: 0 });
    expect(waitingBadgeOf({ ...page, loaded: true, failed: false }, [draft])).toEqual({ count: 2, drafts: 1 });
    expect(waitingBadgeOf({ total: 0, items: [], loaded: true, failed: false }, [])).toEqual({ count: 0, drafts: 0 });
  });

  it('names an unknown count, the count and drafts each in their own words', () => {
    expect(reviewRowLabel({ count: null, drafts: 0 })).toBe('Review, count unavailable');
    expect(reviewRowLabel({ count: null, drafts: 2 })).toBe('Review, count unavailable, 2 unsaved drafts');
    expect(reviewRowLabel({ count: 0, drafts: 0 })).toBe('Review');
    expect(reviewRowLabel({ count: 3, drafts: 0 })).toBe('Review, 3 waiting');
    expect(reviewRowLabel({ count: 3, drafts: 1 })).toBe('Review, 3 waiting, 1 unsaved draft');
    expect(reviewRowLabel({ count: 0, drafts: 1 })).toBe('Review, 1 unsaved draft');
    expect(reviewRowLabel({ count: null, drafts: 1 }, 'Needs you')).toBe('Needs you, count unavailable, 1 unsaved draft');
    expect(reviewRowLabel({ count: 3, drafts: 0 }, 'Needs you')).toBe('Needs you, 3 waiting');
    // The visible quiet word beside the label.
    expect([0, 1, 3].map(draftsNote)).toEqual(['', '1 draft', '3 drafts']);
  });
});
