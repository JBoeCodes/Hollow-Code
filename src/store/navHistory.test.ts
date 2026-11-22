import { describe, expect, it } from 'vitest';
import { keepLibraryView, launchLanded, learnConversation, nav, setNav, step, visit, type NavEntry, type NavStack } from './navHistory';

const start: NavStack = { entries: [{ page: 'today' }], cursor: 0 };
const walk = (...entries: NavEntry[]) => entries.reduce(visit, start);

// P1-42: workspace A to B to C used to leave no step (same-page moves
// coalesced), so ⌘[ jumped straight past them to Tasks.
describe('Back and Forward through workspaces (P1-42)', () => {
  it('each workspace and conversation opened is its own step', () => {
    const s = walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'b', sessionKey: 'k2' }, { page: 'tasks' });
    expect(s.entries).toEqual([{ page: 'today' }, { page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'b', sessionKey: 'k2' }, { page: 'tasks' }]);
    const back = step(s, -1)!;
    expect(back.entry).toEqual({ page: 'today', workspaceId: 'b', sessionKey: 'k2' });
    const again = step(back.stack, -1)!;
    expect(again.entry).toEqual({ page: 'today', workspaceId: 'a', sessionKey: 'k1' });
    const forward = step(again.stack, 1)!;
    expect(forward.entry).toEqual({ page: 'today', workspaceId: 'b', sessionKey: 'k2' });
    expect(forward.stack.cursor).toBe(2);
  });

  it('another conversation in the same workspace is a step too', () => {
    const s = walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'a', sessionKey: 'k2' });
    expect(s.entries).toHaveLength(3);
  });

  it('only identical entries coalesce', () => {
    const a = { page: 'today', workspaceId: 'a', sessionKey: 'k1' } as const;
    const s = walk(a, a, { page: 'tasks' }, { page: 'tasks' });
    expect(s.entries).toEqual([{ page: 'today' }, a, { page: 'tasks' }]);
    // Leaving a workspace for Work's overview is a step.
    expect(visit(walk(a), { page: 'today' }).entries).toHaveLength(3);
    // The same page with a note open, then without, is a step (as before).
    const noted = { entries: [{ page: 'vault', notePath: 'n.md' }], cursor: 0 } satisfies NavStack;
    expect(visit(noted, { page: 'vault' }).entries).toHaveLength(2);
  });

  it('a new conversation not sent yet is its own step, and opening another doesn’t swallow it', () => {
    const s = walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'a' }, { page: 'today', workspaceId: 'a', sessionKey: 'k1' });
    expect(s.entries).toHaveLength(4);
    expect(step(s, -1)!.entry).toEqual({ page: 'today', workspaceId: 'a' });
  });

  it('a new conversation learns its key at its first send; another entry doesn’t', () => {
    setNav(walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'a' }));
    learnConversation('b', 'k9');
    expect(nav().entries[2]).toEqual({ page: 'today', workspaceId: 'a' });
    learnConversation('a', 'k3');
    expect(nav().entries[2]).toEqual({ page: 'today', workspaceId: 'a', sessionKey: 'k3' });
    // An entry that has its key keeps it.
    learnConversation('a', 'k4');
    expect(nav().entries[2].sessionKey).toBe('k3');
    expect(nav().entries[1].sessionKey).toBe('k1');
    // A refresh re-reports the key on screen: an older unsent entry of the
    // same workspace, or of another visit, isn't given it.
    const back = walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'today', workspaceId: 'a' }, { page: 'today', workspaceId: 'a', sessionKey: 'k1' });
    setNav(back);
    learnConversation('a', 'k1');
    expect(nav()).toBe(back);
    const visits = walk({ page: 'today', workspaceId: 'a' }, { page: 'today', workspaceId: 'b', sessionKey: 'k2' }, { page: 'today', workspaceId: 'a', sessionKey: 'k7' });
    setNav(visits);
    learnConversation('a', 'k8');
    expect(nav()).toBe(visits);
    // The person left for Tasks before the key came: that entry still learns it.
    setNav(walk({ page: 'today', workspaceId: 'a' }, { page: 'tasks' }));
    learnConversation('a', 'k5');
    expect(nav().entries[1]).toEqual({ page: 'today', workspaceId: 'a', sessionKey: 'k5' });
    expect(nav().cursor).toBe(2);
  });

  it('launch reopening the last conversation replaces the starting overview entry', () => {
    setNav(walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }));
    launchLanded('a');
    expect(nav()).toEqual({ entries: [{ page: 'today', workspaceId: 'a', sessionKey: 'k1' }], cursor: 0 });
    // Once the person has moved on, or opened another workspace meanwhile, it
    // leaves history alone.
    const moved = walk({ page: 'today', workspaceId: 'a', sessionKey: 'k1' }, { page: 'tasks' });
    setNav(moved);
    launchLanded('a');
    expect(nav()).toBe(moved);
    const other = walk({ page: 'today', workspaceId: 'b', sessionKey: 'k2' });
    setNav(other);
    launchLanded('a');
    expect(nav()).toBe(other);
  });

  it('a new visit after Back drops the forward entries, and the stack stays bounded', () => {
    const s = walk({ page: 'tasks' }, { page: 'vault' });
    const back = step(s, -1)!.stack;
    expect(visit(back, { page: 'settings' }).entries.map(e => e.page)).toEqual(['today', 'tasks', 'settings']);
    let long = start;
    for (let i = 0; i < 80; i++) long = visit(long, { page: 'today', workspaceId: `w${i}` });
    expect(long.entries).toHaveLength(50);
    expect(long.cursor).toBe(49);
    expect(long.entries[49].workspaceId).toBe('w79');
  });

  it('no step past either end', () => {
    expect(step(start, -1)).toBeNull();
    expect(step(start, 1)).toBeNull();
  });
});

// S4 review: Notes → Conversations → Results → Back → Back showed Results,
// because Library's steps didn't say which tab they showed.
describe('Library steps keep their tab (S4)', () => {
  it('a Library step left records the tab it showed; other steps and Conversations do not', () => {
    const s = walk({ page: 'vault' });
    const kept = keepLibraryView(s, 'notes');
    expect(kept.entries[kept.cursor]).toEqual({ page: 'vault', libraryView: 'notes' });
    // Same tab again: the same stack.
    expect(keepLibraryView(kept, 'notes')).toBe(kept);
    // A tab switch with no step of its own updates it when the step is left.
    expect(keepLibraryView(kept, 'results').entries[kept.cursor]).toEqual({ page: 'vault', libraryView: 'results' });
    const chat = walk({ page: 'vault' }, { page: 'chat' });
    expect(keepLibraryView(chat, 'notes')).toBe(chat);
    expect(keepLibraryView(s, 'conversations')).toBe(s);
    const tasks = walk({ page: 'tasks' });
    expect(keepLibraryView(tasks, 'notes')).toBe(tasks);
  });

  it('the kept tab does not split a coalesced visit', () => {
    const kept = keepLibraryView(walk({ page: 'vault' }), 'results');
    expect(visit(kept, { page: 'vault' })).toBe(kept);
  });
});
