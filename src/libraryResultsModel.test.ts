import { describe, expect, it } from 'vitest';
import type { LibraryResult } from './generated/ipc';
import type { GalleryItem } from './canvasGallery';
import { filterMixedResults, mixedLibraryResults, resetMixedFilters } from './libraryResultsModel';

const file = (over: Partial<LibraryResult> = {}): LibraryResult => ({
  workspaceId: 'one', workspaceTitle: 'First workspace', path: 'workspaces/one/outputs/a.md',
  name: 'outputs/a.md', title: 'Written report', kind: 'document', size: 100,
  modified: '2026-10-27T01:00:00Z', sha256: null, review: 'draft', acceptedSha256: null,
  acceptedAt: null, readyAt: null, hasSnapshot: false, acceptance: null, history: [], origin: null,
  checks: { total: null, failed: null, unknown: null }, ...over,
});
const canvas = (over: Partial<GalleryItem> = {}): GalleryItem => ({
  workspaceId: 'one', workspaceTitle: 'First workspace', id: 'chart', title: 'Revenue chart',
  revision: 3, updatedAt: '2026-10-27T02:00:00Z', error: null, ...over,
});

describe('one mixed Library Results list', () => {
  it('lists ordinary files and saved canvases together without inventing a review', () => {
    const rows = mixedLibraryResults([file()], [canvas({ id: 'plan' })]);
    expect(rows.map(r => r.kind)).toEqual(['canvas', 'file']);
    expect(rows[0].canvas?.id).toBe('plan');
    expect(rows[0].card).toBeNull();
    expect(rows[1].card?.review).toBe('draft');
  });
  it('deduplicates a listed canvas by workspace/id and retains its review evidence', () => {
    const listed = file({ kind: 'canvas', canvas: { id: 'chart', revision: 2 }, review: 'changed' });
    const rows = mixedLibraryResults([listed], [canvas()]);
    expect(rows).toHaveLength(1);
    expect(rows[0].canvas?.revision).toBe(3);
    expect(rows[0].card?.review).toBe('changed');
    expect(rows[0].card?.path).toBe(listed.path);
  });
  it('retains listed canvases while stores load and keeps their key after metadata arrives', () => {
    const listed = file({ kind: 'canvas', canvas: { id: 'chart', revision: 2 } });
    const early = mixedLibraryResults([listed, file()], []);
    const settled = mixedLibraryResults([listed, file()], [canvas()]);
    expect(early).toHaveLength(2);
    expect(early.find(r => r.kind === 'canvas')).toBeDefined();
    expect(early.find(r => r.kind === 'canvas')?.key).toBe(settled.find(r => r.kind === 'canvas')?.key);
    expect(early.find(r => r.kind === 'file')?.card?.title).toBe('Written report');
  });
  it('never merges identically named canvases from different workspaces', () => {
    const rows = mixedLibraryResults([], [canvas(), canvas({ workspaceId: 'two' })]);
    expect(new Set(rows.map(r => r.key)).size).toBe(2);
  });
  it('keeps unreadable canvases explicit, with unknown dates last rather than epoch zero', () => {
    const rows = mixedLibraryResults([file({ modified: 'invalid' })], [
      canvas({ id: 'broken', updatedAt: null, error: 'Cannot read store entry' }), canvas(),
    ]);
    expect(rows[0].canvas?.id).toBe('chart');
    expect(rows.find(r => r.canvas?.id === 'broken')?.canvas?.error).toBe('Cannot read store entry');
    expect(rows.find(r => r.canvas?.id === 'broken')?.modified).toBeNull();
    expect(rows.find(r => r.kind === 'file')?.modified).toBeNull();
  });
  it('hides bookkeeping files but keeps their store metadata entry', () => {
    const rows = mixedLibraryResults([file({ name: 'outputs/canvases/plan.canvas.json' })], [canvas({ id: 'plan' })]);
    expect(rows).toHaveLength(1);
    expect(rows[0].canvas?.id).toBe('plan');
  });
  it('keeps known file metadata when the canvas store entry is unreadable', () => {
    const listed = file({ kind: 'canvas', canvas: { id: 'chart', revision: 2 }, title: 'Known chart', review: 'accepted' });
    const [row] = mixedLibraryResults([listed], [canvas({ title: 'chart.canvas.json', revision: 0, updatedAt: null, error: 'Unreadable' })]);
    expect(row.title).toBe('Known chart');
    expect(row.modified).toBe(listed.modified);
    expect(row.canvas).toMatchObject({ revision: 2, title: 'Known chart', error: 'Unreadable' });
    expect(row.card?.review).toBe('accepted');
  });
  it('does not turn an unreadable store placeholder into revision zero', () => {
    const [row] = mixedLibraryResults([], [canvas({ revision: 0, updatedAt: null, error: 'Unreadable' })]);
    expect(row.canvas?.revision).toBeNull();
  });
  it('applies type, workspace and every search word to the same list', () => {
    const rows = mixedLibraryResults([file()], [canvas(), canvas({ workspaceId: 'two', title: 'Other chart' })]);
    expect(filterMixedResults(rows, { query: 'chart FIRST', workspace: 'one', type: 'canvases' })).toHaveLength(1);
    expect(filterMixedResults(rows, { query: '', workspace: '', type: 'files' })).toHaveLength(1);
    expect(filterMixedResults(rows, { query: '', workspace: '', type: 'all' })).toHaveLength(3);
  });
  it('resets search separately from workspace/type without a review filter', () => {
    const filters = { query: 'chart', workspace: 'one', type: 'canvases' as const };
    expect(resetMixedFilters(filters, 'search')).toEqual({ ...filters, query: '' });
    expect(resetMixedFilters(filters, 'filters')).toEqual({ query: 'chart', workspace: '', type: 'all' });
    expect(filters.query).toBe('chart');
  });
});
