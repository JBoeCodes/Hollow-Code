import { describe, expect, it } from 'vitest';
import { isNotionMirrorId, repoDetail, unsavedText, worktreeUnsaved } from './workspaceTypes';

describe('worktree removal refusals', () => {
  it('reads the counts from the backend prefix, as a string or an Error', () => {
    expect(worktreeUnsaved('WORKTREE_UNSAVED: {"uncommitted":2,"unpushed":1}')).toEqual({ uncommitted: 2, unpushed: 1 });
    expect(worktreeUnsaved(new Error('WORKTREE_UNSAVED: {"uncommitted":0,"unpushed":3}'))).toEqual({ uncommitted: 0, unpushed: 3 });
    expect(worktreeUnsaved('An agent run is using the vault')).toBeNull();
    expect(worktreeUnsaved('WORKTREE_UNSAVED: not json')).toBeNull();
  });
  it('names only what is unsaved', () => {
    expect(unsavedText({ uncommitted: 1, unpushed: 0 })).toBe('1 uncommitted change');
    expect(unsavedText({ uncommitted: 3, unpushed: 1 })).toBe('3 uncommitted changes and 1 unpushed commit');
    expect(unsavedText({ uncommitted: 0, unpushed: 2 })).toBe('2 unpushed commits');
  });
});

describe('repository identity (audit H05)', () => {
  it('tells two copies of a repository apart', () => {
    expect(repoDetail({ name: 'hollow', path: '/p/hollow', parent: '', branch: 'main', worktree: false, remote: 'github.com/o/hollow' }))
      .toBe('Projects folder · on main · github.com/o/hollow');
    expect(repoDetail({ name: 'hollow', path: '/p/work/hollow', parent: 'work', branch: 'feature/x', worktree: true, remote: null }))
      .toBe('in work · on feature/x · linked worktree');
    expect(repoDetail({ name: 'x', path: '/p/x', parent: '', branch: null })).toBe('Projects folder · detached HEAD');
  });
  it('locates a chosen folder outside discovery by its containing path', () => {
    expect(repoDetail({ name: 'hollow', path: '/Users/me/Code/hollow' })).toBe('~/Code');
  });
});

describe('Notion mirror ids (overnight 10-11 Y2)', () => {
  it('needs the page id, not just the prefix', () => {
    expect(isNotionMirrorId('notion-00000000000000000000000000000002')).toBe(true);
    expect(isNotionMirrorId('notion-00000000-0000-4000-8000-000000000002')).toBe(true);
    expect(isNotionMirrorId('notion-linked-edits-e676455ab944cf3b')).toBe(false);
    expect(isNotionMirrorId('local-0aadea87cfecbd27b88cb0e6')).toBe(false);
  });
});
