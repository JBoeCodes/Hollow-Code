import { flatDescendants, isSubagentChat, splitChildren } from './railChildren';
import type { AgentSessionSummary } from './types';

/** Display roots include active descendants whose archived parent is absent
 * from the refreshed list. The sessions and full status graph stay intact. */
export function projectChildGraph(sessions: readonly AgentSessionSummary[], lead: string | null) {
  const split = splitChildren(sessions);
  if (!lead) return split.kids;
  const children = [...(split.kids.get(lead) ?? [])];
  const keys = new Set(children.map(s => s.key));
  for (const root of split.top) if (root.key !== lead && isSubagentChat(root) && !keys.has(root.key)) {
    children.push(root); keys.add(root.key);
  }
  split.kids.set(lead, children);
  return split.kids;
}

export function visibleProjectChildren(lead: string | null, kids: ReadonlyMap<string, readonly AgentSessionSummary[]>, visible: (s: AgentSessionSummary) => boolean): number {
  return lead ? flatDescendants(lead, kids).filter(visible).length : 0;
}
