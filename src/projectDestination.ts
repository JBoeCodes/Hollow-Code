import { isBuiltinWorkspace, isCurrentWorkspace, type WorkWorkspace, type WorkspaceSessions } from './workspaceTypes';
/** Never choose by pin order, alphabetical order, or a newly created brief's
 * mtime. Only a recorded conversation with real turns establishes recency. */
export function projectChoices(projects: readonly WorkWorkspace[], groups: Record<string, WorkspaceSessions>, exclude?: string | null) {
  return projects.filter(w => w.id !== exclude && !isBuiltinWorkspace(w) && isCurrentWorkspace(w.status) && !w.mergedInto && (!w.build || w.build.exists))
    .map(project => ({ project, activity: Math.max(0, ...(groups[project.id]?.sessions ?? []).filter(s => s.turnCount !== 0).map(s => Date.parse(s.lastActivityAt) || 0)) }))
    .sort((a,b) => b.activity - a.activity || a.project.title.localeCompare(b.project.title));
}
export const mostRecentProject = (projects: readonly WorkWorkspace[], groups: Record<string, WorkspaceSessions>, exclude?: string | null) => projectChoices(projects, groups, exclude).find(item => item.activity > 0)?.project.id ?? '';
