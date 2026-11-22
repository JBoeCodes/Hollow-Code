/**
 * Where a workspace comes from, in words, and which titles need it to be told
 * apart (resume-first Home, consolidation E, 2026-10-13). A real library has
 * a local project folder, a Notion mirror and a Hollow workspace with the
 * same name; the title alone can't say which is which. The rail, Home's cards
 * and the palette all use these, so the same workspace reads the same way
 * everywhere. Pure; unit-tested.
 */
import { isNotionMirrorId, type WorkWorkspace } from './workspaceTypes';

type Origin = Pick<WorkWorkspace, 'id' | 'title' | 'path' | 'build'>;

/** "Build · feature/x", "Notion project", "Project folder · Acme", or
 *  "Hollow workspace". */
export function workspaceOrigin(w: Origin, owner?: string | null): string {
  if (w.build) return `Build · ${w.build.branch}`;
  if (isNotionMirrorId(w.id)) return 'Notion project';
  if (w.id.startsWith('local-')) return owner ? `Project folder · ${owner}` : 'Project folder';
  return 'Hollow workspace';
}

const norm = (title: string) => title.trim().toLowerCase().replace(/\s+/g, ' ');

/** Ids of workspaces whose title another workspace also has. */
export function duplicateTitleIds(all: readonly Pick<WorkWorkspace, 'id' | 'title'>[]): Set<string> {
  const byTitle = new Map<string, string[]>();
  for (const w of all) { const k = norm(w.title); byTitle.set(k, [...(byTitle.get(k) ?? []), w.id]); }
  return new Set([...byTitle.values()].filter(ids => ids.length > 1).flat());
}

/** The list grouped by normalised title, built once per list (ST2,
 *  2026-10-14). The rail, Home and the top bar ask about every row: a scan of
 *  the whole list per row was ~31 ms per Home or chat switch at 200
 *  workspaces. Keyed by the array itself, which the stores replace rather than
 *  mutate, so a changed list gets a fresh index. */
const indexes = new WeakMap<readonly Origin[], Map<string, Origin[]>>();
function titleIndex(all: readonly Origin[]): Map<string, Origin[]> {
  let index = indexes.get(all);
  if (!index) {
    index = new Map();
    for (const o of all) { const k = norm(o.title); const list = index.get(k); if (list) list.push(o); else index.set(k, [o]); }
    indexes.set(all, index);
  }
  return index;
}

/** What tells this workspace apart from others with its title: its origin,
 *  and when the origin is shared too, the last folder of its path. Null when
 *  the title is unique. */
export function distinguishing(w: Origin, all: readonly Origin[], owner?: string | null): string | null {
  const twins = (titleIndex(all).get(norm(w.title)) ?? []).filter(o => o.id !== w.id);
  if (!twins.length) return null;
  const origin = workspaceOrigin(w, owner);
  const sameOrigin = twins.some(o => workspaceOrigin(o) === workspaceOrigin(w));
  if (!sameOrigin) return origin;
  const folder = w.path.split('/').filter(Boolean).pop() ?? w.id;
  return `${origin} · ${folder}`;
}

/** The full name and origin, for a tooltip on a row whose title may be cut:
 *  "Q3 review (Notion project)". `twin` (from `distinguishing`) replaces the
 *  origin where the origin alone doesn't tell same-named workspaces apart. */
export function workspaceTooltip(w: Origin, owner?: string | null, twin?: string | null): string {
  return `${w.title} (${twin || workspaceOrigin(w, owner)})`;
}
