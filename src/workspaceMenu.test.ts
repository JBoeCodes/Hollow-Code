import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { WorkWorkspace } from './workspaceTypes';
import type { ContextMenuItem } from './store';
import type { AgentSessionSummary } from './types';

// P0-13: Delete… says why it is off before the click, for the reasons the
// backend refuses (`workspaces::trash_checks`): a build workspace whose
// worktree is still there, and a conversation in it with a live turn.
const state = vi.hoisted(() => ({
  sessions: {} as Record<string, { sessions: { key: string; title: string | null }[] }>,
  queue: [] as { source: string; sourceId: string | null; status: string }[],
  background: [] as string[],
  open: null as string | null,
  busy: false,
  obsidian: false,
  leave: true,
}));
vi.mock('./api', () => ({ api: { obsidianAvailable: () => Promise.resolve(state.obsidian) } }));
vi.mock('./store/canvasLookTarget', () => ({ setCanvasLookTarget: vi.fn() }));
vi.mock('./store/theme', () => ({ customThemes: { latest: [] } }));
vi.mock('./store', () => ({
  canLeaveConversation: () => state.leave, flashFooter: vi.fn(), backgroundChatKeys: () => state.background, chatBusy: () => state.busy,
  chatCwd: () => state.open,
}));
vi.mock('./store/runQueue', () => ({ runQueue: () => state.queue }));
vi.mock('./store/mainAgent', () => ({ openMainAgent: vi.fn() }));
vi.mock('./cooMenu', () => ({ cooMenuItems: () => [{ label: 'Change hard stop', action: () => {} }] }));
const setIconEditor = vi.hoisted(() => vi.fn());
vi.mock('./store/customIcons', () => ({ setIconEditor }));
vi.mock('./store/workspacePins', () => ({ moveWorkspacePin: vi.fn(), pinWorkspace: vi.fn(), pinnedWorkspaces: () => [], unpinWorkspace: vi.fn() }));
vi.mock('./store/workspaces', () => ({
  archiveFromRail: vi.fn(), archiveOlderEntries: vi.fn(), chooseMergeTarget: vi.fn(), chooseMoveTarget: vi.fn(), deleteWorkspace: vi.fn(),
  isInRail: () => true, railHiddenIds: () => [], confirmUndoMerge: vi.fn(), newWorkspaceSession: vi.fn(), openProjectAgent: vi.fn(),
  removeWorktree: vi.fn(), renameWorkspace: vi.fn(), selectWorkspace: vi.fn(), lifecycleFailed: vi.fn(),
  setWorkspaceStatus: vi.fn(), proposeArchive: vi.fn(), startQuickChat: vi.fn(), workspaceEditLock: () => '', unarchiveWorkspace: vi.fn(), pickConversation: vi.fn(),
  workspaceSessions: () => state.sessions,
  sessionWorking: (key: string) => state.background.includes(key) || state.queue.some(q => q.source === 'chat' && q.sourceId === key && ['queued', 'claimed', 'running'].includes(q.status)),
  workspaceOfSession: (key: string) => Object.entries(state.sessions).find(([, g]) => g.sessions.some(s => s.key === key))?.[0] ?? null,
}));

const { workspaceMenuItems, sessionMenuItems, historyMenuItems } = await import('./workspaceMenu');
const { default: menuSource } = await import('./workspaceMenu.ts?raw');

const ws = (over: Partial<WorkWorkspace> = {}): WorkWorkspace => ({
  id: 'hollow-build-1', title: 'Hollow build', status: 'active', path: 'workspaces/hollow-build-1',
  cwd: '/vault/workspaces/hollow-build-1', modified: '2026-10-23T03:00:00Z', ...over,
});
const build = (exists: boolean) => ({ repo: '/repo', worktree: '/repo/.worktrees/b1', branch: 'hollow/b1', base: 'main', exists });
const deleteItem = (w: WorkWorkspace) => {
  const item = workspaceMenuItems(w).find(i => 'label' in i && i.label?.startsWith('Delete…'));
  if (!item || !('label' in item)) throw new Error('no Delete… item');
  return item;
};
const RUNNING = 'Delete… (a conversation here is running)';

beforeEach(() => {
  state.sessions = { 'plain-1': { sessions: [{ key: 's-1', title: 'Idle one' }, { key: 's-2', title: 'Busy one' }] }, other: { sessions: [{ key: 's-9', title: 'Elsewhere' }] } };
  state.queue = []; state.background = []; state.open = null; state.busy = false; state.leave = true;
});

it('the COO menu holds Change icon… and the actions of its removed page header band (11-06, 11-11)', () => {
  const items = workspaceMenuItems(ws({ id: 'main', title: 'COO', path: 'workspaces/main', cwd: '/vault/workspaces/main' }));
  expect(items.map(i => ('label' in i ? i.label : 'separator'))).toEqual(['Open the COO', 'Change icon…', 'separator', 'Change hard stop', 'separator', 'Reveal in Finder']);
  const change = items.find(i => 'label' in i && i.label === 'Change icon…');
  if (!change || !('action' in change) || !change.action) throw new Error('no Change icon… item');
  void change.action();
  expect(setIconEditor).toHaveBeenCalledWith({ target: 'mainAgent', mode: 'upload' });
});

it('a build workspace with its worktree still there has Delete off, saying to remove the worktree first', () => {
  const item = deleteItem(ws({ build: build(true) }));
  expect(item.disabled).toBe(true);
  expect(item.label).toBe('Delete… (remove its worktree first)');
});

it('once the worktree is removed, Delete is on', () => {
  const item = deleteItem(ws({ build: build(false) }));
  expect(item.disabled).toBe(false);
  expect(item.label).toBe('Delete…');
});

for (const status of ['running', 'claimed', 'queued']) it(`a ${status} turn of a conversation in the workspace turns Delete off, saying so`, () => {
  state.queue = [{ source: 'chat', sourceId: 's-2', status }];
  const item = deleteItem(ws({ id: 'plain-1' }));
  expect(item.disabled).toBe(true);
  expect(item.label).toBe(RUNNING);
});

it('a conversation running in the background (one paused on an approval card too) turns Delete off', () => {
  state.background = ['s-1'];
  expect(deleteItem(ws({ id: 'plain-1' })).label).toBe(RUNNING);
});

it('the busy chat on screen turns Delete off before it is listed, only in its own folder', () => {
  const plain = ws({ id: 'plain-1', cwd: '/vault/workspaces/plain-1' });
  state.open = '/vault/workspaces/plain-1'; state.busy = true;
  expect(deleteItem(plain).label).toBe(RUNNING);
  // A busy chat at the vault root (the Chat page) leaves this workspace alone.
  state.open = null;
  expect(deleteItem(plain).disabled).toBe(false);
});

// P1-41: every menu failure goes through lifecycleFailed (banner plus toast),
// so a failure from the rail on Tasks is seen, not left on the Work page.
it('workspaceMenu.ts calls no setWorkspaceError directly', () => {
  expect(menuSource).not.toMatch(/setWorkspaceError/);
  expect(menuSource).toMatch(/lifecycleFailed/);
});

it('the menu’s Pin, Unpin and Move up/down fail through lifecycleFailed too', async () => {
  const { default: pins } = await import('./store/workspacePins.ts?raw');
  expect(pins).not.toMatch(/setWorkspaceError/);
  expect(pins).toMatch(/lifecycleFailed\(String\(e\)\)/);
});

it('with no obsidian:// handler, Open brief in Obsidian is off and says why', async () => {
  const obsidian = () => workspaceMenuItems(ws()).find(i => 'label' in i && i.label?.startsWith('Open brief in Obsidian'));
  // Asked at load: the first menu already knows.
  await new Promise(r => setTimeout(r, 0));
  const item = obsidian();
  expect(item && 'label' in item && item.label).toBe('Open brief in Obsidian (Obsidian isn’t installed)');
  expect(item && 'disabled' in item && item.disabled).toBe(true);
});

// P1-38: an archived workspace's only status item is Unarchive, which puts
// back the status it had; an active one offers Mark as and Archive….
const labels = (w: WorkWorkspace) => workspaceMenuItems(w).flatMap(i => 'label' in i && i.label ? [i.label] : []);
it('an archived workspace offers Unarchive and no Mark as or Archive…', async () => {
  const items = labels(ws({ status: 'archived', archivedFrom: 'planning' }));
  expect(items.filter(l => l.startsWith('Mark as'))).toEqual([]);
  expect(items.some(l => l.startsWith('Archive…'))).toBe(false);
  expect(items).toContain('Unarchive (back to planning)');
  const { unarchiveWorkspace } = await import('./store/workspaces');
  const item = workspaceMenuItems(ws({ status: 'archived', archivedFrom: 'planning' })).find(i => 'label' in i && i.label?.startsWith('Unarchive'));
  if (item && 'action' in item) item.action?.();
  expect(unarchiveWorkspace).toHaveBeenCalledWith(expect.objectContaining({ id: 'hollow-build-1' }));
});
it('an archived workspace with no recorded status unarchives to active', () => {
  expect(labels(ws({ status: 'archived' }))).toContain('Unarchive (back to active)');
});
it('an active workspace offers the other statuses and Archive…, not Unarchive', () => {
  const items = labels(ws());
  expect(items.filter(l => l.startsWith('Mark as'))).toEqual(['Mark as planning', 'Mark as blocked', 'Mark as done']);
  expect(items).toContain('Archive… (off the active list)');
  expect(items.some(l => l.startsWith('Unarchive'))).toBe(false);
});

// V5a: Archive older entries… is back on an active workspace's menu (not an
// archived one's) and opens the dry-run confirm for that workspace.
it('an active workspace offers Archive older entries…, an archived one does not', async () => {
  expect(labels(ws())).toContain('Archive older entries…');
  expect(labels(ws({ status: 'archived' })).some(l => l.startsWith('Archive older entries'))).toBe(false);
  const { archiveOlderEntries } = await import('./store/workspaces');
  const item = workspaceMenuItems(ws()).find(i => 'label' in i && i.label?.startsWith('Archive older entries'));
  if (item && 'action' in item) item.action?.();
  expect(archiveOlderEntries).toHaveBeenCalledWith(expect.objectContaining({ id: 'hollow-build-1' }));
});

// P1-39: one conversation menu in its three homes (a rail row, a Conversation
// history row, and the ⋯ beside the open conversation's title, which builds
// its items as a history row does), and no disabled item without its reason.
describe('one conversation menu', () => {
  const s = { key: 's-1', title: 'Idle one', agent: 'claude', harnessSessionId: null, cwd: '/vault/workspaces/plain-1', createdAt: '', lastActivityAt: '',
    endedAt: null, turnCount: 2, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'briefOnly' } as AgentSessionSummary;
  const names = (items: ContextMenuItem[]) => items.flatMap(i => 'label' in i && i.label ? [i.label] : []);
  const rail = () => sessionMenuItems(ws({ id: 'plain-1' }), s, 'Idle one');
  const history = (openBlocked: string | null = null, archived = false) => historyMenuItems(s, { archived, open: vi.fn(), openBlocked, after: vi.fn() });
  // The everyday ones lead, as in ChatGPT's and Claude's row menus (overnight 11-04 S3).
  const ALL = ['Rename…', 'Regenerate title', 'Move to project…', 'New project from this chat', 'Pin conversation', 'Snooze until…', 'Automatic settlement…', 'Open conversation', 'New conversation in project', 'Schedule in this chat…', 'Copy conversation ID', 'Settle conversation'];

  it('the rail, Conversation history and the conversation heading offer the same items', () => {
    expect(names(rail())).toEqual(ALL);
    expect(names(history())).toEqual(ALL);
  });

  it('Regenerate title asks the chat’s own agent through its own action, and is off while the chat works', () => {
    const regen = (items: ContextMenuItem[]) => items.find(i => 'label' in i && i.label?.startsWith('Regenerate title')) as { disabled?: boolean; label: string };
    expect(regen(rail()).disabled).toBeFalsy();
    state.background = ['s-1'];
    expect(regen(rail()).disabled).toBe(true);
    expect(regen(rail()).label).toMatch(/ \([a-z][^()]+\)$/);
  });

  it('the title renames in place, history by dialog', () => {
    const inPlace = vi.fn();
    const rename = sessionMenuItems(ws({ id: 'plain-1' }), s, 'Idle one', inPlace).find(i => 'label' in i && i.label === 'Rename…');
    (rename as { action: () => void }).action();
    expect(inPlace).toHaveBeenCalledTimes(1);
  });

  it('an archived conversation offers Restore in place of Archive', () => {
    expect(names(history(null, true))).toEqual([...ALL.slice(0, -1).map(l => l === 'Schedule in this chat…' ? `${l} (archived)` : l), 'Restore conversation']);
  });

  for (const [why, setup] of [
    ['queued', () => { state.queue = [{ source: 'chat', sourceId: 's-1', status: 'queued' }]; }],
    ['running in the background', () => { state.background = ['s-1']; }],
    ['a message still being sent', () => { state.leave = false; }],
  ] as const) it(`no disabled item is bare (${why}): each says why it is off`, () => {
    setup();
    for (const items of [rail(), history('the conversation on screen is working')]) {
      const off = items.filter(i => 'label' in i && i.disabled);
      expect(off.length).toBeGreaterThan(0);
      for (const i of off) expect('label' in i && i.label).toMatch(/ \([a-z][^()]+\)$/);
      expect(names(items).some(l => l.endsWith('(wait)') || l.endsWith('(working)'))).toBe(false);
    }
  });

  it('a working conversation’s Archive is off, saying it is working', () => {
    state.queue = [{ source: 'chat', sourceId: 's-1', status: 'running' }];
    const archive = rail().find(i => 'label' in i && i.label?.startsWith('Settle conversation'));
    expect(archive && 'label' in archive && archive.label).toBe('Settle conversation (this conversation is working)');
    expect(archive && 'disabled' in archive && archive.disabled).toBe(true);
  });

  it('Conversation history names the Hollow folder, never Obsidian', async () => {
    const { default: chat } = await import('./components/ConversationHistory.tsx?raw');
    expect(chat).not.toMatch(/Obsidian/);
    expect(chat).toMatch(/Hollow folder/);
  });
});

it('a turn running in another workspace, or one that has ended, leaves Delete on', () => {
  state.queue = [{ source: 'chat', sourceId: 's-9', status: 'running' }, { source: 'chat', sourceId: 's-1', status: 'done' }, { source: 'cron', sourceId: 's-2', status: 'running' }];
  expect(deleteItem(ws({ id: 'plain-1' })).disabled).toBe(false);
});
