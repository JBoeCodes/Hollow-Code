import type { WorkWorkspace, WorkspaceSessions } from './workspaceTypes';
import { monthDay } from './monthDay';
import { ICON_MEANING } from './iconMeaning';

export const WORKSPACE_STARTERS = [
  { id: 'research', icon: 'search', title: 'Research a question', description: 'Compare the evidence and write a brief.', name: 'Research brief', goal: 'Research this question: …\nCompare the evidence, cite sources, and save a concise brief with a recommendation in outputs/.', kind: 'general' },
  { id: 'create', icon: ICON_MEANING.draft, title: 'Draft a document', description: 'Turn notes into a first draft.', name: 'Creative project', goal: 'Create a polished first draft of: …\nClarify the audience and purpose, propose a direction, and save it as a Result in outputs/.', kind: 'general' },
  { id: 'review', icon: 'checkSquare', title: 'Review material', description: 'Find the risks and open decisions.', name: 'Decision brief', goal: 'Review these materials: …\nIdentify the important changes, risks, and open decisions. Save an evidence-backed recommendation in outputs/.', kind: 'general' },
  { id: 'build', icon: 'terminal', title: 'Build in a repository', description: 'A new branch in its own worktree.', name: 'Build a feature', goal: 'Implement this change: …\nRead the project instructions, make a focused plan, implement and verify the result, and update the project documentation.', kind: 'build' },
] as const;
export type WorkspaceStarter = typeof WORKSPACE_STARTERS[number];
export type WorkspaceSort = 'recent' | 'name';
const timestamp = (value: string) => Number.isFinite(Date.parse(value)) ? Date.parse(value) : 0;

/** A conversation timestamp is activity; a brief mtime is only a fallback. */
export function latestWorkspaceSession(group?: WorkspaceSessions) {
  return group?.sessions.reduce<WorkspaceSessions['sessions'][number] | undefined>((latest, s) =>
    !latest || timestamp(s.lastActivityAt) > timestamp(latest.lastActivityAt) ? s : latest, undefined);
}
export function orderWorkspaces(list: WorkWorkspace[], groups: Record<string, WorkspaceSessions>, pins: string[], sort: WorkspaceSort) {
  // Pinned first, in pin order (the sidebar's Pinned group's order).
  const pinned = new Map(pins.map((id, i) => [id, i]));
  // Each workspace's last activity once, not per comparison (P4: 200
  // workspaces re-sorted on every keystroke in Find a workspace).
  const at = new Map(list.map(w => [w, sort === 'name' ? 0 : timestamp(latestWorkspaceSession(groups[w.id])?.lastActivityAt ?? w.modified)]));
  const rank = (w: WorkWorkspace) => pinned.get(w.id) ?? pins.length;
  return [...list].sort((a, b) => rank(a) - rank(b) ||
    (sort === 'name' ? a.title.localeCompare(b.title) : at.get(b)! - at.get(a)!) || a.title.localeCompare(b.title));
}
// One formatter for every row (ST2, 2026-10-14; monthDay.ts): a new
// Intl.DateTimeFormat per call was ~12 ms of a cold launch to Home at 200
// workspaces.
export function studioDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Date unavailable' : monthDay(date);
}
