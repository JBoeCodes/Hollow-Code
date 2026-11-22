import { describe, expect, it } from 'vitest';
import { branchLabel, branchParts, branchWords, changesCount, finalMessage, primaryAction, splitMessage } from './cardGitModel';

const base = { branch: 'main', head: 'abc1234', ahead: 0, behind: 0, linkedWorktree: false, files: [], moreFiles: 0, blocked: null };
const file = { path: 'a.txt', status: 'modified', additions: 1, deletions: 0 };

describe('the card git rows (parity 5.2)', () => {
  it('words the branch chip with push and behind marks', () => {
    expect(branchWords(base)).toBe('main');
    expect(branchWords({ ...base, ahead: 2, behind: 1 })).toBe('main ↑2 ↓1');
    expect(branchWords({ ...base, branch: '' })).toBe('Detached at abc1234');
    expect(branchWords({ ...base, branch: '', head: '' })).toBe('No commits yet');
    expect(branchParts({ ...base, ahead: 2, behind: 1 })).toEqual({ name: 'main', marks: '↑2 ↓1' });
    expect(branchLabel({ ...base, ahead: 1, linkedWorktree: true })).toBe('Branch main, in a worktree, 1 commit to push');
  });
  it('counts changed files', () => {
    expect(changesCount({ files: [], moreFiles: 0 })).toBe('None');
    expect(changesCount({ files: [file], moreFiles: 0 })).toBe('1 file');
    expect(changesCount({ files: [file], moreFiles: 4 })).toBe('5 files');
  });
  it('picks the primary action and says why it is off', () => {
    expect(primaryAction({ ...base, files: [file] }, false)).toEqual({ label: 'Commit and push', kind: 'commit-push', why: null });
    expect(primaryAction({ ...base, ahead: 2 }, false)).toEqual({ label: 'Push', kind: 'push', why: null });
    expect(primaryAction(base, false).kind).toBe('none');
    expect(primaryAction({ ...base, files: [file] }, true).kind).toBe('none');
    expect(primaryAction({ ...base, files: [file], blocked: 'Wait for a run' }, false).why).toBe('Wait for a run');
    expect(primaryAction({ ...base, branch: '', files: [file] }, false).why).toMatch(/detached/);
  });
  it('keeps the generated file list only while the subject is unchanged', () => {
    const g = splitMessage('Update a.txt\n\n- update a.txt (+1 -0)');
    expect(g).toEqual({ subject: 'Update a.txt', body: '- update a.txt (+1 -0)' });
    expect(finalMessage('Update a.txt', g)).toBe('Update a.txt\n\n- update a.txt (+1 -0)');
    expect(finalMessage('  Fix the thing ', g)).toBe('Fix the thing');
    expect(splitMessage('One line')).toEqual({ subject: 'One line', body: '' });
  });
});
