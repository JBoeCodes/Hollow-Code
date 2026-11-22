import { describe, expect, it } from 'vitest';
import { railSearchRows } from './railSearch';
import type { SessionTarget, WorkWorkspace } from './workspaceTypes';

const ws = (id: string, title: string, status = 'active'): WorkWorkspace =>
  ({ id, title, status, path: `workspaces/${id}`, cwd: `/v/workspaces/${id}`, modified: '' });
const hit = (key: string, workspaceId: string | null): SessionTarget =>
  ({ workspaceId, session: { key, title: key } as SessionTarget['session'] });
const kinds = (rows: ReturnType<typeof railSearchRows>) => rows.map(r => r.kind === 'project' ? `p:${r.workspace.id}` : r.kind === 'chat' ? `c:${r.hit.session.key}` : 'everything');

describe('sidebar Search chats rows (overnight 11-04 S2)', () => {
  it('lists matching projects, then the chats found, then Search everything', () => {
    const projects = [ws('q3', 'Q3 regional review'), ws('hire', 'Q4 hiring'), ws('old', 'Q3 archive', 'archived')];
    expect(kinds(railSearchRows('q3', projects, [hit('s-1', 'q3'), hit('s-2', 'quick-chats')]))).toEqual(['p:q3', 'c:s-1', 'c:s-2', 'everything']);
  });

  it('needs every word in a project title, ignores case and leaves out chats with no workspace', () => {
    const projects = [ws('a', 'Budget East'), ws('b', 'Budget West')];
    expect(kinds(railSearchRows('  budget   WEST ', projects, [hit('s-9', null)]))).toEqual(['p:b', 'everything']);
  });

  it('puts chats whose title has every word before ones found in what was asked', () => {
    const hits = [{ ...hit('s-new', 'a'), session: { key: 's-new', title: 'Weekly sync' } as SessionTarget['session'] }, { ...hit('s-old', 'a'), session: { key: 's-old', title: 'Garage lease renewal' } as SessionTarget['session'] }];
    expect(kinds(railSearchRows('garage lease', [], hits))).toEqual(['c:s-old', 'c:s-new', 'everything']);
  });

  it('shows at most five projects', () => {
    const projects = Array.from({ length: 8 }, (_, i) => ws(`p${i}`, `Plan ${i}`));
    expect(railSearchRows('plan', projects, []).filter(r => r.kind === 'project')).toHaveLength(5);
  });

  it('with no words, offers only Search everything', () => {
    expect(kinds(railSearchRows('   ', [ws('a', 'Anything')], [hit('s-1', 'a')]))).toEqual(['everything']);
  });
});
