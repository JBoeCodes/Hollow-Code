import { describe, expect, it } from 'vitest';
import type { MergePlan } from './generated/ipc';
import { canMergeFrom, mergeConfirmBody, mergeConfirmSummary, mergeDoneMessage, mergeTargets, unmergeDoneMessage } from './workspaceMerge';
import type { WorkWorkspace } from './workspaceTypes';

const ws = (id: string, status = 'active', kind?: 'quick' | 'main'): WorkWorkspace =>
  ({ id, title: id.toUpperCase(), status, path: `workspaces/${id}`, cwd: `/v/workspaces/${id}`, modified: '', kind }) as WorkWorkspace;

const plan: MergePlan = {
  from: 'old', fromTitle: 'Old research', into: 'launch', intoTitle: 'Launch',
  files: [
    { from: 'workspaces/old/outputs/report.md', to: 'workspaces/launch/outputs/report-merged.md', renamed: true },
    { from: 'workspaces/old/sources/memo.pdf', to: 'workspaces/launch/sources/memo.pdf', renamed: false },
  ],
  sessions: [{ key: 's-1', title: 'Find Q3', archived: false }, { key: 's-2', title: '', archived: true }],
  left: ['outputs/link.md'], keepsWorktree: true, hash: 'h', blocked: null,
};

describe('workspace merge', () => {
  it('offers every other ordinary, unarchived workspace', () => {
    const all = [ws('a'), ws('b'), ws('c', 'archived'), ws('quick-chats', 'active', 'quick'), ws('main', 'active', 'main')];
    expect(mergeTargets(all, [all[1]], 'a').map(c => c.id)).toEqual(['b']);
  });

  it('refuses Notion mirrors and archived workspaces as the source', () => {
    expect(canMergeFrom(ws('a'))).toBe(true);
    // A legacy Notion workspace merges like any other since Notion sync was removed (2026-11-10).
    expect(canMergeFrom(ws('notion-00000000000000000000000000000002'))).toBe(true);
    expect(canMergeFrom(ws('notion-linked-edits-e676455ab944cf3b'))).toBe(true);
    expect(canMergeFrom(ws('a', 'Archived'))).toBe(false);
    // A merge that stopped part-way must be undone first.
    expect(canMergeFrom({ ...ws('a'), mergedInto: 'b' })).toBe(false);
  });

  it('lists every file (with renames) and every conversation', () => {
    const body = mergeConfirmBody(plan);
    expect(body).toContain('Files (2):\n• outputs/report.md → outputs/report-merged.md (name taken)\n• sources/memo.pdf');
    expect(body).toContain('Conversations (2):\n• Find Q3\n• s-2 (archived)');
    expect(body).toContain('git worktree stays');
    expect(body).toContain('“Old research” is then archived. Nothing is deleted, and Undo merge');
    expect(body).toContain('Left in “Old research” (symbolic links never move):\n• outputs/link.md');
  });

  it('says so when nothing but the docs move', () => {
    const body = mergeConfirmBody({ ...plan, files: [], sessions: [], left: [], keepsWorktree: false });
    expect(body).toContain('No files in sources/ or outputs/.');
    expect(body).toContain('No conversations.');
    expect(body).not.toContain('worktree');
    expect(body).not.toContain('Left in');
  });

  // P1-38: one line a screen reader reads with the title; the body says the
  // conversations change folder (as a single move's notice does).
  it('sums the merge up in one line', () => {
    expect(mergeConfirmSummary(plan)).toBe('Moves 2 files and 2 conversations into “Launch”, then archives “Old research”.');
    expect(mergeConfirmSummary({ ...plan, files: [plan.files[0]], sessions: [] })).toBe('Moves 1 file and no conversations into “Launch”, then archives “Old research”.');
  });

  it('says moved conversations run in the target’s folder from their next turn', () => {
    expect(mergeConfirmBody(plan)).toContain('The conversations’ next turns run in the folder of “Launch”, with its brief and sources; each conversation so far comes along as written context.');
    expect(mergeConfirmBody({ ...plan, sessions: [] })).not.toContain('next turns run');
  });

  it('sums up what moved', () => {
    expect(mergeDoneMessage({ plan, expiredCards: 0, warnings: [] })).toBe('Merged Old research into Launch (2 files, 2 conversations)');
    expect(unmergeDoneMessage({ from: 'old', into: 'launch', files: 1, sessions: 2, sessionKeys: ['s-1', 's-2'], warnings: [] }, 'Old research')).toBe('Undid the merge of Old research (1 file, 2 conversations back)');
  });
});
