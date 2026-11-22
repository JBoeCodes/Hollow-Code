import { describe, expect, it } from 'vitest';
import type { TrackerProjectRow } from '../../generated/ipc';
import { orderProjects } from './ProjectList';

const p = (name: string, statusKind: string): TrackerProjectRow => ({
  id: name, key: null, name, statusName: statusKind, statusKind, lead: null, targetDate: null, progress: null,
  issueCount: 0, openCount: 0, url: '', workspaceId: null, teamKey: 'ENG',
});

describe('the Projects tab (B4)', () => {
  it('lists active projects first, closed ones last, then by name', () => {
    const order = orderProjects([p('Zed', 'completed'), p('Beta', 'planned'), p('Alpha', 'started'), p('Odd', 'weird'), p('Gamma', 'started'), p('Paused', 'paused')]);
    expect(order.map(x => x.name)).toEqual(['Alpha', 'Gamma', 'Beta', 'Paused', 'Zed', 'Odd']);
  });
});
