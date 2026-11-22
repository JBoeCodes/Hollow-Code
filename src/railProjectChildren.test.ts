import { describe, expect, it } from 'vitest';
import { flatDescendants } from './railChildren';
import { projectChildGraph, visibleProjectChildren } from './railProjectChildren';
import type { AgentSessionSummary } from './types';

const row = (key: string, parentKey: string | null = null) => ({ key, parentKey, forkKind: parentKey ? 'subagent' : null, createdAt: key } as AgentSessionSummary);
describe('project child display after a section move', () => {
  it('retains descendants when their parent row belongs to another shelf', () => {
    const kids = projectChildGraph([row('lead'), row('parent', 'lead'), row('child', 'parent')], 'lead');
    expect(visibleProjectChildren('lead', kids, s => s.key !== 'parent')).toBe(1);
    expect(flatDescendants('lead', kids).map(s => s.key)).toEqual(['parent', 'child']);
  });
  it('keeps orphaned descendants reachable after the archived parent leaves the list', () => {
    const kids = projectChildGraph([row('lead'), row('child', 'parent'), row('grandchild', 'child'), row('ordinary')], 'lead');
    expect(flatDescendants('lead', kids).map(s => s.key)).toEqual(['child', 'grandchild']);
    expect(visibleProjectChildren('lead', kids, () => true)).toBe(2);
  });
  it('deduplicates malformed cycles and handles an unloaded lead', () => {
    const sessions = [row('a', 'b'), row('b', 'a')];
    const kids = projectChildGraph(sessions, 'lead');
    expect(new Set(flatDescendants('lead', kids).map(s => s.key)).size).toBe(2);
    expect(visibleProjectChildren(null, kids, () => true)).toBe(0);
    expect(sessions).toHaveLength(2);
  });
});
