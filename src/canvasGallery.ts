/**
 * Canvas gallery (overnight 10-11/12 F3): every canvas saved in a workspace's
 * store (outputs/canvases/), across workspaces, in one view. Pure, so the
 * node suite can load it; the overlay is `components/CanvasGallery.tsx`.
 * Viewing writes nothing.
 */
import type { CanvasSummary } from './generated/ipc';

export type GalleryItem = {
  workspaceId: string;
  workspaceTitle: string;
  id: string;
  title: string;
  revision: number;
  updatedAt: string | null;
  error: string | null;
};

/** One workspace's store as listed. */
export type GalleryStore = { workspaceId: string; workspaceTitle: string; canvases: readonly CanvasSummary[] };

/** Every listed canvas, newest first; one without a date (unreadable) goes last. */
export function galleryItems(stores: readonly GalleryStore[]): GalleryItem[] {
  const items: GalleryItem[] = [];
  for (const s of stores) {
    for (const c of s.canvases) {
      items.push({
        workspaceId: s.workspaceId, workspaceTitle: s.workspaceTitle, id: c.id, title: c.title || c.id,
        revision: c.revision, updatedAt: c.updatedAt, error: c.error,
      });
    }
  }
  return items.sort((a, b) => (b.updatedAt ?? '').localeCompare(a.updatedAt ?? ''));
}

/** Items matching every word of `query` (title, workspace or id, any case)
 *  and, when given, workspace `workspaceId`. */
export function filterGallery(items: readonly GalleryItem[], query: string, workspaceId: string | null): GalleryItem[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return items.filter(i => {
    if (workspaceId && i.workspaceId !== workspaceId) return false;
    if (!words.length) return true;
    const hay = `${i.title}\n${i.workspaceTitle}\n${i.id}`.toLowerCase();
    return words.every(w => hay.includes(w));
  });
}

export type ComparePair = { before: number; after: number };

/** The two revisions to show side by side: `chosen` while both still exist,
 *  else the one before `current` and `current`. Null with one version. */
export function comparePair(revisions: readonly number[], current: number, chosen: ComparePair | null): ComparePair | null {
  const kept = [...new Set(revisions)].sort((a, b) => a - b);
  if (kept.length < 2) return null;
  if (chosen && chosen.before !== chosen.after && kept.includes(chosen.before) && kept.includes(chosen.after)) return chosen;
  const after = kept.includes(current) ? current : kept[kept.length - 1];
  const before = [...kept].reverse().find(n => n < after) ?? kept.find(n => n !== after)!;
  return { before, after };
}

/** `fn` over `list`, at most `limit` at once, results in order. */
export async function mapLimit<T, R>(list: readonly T[], limit: number, fn: (item: T) => Promise<R>): Promise<R[]> {
  const out = new Array<R>(list.length);
  let next = 0;
  const worker = async () => {
    while (next < list.length) {
      const i = next++;
      out[i] = await fn(list[i]);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, list.length) }, worker));
  return out;
}
