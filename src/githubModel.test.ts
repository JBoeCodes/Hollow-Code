import { describe, expect, it } from 'vitest';
import {
  LINEAR_UNTRACKED, buildQuery, confirmCopy, discussionDraft, emptyWords, filtered, firstChoice, itemDraft, labelColor, linearLink, linearWords,
  movesFor, repoChoices, rowByline, rowChecksWords, rowStateWords, stateChoices, stateFor, type GhDetail, type Triage,
} from './githubModel';
import { WORK_VIEWS } from './store/destinations';
import { ACTION_BY_PALETTE } from './actionDefs';

const filters = { state: 'open' as const, label: '', assignee: '', author: '', search: '', reviewRequested: false };

const detail = (over: Partial<GhDetail> = {}): GhDetail => ({
  kind: 'pr', repo: 'o/r', author: 'ann', labels: [{ name: 'bug', color: 'd73a4a' }], assignees: [], createdAt: '', updatedAt: '', linear: [], body: 'Adds it.\nSecond line.',
  item: { number: 7, title: 'Add the thing', state: 'open', url: 'https://github.com/o/r/pull/7', draft: false, reviewDecision: null, mergeState: null, head: 'topic', base: 'main', checks: [], moreChecks: 0, comments: [], moreComments: 0 },
  ...over,
});

describe('the Pull requests list and its filters (parity 7.3)', () => {
  it('builds the query from the filters and leaves empty ones out', () => {
    expect(buildQuery('pr', filters)).toEqual({ kind: 'pr', state: 'open' });
    expect(buildQuery('issue', { ...filters, state: 'closed', label: ' bug ', assignee: '@me', search: ' crash ', reviewRequested: true }))
      .toEqual({ kind: 'issue', state: 'closed', label: 'bug', assignee: '@me', search: 'crash' });
    expect(buildQuery('pr', { ...filters, reviewRequested: true })).toEqual({ kind: 'pr', state: 'open', reviewRequested: true });
  });

  it('offers Merged for pull requests only, and falls back to Open when the kind changes', () => {
    expect(stateChoices('pr').map(c => c.value)).toEqual(['open', 'closed', 'merged', 'all']);
    expect(stateChoices('issue').map(c => c.value)).toEqual(['open', 'closed', 'all']);
    expect(stateFor('issue', 'merged')).toBe('open');
    expect(stateFor('pr', 'merged')).toBe('merged');
    expect(buildQuery('issue', { ...filters, state: 'merged' }).state).toBe('open');
  });

  it('says why a list is empty in words that fit the filters', () => {
    expect(filtered({ kind: 'pr', state: 'open' })).toBe(false);
    expect(filtered({ kind: 'pr', state: 'closed' })).toBe(true);
    expect(filtered({ kind: 'issue', state: 'open', label: 'bug' })).toBe(true);
    expect(emptyWords('pr', { kind: 'pr', state: 'open' })).toBe('No open pull requests.');
    expect(emptyWords('issue', { kind: 'issue', state: 'open', search: 'x' })).toBe('No issues match these filters.');
  });

  it('words a row: state, checks, byline and label colour', () => {
    expect(rowStateWords({ state: 'open', draft: true })).toBe('Draft');
    expect(rowStateWords({ state: 'merged', draft: false })).toBe('Merged');
    expect(rowChecksWords('failure')).toBe('Checks failing');
    expect(rowChecksWords('pending')).toBe('Checks running');
    expect(rowChecksWords(null)).toBeNull();
    expect(rowByline({ author: 'ann', comments: 1, assignees: [] })).toBe('ann · 1 comment');
    expect(rowByline({ author: 'ann', comments: 3, assignees: ['bob', 'cy'] })).toBe('ann · assigned to bob, cy · 3 comments');
    expect(labelColor('D73A4A')).toBe('#d73a4a');
    expect(labelColor('zz')).toBeNull();
    expect(labelColor('')).toBeNull();
  });
});

describe('triage is always asked first (parity 7.3, permission gating)', () => {
  const t = (action: Triage['action'], value?: string): Triage => ({ kind: 'pr', number: 7, action, value });

  it('every move shows a dialog that names the repository and the item', () => {
    for (const action of ['comment', 'label', 'unlabel', 'assign', 'unassign', 'close', 'reopen'] as const) {
      const c = confirmCopy('o/r', 'pr', t(action, action === 'comment' ? 'Ship it' : 'bug'), 'Add the thing');
      expect(c.body, action).toContain('o/r on GitHub');
      expect(c.body, action).toContain('pull request #7 (Add the thing)');
      expect(c.confirmLabel, action).toBeTruthy();
    }
  });

  it('a comment shows its exact text and says it is public; close is the red one', () => {
    const c = confirmCopy('o/r', 'issue', { kind: 'issue', number: 3, action: 'comment', value: ' Fixed in the next build. ' }, 'Crash');
    expect(c.body).toContain('Fixed in the next build.');
    expect(c.body).toContain('public');
    expect(c.confirmLabel).toBe('Post comment');
    expect(confirmCopy('o/r', 'pr', t('close'), 'x').danger).toBe(true);
    expect(confirmCopy('o/r', 'pr', t('reopen'), 'x').danger).toBe(false);
    expect(confirmCopy('o/r', 'pr', t('close'), 'x').body).toContain('unless it was merged');
    expect(confirmCopy('o/r', 'issue', { kind: 'issue', number: 1, action: 'close' }, 'x').body).not.toContain('merged');
  });

  it('names GitHub when the repository is unknown, and never offers merge', () => {
    expect(confirmCopy(null, 'pr', t('close'), 'x').body).toContain('in GitHub.');
    expect(['comment', 'label', 'unlabel', 'assign', 'unassign', 'close', 'reopen']).not.toContain('merge');
  });

  it('offers close while open and reopen while closed; a merged pull request offers neither', () => {
    expect(movesFor('open')).toEqual({ close: true, reopen: false });
    expect(movesFor('closed')).toEqual({ close: false, reopen: true });
    expect(movesFor('merged')).toEqual({ close: false, reopen: false });
  });
});

describe('Ask about this and Linear links (parity 7.3)', () => {
  it('drafts the whole item with its state, author, labels and quoted description', () => {
    const d = itemDraft(detail());
    expect(d).toContain('About pull request #7 in o/r: Add the thing');
    expect(d).toContain('State: open. By ann. Labels: bug.');
    expect(d).toContain('> Adds it.\n> Second line.');
    expect(d.endsWith('\n\n')).toBe(true);
    expect(itemDraft(detail({ repo: null, body: '', labels: [] }))).toBe('About pull request #7 in this repository: Add the thing\nState: open. By ann.\n\n');
  });

  it('drafts one comment the way the PR tab does', () => {
    expect(discussionDraft(detail(), { author: 'bob', kind: 'comment', state: null, body: 'Looks good' })).toContain('> Looks good');
    expect(discussionDraft(detail(), { author: 'cy', kind: 'review', state: 'CHANGES_REQUESTED', body: 'Fix it' })).toContain('requested changes');
  });

  it('links a mirrored Linear issue and leaves an unknown one as plain text', () => {
    expect(linearWords({ key: 'ENG-12', title: 'Fix the lot map' })).toBe('ENG-12 Fix the lot map');
    expect(linearWords({ key: 'ENG-99', title: null })).toBe('ENG-99');
    expect(linearLink({ url: 'https://linear.app/x/issue/ENG-12' })).toBe('https://linear.app/x/issue/ENG-12');
    expect(linearLink({ url: 'javascript:alert(1)' })).toBeNull();
    expect(linearLink({ url: null })).toBeNull();
    expect(LINEAR_UNTRACKED).toContain('no link');
  });
});

describe('which project the page lists', () => {
  const ws = [
    { id: 'q', title: 'Quick chats', cwd: '/q', kind: 'quick' },
    { id: 'm', title: 'COO', cwd: '/m', kind: 'main' },
    { id: 'a', title: 'App', cwd: '/a' },
    { id: 'b', title: 'Site', cwd: '/b' },
    { id: 'n', title: 'No folder', cwd: '' },
  ];
  it('lists real folders only, never Quick chats or the COO', () => {
    expect(repoChoices(ws).map(c => c.id)).toEqual(['a', 'b']);
  });
  it('starts on the open project, else the last one chosen, else the first', () => {
    const c = repoChoices(ws);
    expect(firstChoice(c, { open: 'b', saved: 'a' })?.id).toBe('b');
    expect(firstChoice(c, { open: 'zzz', saved: 'b' })?.id).toBe('b');
    expect(firstChoice(c, { open: null, saved: null })?.id).toBe('a');
    expect(firstChoice([], { open: null, saved: null })).toBeNull();
  });
});

describe('where the page lives', () => {
  it('is a Projects view, reachable by its tab and a palette row, with no new chord', () => {
    expect(WORK_VIEWS.map(v => v.id)).toEqual(['home', 'activity', 'pulls']);
    expect(WORK_VIEWS.find(v => v.id === 'pulls')?.label).toBe('Pull requests');
    const row = ACTION_BY_PALETTE['act:pull-requests'];
    expect(row.route).toBe('work.pulls');
    expect(row.key).toBeUndefined();
    expect(row.go).toBeUndefined();
    expect(row.effect).toBe('navigate');
  });
});
