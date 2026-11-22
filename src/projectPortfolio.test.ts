import { describe, expect, it } from 'vitest';
import type { ProjectPortfolio, TrackerViewRow } from './generated/ipc';
import { changedValues, draftOf, draftProblems, isVaultPath, itemLabel, percent, projectCounts } from './projectPortfolio';

const record = (over: Partial<ProjectPortfolio> = {}): ProjectPortfolio => ({
  workspaceId: 'ws', path: 'workspaces/ws/PROJECT.md', exists: true, title: 'Fleet', status: null, targetDate: null, description: '',
  tracked: null, text: { department: 'AI', type: '', hours_saved_per_year: '', value_per_year: '', owners_note: '', github_url: '', live_url: '' },
  lists: { requested_by: [], stakeholders: [], domains: ['library/records/domains/hr.md'], meetings: [], captures: [], war_room: [], docs: [] },
  revision: 'r1', ...over,
});
const row = (statusKind: TrackerViewRow['statusKind']) => ({ statusKind } as TrackerViewRow);

describe('a project record (B3)', () => {
  it('a draft sends only what changed, lists one item per line', () => {
    const r = record();
    const draft = draftOf(r);
    expect(draft.domains).toBe('library/records/domains/hr.md');
    expect(changedValues(r, draft)).toEqual({});
    expect(changedValues(r, { ...draft, department: ' Ops ', meetings: 'a.md\n\n b.md ' })).toEqual({ department: 'Ops', meetings: ['a.md', 'b.md'] });
    expect(changedValues(r, { ...draft, domains: '' })).toEqual({ domains: [] });
  });

  it('numbers and web addresses are checked before saving', () => {
    expect(draftProblems({ hours_saved_per_year: '1,200', value_per_year: '$40,000', live_url: 'https://x.example' })).toEqual({});
    expect(Object.keys(draftProblems({ hours_saved_per_year: 'lots', github_url: 'example.com' }))).toEqual(['hours_saved_per_year', 'github_url']);
  });

  it('progress is completed over all of the project’s issues', () => {
    expect(projectCounts([row('completed'), row('started'), row('canceled'), row('backlog')])).toEqual({ total: 4, done: 1, open: 2, canceled: 1, progress: 0.25 });
    expect(projectCounts([]).progress).toBeNull();
    expect(percent(0.25)).toBe('25%');
    expect(percent(null)).toBeNull();
  });

  it('vault paths in lists open in Library and read as their names', () => {
    expect(isVaultPath('library/records/meetings/2026/2026-11-04-receipts-kickoff.md')).toBe(true);
    expect(isVaultPath('Dana Kim')).toBe(false);
    expect(itemLabel('library/records/meetings/2026/2026-11-04-receipts-kickoff.md')).toBe('receipts kickoff');
    expect(itemLabel('Dana Kim')).toBe('Dana Kim');
  });
});
