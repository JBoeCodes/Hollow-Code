import type { LibraryResult } from './generated/ipc';
import type { GalleryItem } from './canvasGallery';
import { libraryCards, type ResultCard } from './resultsCards';

export type MixedCanvas = Omit<GalleryItem, 'revision'> & { revision: number | null };
export type MixedResult = {
  key: string; kind: 'file' | 'canvas'; workspaceId: string; workspaceTitle: string;
  title: string; name: string; modified: string | null; card: ResultCard | null; canvas: MixedCanvas | null;
};
export type MixedFilters = { query: string; workspace: string; type: 'all' | 'files' | 'canvases' };

const knownDate = (value: string | null): string | null => value && Number.isFinite(Date.parse(value)) ? value : null;
const canvasKey = (workspace: string, id: string) => JSON.stringify(['canvas', workspace, id]);

/** Metadata only. File rows paint before store listings settle; saved canvases
 * enter from store metadata, and matching output rows retain their review evidence.
 * Unknown review state is a null card, never an invented accepted/draft value. */
export function mixedLibraryResults(files: readonly LibraryResult[], canvases: readonly GalleryItem[]): MixedResult[] {
  const rows = new Map<string, MixedResult>();
  for (const r of libraryCards(files)) {
    const saved = r.card.canvas;
    const key = saved ? canvasKey(r.workspaceId, saved.id) : JSON.stringify(['file', r.workspaceId, r.card.path]);
    rows.set(key, {
      key, kind: saved ? 'canvas' : 'file', workspaceId: r.workspaceId, workspaceTitle: r.workspaceTitle,
      title: r.card.title, name: r.card.name, modified: knownDate(r.card.modified), card: r.card,
      canvas: saved ? {
        workspaceId: r.workspaceId, workspaceTitle: r.workspaceTitle, id: saved.id,
        title: r.card.title, revision: saved.revision, updatedAt: knownDate(r.card.modified), error: null,
      } : null,
    });
  }
  for (const canvas of canvases) {
    const key = canvasKey(canvas.workspaceId, canvas.id);
    const existing = rows.get(key);
    const title = canvas.error ? existing?.title ?? canvas.title : canvas.title;
    const modified = knownDate(canvas.updatedAt) ?? (canvas.error ? existing?.modified ?? null : null);
    const revision = canvas.error ? existing?.canvas?.revision ?? null : canvas.revision;
    rows.set(key, {
      key, kind: 'canvas', workspaceId: canvas.workspaceId, workspaceTitle: canvas.workspaceTitle,
      title, name: existing?.name ?? canvas.id, modified,
      card: existing?.card ?? null, canvas: { ...canvas, title, revision, updatedAt: modified },
    });
  }
  return [...rows.values()].map(row => ({ row, at: row.modified === null ? null : Date.parse(row.modified) }))
    .sort((a, b) => a.at === null ? b.at === null ? 0 : 1 : b.at === null ? -1 : b.at - a.at)
    .map(({ row }) => row);
}
export function filterMixedResults(rows: readonly MixedResult[], filters: MixedFilters): MixedResult[] {
  const words = filters.query.toLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter(r => {
    if (filters.workspace && r.workspaceId !== filters.workspace) return false;
    if (filters.type !== 'all' && r.kind !== (filters.type === 'files' ? 'file' : 'canvas')) return false;
    const hay = `${r.title}\n${r.name}\n${r.workspaceTitle}\n${r.canvas?.id ?? ''}`.toLowerCase();
    return words.every(w => hay.includes(w));
  });
}
export function resetMixedFilters(filters: MixedFilters, scope: 'search' | 'filters'): MixedFilters {
  return scope === 'search' ? { ...filters, query: '' } : { ...filters, workspace: '', type: 'all' };
}
