// CH1: the words of Branch from here and the branch notice.
import { describe, expect, it } from 'vitest';
import { branchConfirm, branchedNote, branchNotice, type Branched, type BranchInfo } from './chatBranchWords';

const made = (over: Partial<Branched> = {}): Branched => ({ key: 'b', title: 'T (branch)', through: 2, turns: 2, checkpoint: null, refName: null, changedSince: [], ...over });
const info = (over: Partial<BranchInfo> = {}): BranchInfo => ({ key: 'b', fromKey: 'k', fromTitle: 'Sidebar', through: 2, checkpoint: null, ownTurns: 0, ...over });

describe('CH1 branch words', () => {
  it('says what a branch does to an isolated worktree before it is made', () => {
    expect(branchConfirm('Sidebar', true).body).toContain('neither chat records turn checkpoints');
    expect(branchConfirm('Sidebar', false).body).toContain('same folder');
    expect(branchConfirm('Sidebar', null).body).toContain('If this chat owns its worktree');
    expect(branchConfirm('Sidebar', false).body).toContain('The original stays as it is');
  });
  it('names where the files at the branch point are kept', () => {
    expect(branchedNote(made())).toBe('Branched after turn 2.');
    expect(branchedNote(made({ checkpoint: 'abc', refName: 'refs/hollow/branches/b' }))).toBe('Branched after turn 2. The files are as they were then.');
    expect(branchedNote(made({ checkpoint: 'abc', refName: 'refs/hollow/branches/b', changedSince: ['a', 'b'] })))
      .toBe('Branched after turn 2. 2 files have changed since; the files as they were are in refs/hollow/branches/b.');
  });
  it('names the original and counts the own turns', () => {
    expect(branchNotice(info())).toBe('A branch of “Sidebar” after turn 2. No turns of its own yet.');
    expect(branchNotice(info({ fromTitle: null, ownTurns: 3 }))).toBe('A branch of another chat after turn 2. 3 turns of its own.');
  });
});
