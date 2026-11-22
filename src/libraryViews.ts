/**
 * Library saved views and note templates (next-level P06/P08). Pure,
 * unit-tested; the views persist in localStorage `hollow.library.views`.
 * A view is a named collection + query: a pinned collection is a view with
 * no query. Templates are plain Markdown; nothing renames existing notes.
 * A view also keeps whether it searched by meaning (P1-19); views saved
 * before that read as by name.
 */
import { SOURCE_COLLECTIONS, inSourceCollection } from './sourceCatalog';

export interface LibraryView { id: string; name: string; folder: string; query: string; byMeaning: boolean }

export function addView(views: readonly LibraryView[], v: Omit<LibraryView, 'id' | 'byMeaning'> & { byMeaning?: boolean }): LibraryView[] {
  const name = v.name.trim().slice(0, 60);
  if (!name) return [...views];
  const byMeaning = !!v.byMeaning;
  const same = views.find(x => x.folder === v.folder && x.query.trim() === v.query.trim() && x.byMeaning === byMeaning);
  if (same) return views.map(x => x === same ? { ...x, name } : x);
  return [...views, { ...v, name, query: v.query.trim(), byMeaning, id: `v${Date.now().toString(36)}${views.length}` }].slice(-12);
}
export const removeView = (views: readonly LibraryView[], id: string) => views.filter(v => v.id !== id);

export function readViews(raw: string | null): LibraryView[] {
  try { const v = JSON.parse(raw ?? '[]'); return Array.isArray(v) ? v.filter(x => x && typeof x.id === 'string' && typeof x.name === 'string' && typeof x.folder === 'string' && typeof x.query === 'string').map(x => ({ ...x, byMeaning: x.byMeaning === true })) : []; }
  catch { return []; }
}

/** Meaning hits inside the chosen collection or folder (P1-19): the index
 *  searches the whole vault, the page names one collection. */
export const meaningInCollection = <T extends { path: string }>(hits: readonly T[], collection: string): T[] => hits.filter(h => inSourceCollection(h.path, collection));

/** A collection's name as the folder picker shows it. */
export const collectionLabel = (collection: string) => SOURCE_COLLECTIONS.find(c => c.id === collection)?.label ?? `${collection}/`;

/** "12 in People by meaning"; all files: "12 files by meaning". */
export function meaningCountText(n: number, collection: string): string {
  return collection === 'all' ? `${n} ${n === 1 ? 'file' : 'files'} by meaning` : `${n} in ${collectionLabel(collection)} by meaning`;
}

export const NOTE_TEMPLATES = [
  { id: 'blank', label: 'Blank note', body: (t: string) => `# ${t}\n\n` },
  { id: 'meeting', label: 'Meeting notes', body: (t: string, d: string) => `# ${t}\n\n**Date:** ${d}\n**Attendees:** \n\n## Notes\n\n## Decisions\n\n- \n\n## Action items\n\n- [ ] \n` },
  { id: 'decision', label: 'Decision record', body: (t: string, d: string) => `# ${t}\n\n**Decided:** ${d}\n**Status:** proposed\n\n## Context\n\n## Options considered\n\n1. \n\n## Decision\n\n## Consequences\n` },
  { id: 'research', label: 'Research note', body: (t: string) => `# ${t}\n\n## Question\n\n## Findings\n\n- \n\n## Sources\n\n- \n\n## Open questions\n` },
  { id: 'weekly', label: 'Weekly review', body: (t: string, d: string) => `# ${t}\n\n**Week of:** ${d}\n\n## Done\n\n## Still open\n\n## Next week\n\n- [ ] \n` },
] as const;
export function templateBody(id: string, title: string, date: string): string {
  const t = NOTE_TEMPLATES.find(x => x.id === id) ?? NOTE_TEMPLATES[0];
  return t.body(title, date);
}
