import { describe, expect, it } from 'vitest';
import { comparePair, filterGallery, galleryItems, mapLimit, type GalleryItem } from './canvasGallery';

const summary = (id: string, title: string, updatedAt: string | null, revision = 1, error: string | null = null) =>
  ({ id, title, revision, updatedAt, fenceSha256: null, path: null, file: null, error });

describe('canvas gallery (overnight 10-11/12 F3)', () => {
  it('lists every workspace’s canvases newest first', () => {
    const items = galleryItems([
      { workspaceId: 'w1', workspaceTitle: 'Release', canvases: [summary('status', 'Release status', '2026-10-11T10:00:00Z', 3), summary('plan', 'Plan', '2026-10-12T09:00:00Z')] },
      { workspaceId: 'w2', workspaceTitle: 'Hiring', canvases: [summary('pipeline', 'Pipeline', '2026-10-12T01:00:00Z')] },
    ]);
    expect(items.map(i => `${i.workspaceId}/${i.id}`)).toEqual(['w1/plan', 'w2/pipeline', 'w1/status']);
    expect(items[0]).toMatchObject({ title: 'Plan', revision: 1 });
    expect(items[2]).toMatchObject({ revision: 3, workspaceTitle: 'Release' });
  });

  it('keeps an unreadable canvas, with its error, after the readable ones of its age', () => {
    const items = galleryItems([{ workspaceId: 'w1', workspaceTitle: 'A', canvases: [summary('bad', 'bad', null, 0, 'unreadable sidecar'), summary('ok', 'Ok', '2026-10-01T00:00:00Z')] }]);
    expect(items.map(i => i.id)).toEqual(['ok', 'bad']);
    expect(items[1].error).toBe('unreadable sidecar');
  });

  const items: GalleryItem[] = galleryItems([
    { workspaceId: 'w1', workspaceTitle: 'Release train', canvases: [summary('status', 'Release status', '2026-10-11T10:00:00Z')] },
    { workspaceId: 'w2', workspaceTitle: 'Hiring', canvases: [summary('pipeline', 'Candidate pipeline', '2026-10-12T01:00:00Z'), summary('budget-q4', 'Q4 budget', '2026-10-10T01:00:00Z')] },
  ]);

  it('searches title, workspace and id, every word, any case', () => {
    expect(filterGallery(items, '', null).length).toBe(3);
    expect(filterGallery(items, 'release', null).map(i => i.id)).toEqual(['status']);
    expect(filterGallery(items, 'HIRING budget', null).map(i => i.id)).toEqual(['budget-q4']);
    expect(filterGallery(items, 'budget-q4', null).map(i => i.id)).toEqual(['budget-q4']);
    expect(filterGallery(items, 'nothing like it', null)).toEqual([]);
  });

  it('filters by workspace', () => {
    expect(filterGallery(items, '', 'w2').map(i => i.id)).toEqual(['pipeline', 'budget-q4']);
    expect(filterGallery(items, 'pipeline', 'w1')).toEqual([]);
  });

  it('compares the previous version with the current one by default, and keeps a valid choice', () => {
    expect(comparePair([1, 2, 3], 3, null)).toEqual({ before: 2, after: 3 });
    expect(comparePair([1, 2, 3], 3, { before: 1, after: 2 })).toEqual({ before: 1, after: 2 });
    // A choice that no longer exists (pruned) falls back.
    expect(comparePair([2, 3], 3, { before: 1, after: 3 })).toEqual({ before: 2, after: 3 });
    // One version: nothing to compare.
    expect(comparePair([1], 1, null)).toBeNull();
  });

  it('reads many stores with a bounded number at once, in order', async () => {
    let running = 0, most = 0;
    const out = await mapLimit([1, 2, 3, 4, 5, 6, 7], 3, async n => {
      running++; most = Math.max(most, running);
      await new Promise(r => setTimeout(r, 2));
      running--;
      return n * 10;
    });
    expect(out).toEqual([10, 20, 30, 40, 50, 60, 70]);
    expect(most).toBe(3);
  });
});
