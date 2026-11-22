import { describe, expect, it } from 'vitest';
import { namesFolder, quickChatMoveTarget, titleMentions } from './quickChatMove';

const atlas = { id: 'atlas20-1a2b', title: 'Atlas 2.0', cwd: '/vault/workspaces/atlas20-1a2b', path: 'workspaces/atlas20-1a2b' };
const herald = { id: 'morning-paper', title: 'Morning paper', cwd: '/Users/me/Hollow/Projects/morning-paper' };
const work = { id: 'work', title: 'Work' };

describe('quickChatMoveTarget (quick chats, 2026-11-14)', () => {
  it('offers the one project whose title is said twice', () => {
    expect(quickChatMoveTarget('Can Atlas 2.0 price weekends?', 'Yes: Atlas 2.0 already reads the weekend rates.', [atlas, herald])).toBe(atlas);
  });

  it('offers a project whose folder is named, even once', () => {
    expect(quickChatMoveTarget('look at this', 'The draft is in /Users/me/Hollow/Projects/morning-paper/press.py now.', [atlas, herald])).toBe(herald);
    expect(namesFolder('see workspaces/atlas20-1a2b/PLAN.md', atlas)).toBe(true);
    expect(namesFolder('in /Users/me/notes.md', { id: 'x', title: 'Broad', cwd: '/Users/me' })).toBe(false);
  });

  it('offers nothing for one passing mention, a common title or two projects', () => {
    expect(quickChatMoveTarget('What is Atlas 2.0?', 'A pricing tool.', [atlas, herald])).toBeNull();
    expect(quickChatMoveTarget('work work work', 'More work.', [work])).toBeNull();
    expect(quickChatMoveTarget('Atlas 2.0 and the Morning paper', 'Atlas 2.0 feeds the Morning paper.', [atlas, herald])).toBeNull();
    expect(quickChatMoveTarget('', '', [atlas])).toBeNull();
  });

  it('counts whole words in any case only', () => {
    expect(titleMentions('atlas 2.0, ATLAS 2.0', 'Atlas 2.0')).toBe(2);
    expect(titleMentions('Morning papers', 'Morning paper')).toBe(0);
  });
});
