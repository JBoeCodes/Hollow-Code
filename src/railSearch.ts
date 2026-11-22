import type { SessionTarget, WorkWorkspace } from './workspaceTypes';

/** One row of the sidebar's Search chats (overnight 11-04 S2). */
export type RailSearchRow =
  | { kind: 'project'; workspace: WorkWorkspace }
  | { kind: 'chat'; hit: SessionTarget }
  | { kind: 'everything' };

/** At most this many projects lead the results. */
export const RAIL_SEARCH_PROJECTS = 5;

/** The results, top to bottom: projects whose title has every word (not
 *  archived, at most five), then the chats `search_sessions` found, those
 *  whose title has every word first, each part newest first, then Search
 *  everything, which opens the command palette with the same words. With no
 *  words only Search everything shows. */
export function railSearchRows(query: string, projects: readonly WorkWorkspace[], hits: readonly SessionTarget[]): RailSearchRow[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const rows: RailSearchRow[] = [];
  if (words.length) {
    let shown = 0;
    for (const w of projects) {
      if (shown === RAIL_SEARCH_PROJECTS) break;
      if (w.status?.trim().toLowerCase() === 'archived') continue;
      const title = w.title.toLowerCase();
      if (words.every(word => title.includes(word))) { rows.push({ kind: 'project', workspace: w }); shown++; }
    }
    const titled = (hit: SessionTarget) => { const t = (hit.session.title ?? '').toLowerCase(); return words.every(word => t.includes(word)); };
    const chats = hits.filter(hit => hit.workspaceId);
    for (const hit of [...chats.filter(titled), ...chats.filter(hit => !titled(hit))]) rows.push({ kind: 'chat', hit });
  }
  rows.push({ kind: 'everything' });
  return rows;
}
