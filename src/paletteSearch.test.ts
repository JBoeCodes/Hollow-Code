import { describe, expect, it, vi } from 'vitest';
import { CONVERSATION_PAGE, createConversationSearch, excerptLine, firstNewConversationRow, inScope, keptSelection, isWorkspaceResult, paletteScore, projectFileRow, searchedConversations, titleMatchesFirst, vaultSourceLabel, wantsConversationSearch, wantsProjectFiles, wantsVaultSearch } from './paletteSearch';
import { askRowIndex } from './askScopes';
import { ACTION_BY_ID, ACTION_DEFS, actionLabel } from './actionDefs';
import { NAV_PAGES } from './navigation';
import type { AgentSessionSummary, ConnectedFile } from './types';
import type { SessionSearchPage, SessionTarget } from './workspaceTypes';
import { workspaceFixture } from './mockWorkspaces';
import PALETTE_SOURCE from './components/CommandPalette.tsx?raw';

const lease: ConnectedFile = { path: '/Hollow/Projects/Launch/leases/garage-14-lease-renewal-terms.md', relative: 'Launch/leases/garage-14-lease-renewal-terms.md', projectId: 'local-demo', size: 940, readable: true, state: 'text', modified: 0 };

// P1-48: the query matched as one substring, so "new task" found no command
// and "go to tasks" put the paid Ask Hollow row first (since One Ask, the Ask row).
describe('palette word matching (P1-48)', () => {
  // As the palette builds them: a page's row adds "go open" to its words.
  const nav = (id: string) => { const p = NAV_PAGES.find(n => n.id === id)!; return { label: p.label, keywords: `${p.keywords} go open` }; };
  const tasks = nav('tasks');
  const settings = nav('settings');
  const newTask = { label: actionLabel(ACTION_BY_ID['new-task']), keywords: ACTION_BY_ID['new-task'].keywords };
  it('the palette row says New task, as the menu does', () => {
    expect(newTask.label).toBe('New task');
  });
  it('every word typed starts a word of the row, in any order', () => {
    expect(paletteScore('new task', newTask)).toBeGreaterThan(0);
    expect(paletteScore('task new', newTask)).toBeGreaterThan(0);
    expect(paletteScore('open settings', settings)).toBeGreaterThan(0);
    expect(paletteScore('go to tasks', tasks)).toBeGreaterThan(0);
    expect(paletteScore('open tasks', settings)).toBe(0);
    expect(paletteScore('new tax', newTask)).toBe(0);
  });
  it('"new task", "open settings" and "go to tasks" rank their row above the Ask row', () => {
    for (const [q, row] of [['new task', newTask], ['open settings', settings], ['go to tasks', tasks]] as const) {
      expect(askRowIndex(paletteScore(q, row)), q).toBe(1);
    }
  });
  it('and each is the best of the palette’s page and action rows', () => {
    const rows = [
      ...NAV_PAGES.filter(p => !p.hidden).map(p => ({ id: `nav:${p.id}`, label: p.label, keywords: `${p.keywords} go open` })),
      ...ACTION_DEFS.filter(d => d.palette).map(d => ({ id: d.palette!, label: actionLabel(d), keywords: d.keywords })),
    ];
    const best = (q: string) => rows.map(r => ({ r, s: paletteScore(q, r) })).sort((a, b) => b.s - a.s)[0].r.id;
    expect(best('new task')).toBe('act:new-task');
    expect(best('open settings')).toBe('nav:settings');
    expect(best('go to tasks')).toBe('nav:tasks');
  });
  it('words in the label rank above words only in the keywords; the old tiers still order one word', () => {
    expect(paletteScore('new task', newTask)).toBeGreaterThan(paletteScore('new task', { label: 'Capture', keywords: 'new task' }));
    expect(paletteScore('open settings', settings)).toBeGreaterThan(paletteScore('open preferences', settings));
    expect(paletteScore('tasks', tasks)).toBe(1000);
    expect(paletteScore('tas', tasks)).toBe(700);
    expect(paletteScore('ask', { label: 'Plan and ask' })).toBe(500);
    expect(paletteScore('sk', { label: 'Plan and ask' })).toBe(300);
    expect(paletteScore('workbench', tasks)).toBe(100);
    expect(paletteScore('', tasks)).toBe(0);
  });
  it('vault search runs only in scopes that list its rows', () => {
    expect(wantsVaultSearch('all')).toBe(true);
    expect(wantsVaultSearch('notes')).toBe(true);
    expect(wantsVaultSearch('workspaces')).toBe(true);
    expect(wantsVaultSearch('projects')).toBe(false);
  });
  it('a conversation search that fails says so, not "no matches"', async () => {
    const search = createConversationSearch(() => Promise.reject(new Error('search_sessions failed')));
    expect(search.phase()).toBe('idle');
    const running = search.search('budget');
    expect(search.phase()).toBe('pending');
    await running;
    expect(search.phase()).toBe('error');
    expect(search.hits()).toEqual([]);
    const ok = createConversationSearch(async () => ({ items: [], nextCursor: null }));
    await ok.search('budget');
    expect(ok.phase()).toBe('done');
    ok.clear();
    expect(ok.phase()).toBe('idle');
    // Busy from the keystroke (the palette's debounce), so an old failure
    // never shows meanwhile; More waits for the new search.
    let calls = 0;
    const paging = createConversationSearch(async (_q, cursor) => { calls++; return { items: [], nextCursor: cursor ? null : 'page-2' }; });
    await paging.search('budget');
    expect(paging.next()).toBe('page-2');
    paging.waiting();
    expect(paging.phase()).toBe('pending');
    expect(await paging.more()).toBe(false);
    expect(calls).toBe(1);
  });
  it('joining words drop only from a phrase of three or more', () => {
    expect(paletteScore('to do', { label: 'Doors', keywords: 'portal' })).toBe(0);
  });
});

describe('palette search scopes and rows (next-level P07)', () => {
  it('scopes narrow the groups listed', () => {
    expect(inScope('all', 'Actions')).toBe(true);
    expect(inScope('notes', 'Search results')).toBe(true);
    expect(inScope('notes', 'Project files')).toBe(false);
    expect(inScope('projects', 'Project files')).toBe(true);
    expect(inScope('projects', 'Actions')).toBe(false);
    expect(inScope('workspaces', 'Results')).toBe(true);
    expect(inScope('workspaces', 'Workspaces')).toBe(true);
    expect(inScope('workspaces', 'Notes')).toBe(false);
    expect(wantsProjectFiles('notes')).toBe(false);
    expect(wantsProjectFiles('all')).toBe(true);
  });
  it('every search row says where it came from', () => {
    expect(isWorkspaceResult('workspaces/w1/outputs/review.md')).toBe(true);
    expect(isWorkspaceResult('workspaces/w1/PLAN.md')).toBe(false);
    expect(vaultSourceLabel('workspaces/w1/outputs/review.md', id => (id === 'w1' ? 'Q3 review' : undefined))).toBe('Result · Q3 review');
    expect(vaultSourceLabel('workspaces/w1/outputs/review.md')).toBe('Result');
    expect(vaultSourceLabel('library/notes/pricing.md')).toBe('Library');
    expect(vaultSourceLabel('tasks/call-dana.md')).toBe('Task');
    expect(vaultSourceLabel('loose.md')).toBe('Note');
    // A workspace's sources/ file is a snapshot, not just "Workspace" (plan L3 X3).
    expect(vaultSourceLabel('workspaces/w1/sources/lease.md', id => (id === 'w1' ? 'Q3 review' : undefined))).toBe('Snapshot · Q3 review');
    expect(vaultSourceLabel('workspaces/w1/sources/lease.md')).toBe('Snapshot');
    expect(vaultSourceLabel('workspaces/w1/PLAN.md')).toBe('Workspace');
    expect(projectFileRow(lease, 'Launch')).toEqual({ label: 'garage-14-lease-renewal-terms.md', source: 'Project · Launch', detail: 'Launch/leases' });
    expect(projectFileRow({ ...lease, state: 'tooLarge', readable: false }, undefined, true).detail).toBe('Launch/leases · too large · not read · found directly');
    expect(projectFileRow(lease, undefined).source).toBe('Project file');
  });
  it('clips an excerpt to one line at a word', () => {
    expect(excerptLine('  The  lease\nrenewal ')).toBe('The lease renewal');
    const long = excerptLine('word '.repeat(60), 40);
    expect(long.endsWith('…')).toBe(true);
    expect(long.length).toBeLessThanOrEqual(41);
    expect(excerptLine(undefined)).toBe('');
  });
});

describe('palette conversation search (merged plan slice 4, B·F15)', () => {
  const hit = (key: string, title: string | null, workspaceId: string | null): SessionTarget =>
    ({ workspaceId, session: { key, title, agent: 'claude' } as AgentSessionSummary });
  it('asks the backend from 2 characters in the scopes that list conversations', () => {
    expect(wantsConversationSearch('all', 'bu')).toBe(true);
    expect(wantsConversationSearch('workspaces', ' budget ')).toBe(true);
    expect(wantsConversationSearch('all', ' b ')).toBe(false);
    expect(wantsConversationSearch('notes', 'budget')).toBe(false);
    expect(wantsConversationSearch('projects', 'budget')).toBe(false);
  });
  it('adds hits the rail does not list, each in its own workspace', () => {
    const rows = searchedConversations([
      hit('s-old', 'Quarterly budget', 'w-a'),
      hit('s-new', 'Budget draft', 'w-b'),
      hit('s-root', 'Budget at the root', null),
      hit('s-untitled', '  ', 'w-a'),
      { ...hit('s-text', 'Weekly sync', 'w-b'), snippet: '…the budget  for\nQ3…' },
    ], new Set(['s-new']));
    expect(rows).toEqual([
      { key: 's-old', workspaceId: 'w-a', title: 'Quarterly budget', agent: 'claude' },
      { key: 's-untitled', workspaceId: 'w-a', title: 'Conversation', agent: 'claude' },
      { key: 's-text', workspaceId: 'w-b', title: 'Weekly sync', agent: 'claude', detail: '…the budget for Q3…' },
    ]);
  });
  // Review of b36e5aed: a hit by what was asked shows its snippet as the
  // row's second line, and a page lists its title matches first, so one
  // can't fall behind a newer match by text. Within each page only: the
  // cursor still names the backend's last row.
  it('lists each page title matches first and shows a text hit\'s snippet', async () => {
    const byText = (key: string) => ({ ...hit(key, 'Weekly sync', 'w-a'), snippet: `…the ${key} lease…` });
    const keys = (hits: readonly SessionTarget[]) => hits.map(h => h.session.key);
    expect(keys(titleMatchesFirst([byText('s-1'), hit('s-2', 'Lease', 'w-a'), byText('s-3'), hit('s-4', 'Lease 2', 'w-a')]))).toEqual(['s-2', 's-4', 's-1', 's-3']);
    const pages: SessionSearchPage[] = [
      { items: [byText('s-1'), hit('s-2', 'Lease', 'w-a')], nextCursor: 'page-2' },
      { items: [byText('s-3'), hit('s-4', 'Lease 2', 'w-a')], nextCursor: null },
    ];
    const search = createConversationSearch(async (_q, cursor) => pages[cursor ? 1 : 0]);
    await search.search('lease');
    expect(keys(search.hits())).toEqual(['s-2', 's-1']);
    expect(await search.more()).toBe(true);
    expect(keys(search.hits())).toEqual(['s-2', 's-1', 's-4', 's-3']);
    expect(searchedConversations(search.hits(), new Set<string>()).map(r => r.detail ?? null)).toEqual([null, '…the s-1 lease…', null, '…the s-3 lease…']);
    expect(PALETTE_SOURCE).toMatch(/detail: c\.detail, keywords: `conversation chat thread \$\{c\.agent\}`/);
  });
  // Wave 2 review nit: the palette showed only the first 20 hits, with no
  // way to the rest. A "More conversations…" row now fetches nextCursor.
  const many = Array.from({ length: 45 }, (_, i) => hit(`s-${i}`, `Budget ${i}`, 'w-a'));
  const paged = (all: SessionTarget[], calls: [string, string | null][] = []) =>
    async (query: string, cursor: string | null, limit: number): Promise<SessionSearchPage> => {
      calls.push([query, cursor]);
      const start = cursor ? all.findIndex(h => h.session.key === cursor) + 1 : 0;
      const items = all.slice(start, start + limit);
      return { items, nextCursor: start + items.length < all.length ? items[items.length - 1].session.key : null };
    };
  it('reaches every hit past the first page with More conversations…', async () => {
    const calls: [string, string | null][] = [];
    const search = createConversationSearch(paged(many, calls));
    await search.search('budget');
    expect(search.hits()).toHaveLength(CONVERSATION_PAGE);
    expect(search.next()).not.toBeNull();
    await search.more();
    await search.more();
    expect(search.hits().map(h => h.session.key)).toEqual(many.map(h => h.session.key));
    expect(search.next()).toBeNull();
    expect(calls).toEqual([['budget', null], ['budget', 's-19'], ['budget', 's-39']]);
    await search.more();
    expect(calls).toHaveLength(3);
  });
  it('drops a page a newer search overtook, and clear empties the rows', async () => {
    let release!: (p: SessionSearchPage) => void;
    let first = true;
    const search = createConversationSearch((query, cursor, limit) => {
      if (first) { first = false; return new Promise(r => { release = r; }); }
      return paged(many)(query, cursor, limit);
    });
    const slow = search.search('bu');
    await search.search('budget');
    release({ items: [hit('s-stale', 'Stale', 'w-a')], nextCursor: null });
    await slow;
    expect(search.hits()[0].session.key).toBe('s-0');
    search.clear();
    expect(search.hits()).toEqual([]);
    expect(search.next()).toBeNull();
  });
  it('the palette lists a More conversations… row that keeps it open', () => {
    expect(PALETTE_SOURCE).toMatch(/if \(conversations\.next\(\) != null\) conversationItems\.push\(/);
    expect(PALETTE_SOURCE).toMatch(/label: 'More conversations…'[\s\S]{0,120}keepOpen: true, run: \(\) => moreConversations\(\)/);
    expect(PALETTE_SOURCE).toMatch(/if \(item\.keepOpen\) \{/);
  });
  // Wave 2 review nit: after More loaded, the list change reset the
  // selection to row 0, so the keyboard lost its place. It now lands on the
  // first row the page added.
  it('selects the first conversation row More added, not row 0', async () => {
    const before = new Set(['action:new', 'conversation:s-0', 'conversation:s-1', 'conversations:more']);
    const after = ['action:new', 'conversation:s-0', 'conversation:s-1', 'conversation:s-2', 'conversation:s-3', 'conversations:more', 'search:notes/a.md'];
    expect(firstNewConversationRow(after, before)).toBe(3);
    expect(firstNewConversationRow([...before], before)).toBe(-1);
    const search = createConversationSearch(paged(many));
    await search.search('budget');
    expect(await search.more()).toBe(true);
    expect(await search.more()).toBe(true);
    expect(await search.more()).toBe(false);
    expect(PALETTE_SOURCE).toMatch(/if \(!\(await conversations\.more\(\)\) \|\| query\(\) !== q\) return;\s*const at = firstNewConversationRow\(filtered\(\)\.map\(i => i\.id\), before\);\s*if \(at >= 0\) setSelectedIdx\(at\);/);
  });
  // 2026-11-04: the browser mock answers as session_store.rs does: each word
  // in the title or in one of the person's messages (never an answer), and a
  // hit whose title lacks a word carries a snippet of the message.
  it('the browser mock finds a conversation by what the person asked, with a snippet', () => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => { store.set(k, v); },
      removeItem: (k: string) => { store.delete(k); },
    });
    try {
      const cwd = '/Users/fixture/vault/workspaces/w-a';
      const session = (key: string, title: string | null, lastActivityAt: string) => ({ key, title, agent: 'claude', cwd, lastActivityAt });
      const user = (text: string) => ({ role: 'user', text });
      store.set('mockWorkspaces', JSON.stringify([{ id: 'w-a', title: 'Alpha', status: 'active', path: 'workspaces/w-a', cwd, modified: '2026-11-04T00:00:00Z' }]));
      store.set('mockWorkspaceSessions', JSON.stringify([
        session('s-title', 'Garage lease', '2026-11-04T10:00:00Z'),
        session('s-text', 'Weekly sync', '2026-11-04T09:00:00Z'),
        session('s-untitled', null, '2026-11-04T08:00:00Z'),
      ]));
      store.set('mockWorkspaceConversations', JSON.stringify({
        's-text': { turns: [user('Plan the garage lease renewal'), { role: 'assistant', text: 'An answer' }, user('Send it to Dana'), user('⟲ Hollow: check-in at 10:00. The audit is due.')] },
        's-untitled': { turns: [user(`${'word '.repeat(60)}lease ${'more '.repeat(60)}`), user(`${'alpha '.repeat(30)}renewal ${'omega '.repeat(30)}`)] },
      }));
      const search = (query: string) => (workspaceFixture('search_sessions', { query }, new Map())!.value as SessionSearchPage)
        .items.map(t => [t.session.key, t.snippet ?? null]);
      expect(search('LEASE')).toEqual([
        ['s-title', null],
        ['s-text', 'Plan the garage lease renewal'],
        ['s-untitled', expect.stringMatching(/^…(word )+lease( more)+…$/)],
      ]);
      expect(search('weekly dana')).toEqual([['s-text', 'Send it to Dana']]);
      expect(search('answer')).toEqual([]);
      // Hollow's own messages are not the person's.
      expect(search('audit')).toEqual([]);
      // Cut between words at both ends, as the backend cuts it.
      expect(search('renewal')).toEqual([
        ['s-text', 'Plan the garage lease renewal'],
        ['s-untitled', expect.stringMatching(/^…(alpha )+renewal( omega)+…$/)],
      ]);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('palette selection follows the row, not the index (P0-26)', () => {
  const rows = ['act:new-conversation', 'act:jump', 'act:stop-turn', 'page:tasks', 'page:library'];
  it('ArrowDown three times, then a sessions update: the same row stays selected', () => {
    const picked = rows[3];
    const updated = ['conversation:s-new', ...rows];
    expect(updated[keptSelection(updated, picked, true)]).toBe('page:tasks');
  });
  it('a new query or scope, or a row that left, starts at the top', () => {
    expect(keptSelection(rows, 'page:tasks', false)).toBe(0);
    expect(keptSelection(rows, 'conversation:gone', true)).toBe(0);
    expect(keptSelection(rows, undefined, true)).toBe(0);
  });
  it('the palette keeps the selection through its own row changes', () => {
    expect(PALETTE_SOURCE).toMatch(/createEffect\(on\(filtered, \(list, prev\) => \{/);
    expect(PALETTE_SOURCE).toMatch(/setSelectedIdx\(keptSelection\(list\.map\(i => i\.id\), picked, input === lastInput\)\);/);
    expect(PALETTE_SOURCE).not.toMatch(/filtered\(\);\s*setSelectedIdx\(0\);/);
  });
});
