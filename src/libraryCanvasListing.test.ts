import { expect, it, vi } from 'vitest';
import { canvasStoreListing } from './libraryCanvasListing';

const workspaces = Array.from({ length: 18 }, (_, n) => ({ id: String(n), title: `Workspace ${n}` }));
const flush = async () => { for (let n = 0; n < 30; n++) await Promise.resolve(); };
const retainedCanvas = { id: '_plan', title: 'Known plan', revision: 2, updatedAt: '2026-10-27T01:00:00Z', error: null,
  fenceSha256: 'a'.repeat(64), path: 'outputs/canvases/_plan.canvas.html', file: 'outputs/canvases/_plan.canvas.json' };

it('retains last-good store-only canvases on a failed refresh, then accepts verified deletion', async () => {
  const read = vi.fn().mockResolvedValueOnce([retainedCanvas]).mockRejectedValueOnce(new Error('Unreadable')).mockResolvedValueOnce([]);
  const deliver = vi.fn();
  const listing = canvasStoreListing(read, deliver);
  const scope = [{ id: 'one', title: 'First workspace' }];
  listing.load(scope); await flush();
  const first = deliver.mock.calls.at(-1)![0].items;
  expect(first).toHaveLength(1);
  listing.load(scope); await flush();
  expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: first, failed: ['First workspace'] });
  listing.load(scope); await flush();
  expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: [], failed: [] });
  listing.dispose();
});

it('does not retain removed workspaces or resurrect their cache when later reads fail', async () => {
  let fail = false;
  const read = vi.fn(async () => { if (fail) throw new Error('Unreadable'); return [retainedCanvas]; });
  const deliver = vi.fn();
  const listing = canvasStoreListing(read, deliver);
  const scope = [{ id: 'one', title: 'First workspace' }];
  listing.load(scope); await flush();
  expect(deliver.mock.calls.at(-1)![0].items).toHaveLength(1);
  listing.load([]); await flush();
  expect(deliver.mock.calls.at(-1)![0].items).toEqual([]);
  fail = true;
  listing.load(scope); await flush();
  expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: [], failed: ['First workspace'] });
  listing.dispose();
});

it('retains a timed-out store without issuing a second native read while it is still hung', async () => {
  vi.useFakeTimers();
  try {
    const read = vi.fn().mockResolvedValueOnce([retainedCanvas]).mockImplementation(() => new Promise(() => {}));
    const deliver = vi.fn();
    const listing = canvasStoreListing(read, deliver);
    const scope = [{ id: 'one', title: 'First workspace' }];
    listing.load(scope); await flush();
    const first = deliver.mock.calls.at(-1)![0].items;
    listing.load(scope); await vi.advanceTimersByTimeAsync(15_001);
    expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: first, failed: ['First workspace'] });
    listing.load(scope); await flush();
    expect(read).toHaveBeenCalledTimes(2);
    expect(deliver.mock.calls.at(-1)![0].items).toEqual(first);
    listing.dispose();
  } finally { vi.useRealTimers(); }
});

it('cannot repopulate a removed workspace from an overtaken read', async () => {
  let finish!: (value: typeof retainedCanvas[]) => void;
  const read = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockRejectedValue(new Error('Unreadable after re-add'));
  const deliver = vi.fn();
  const listing = canvasStoreListing(read, deliver);
  const scope = [{ id: 'one', title: 'First workspace' }];
  listing.load(scope);
  listing.load([]);
  finish([retainedCanvas]); await flush();
  expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: [], failed: [] });
  listing.load(scope); await flush();
  expect(deliver.mock.calls.at(-1)![0]).toEqual({ items: [], failed: ['First workspace'] });
  listing.dispose();
});

it('releases successful native slots before continuing through a larger workspace list', async () => {
  const read = vi.fn(async () => []);
  const deliver = vi.fn();
  const listing = canvasStoreListing(read, deliver);
  listing.load(workspaces); await flush();
  expect(read).toHaveBeenCalledTimes(18);
  expect(deliver.mock.calls[0][0].failed).toEqual([]);
});

it('caps overlapping refreshes at six reads and delivers only the newest listing', async () => {
  let active = 0, peak = 0;
  const pending: (() => void)[] = [];
  const deliver = vi.fn();
  const listing = canvasStoreListing(async () => {
    active++; peak = Math.max(peak, active);
    await new Promise<void>(resolve => pending.push(resolve)); active--; return [];
  }, deliver);
  listing.load(workspaces);
  expect(active).toBe(6);
  listing.load([{ id: 'new', title: 'Newest' }]);
  expect(active).toBe(6);
  pending.splice(0).forEach(resolve => resolve()); await flush();
  expect(deliver).not.toHaveBeenCalled();
  expect(active).toBe(1);
  pending.splice(0).forEach(resolve => resolve()); await flush();
  expect(peak).toBe(6);
  expect(deliver).toHaveBeenCalledOnce();
  expect(deliver.mock.calls[0][0]).toEqual({ items: [], failed: [] });
});

it('keeps failed workspace coverage separate from a verified empty store', async () => {
  const deliver = vi.fn();
  const listing = canvasStoreListing(async id => { if (id === 'bad') throw new Error('Denied'); return []; }, deliver);
  listing.load([{ id: 'bad', title: 'Unreadable' }, { id: 'ok', title: 'Empty' }]);
  await flush();
  expect(deliver.mock.calls[0][0]).toEqual({ items: [], failed: ['Unreadable'] });
});

it('cancels delivery and stops queued reads after disposal', async () => {
  const pending: (() => void)[] = [];
  const read = vi.fn(async () => { await new Promise<void>(r => pending.push(r)); return []; });
  const deliver = vi.fn();
  const listing = canvasStoreListing(read, deliver);
  listing.load(workspaces); listing.dispose();
  pending.splice(0).forEach(r => r()); await flush();
  expect(read).toHaveBeenCalledTimes(6);
  expect(deliver).not.toHaveBeenCalled();
});

it('settles timed-out coverage without exceeding six still-pending native reads on retry', async () => {
  vi.useFakeTimers();
  try {
    const read = vi.fn(() => new Promise<never>(() => {}));
    const deliver = vi.fn();
    const listing = canvasStoreListing(read, deliver);
    listing.load(workspaces);
    await vi.advanceTimersByTimeAsync(45_001);
    expect(deliver).toHaveBeenCalledOnce();
    expect(deliver.mock.calls[0][0].failed).toHaveLength(18);
    listing.load(workspaces); await vi.advanceTimersByTimeAsync(30_001);
    expect(deliver).toHaveBeenCalledTimes(2);
    expect(read).toHaveBeenCalledTimes(6);
    listing.dispose();
  } finally { vi.useRealTimers(); }
});

it('retries seventeen healthy stores beside one hung read without duplicating it', async () => {
  vi.useFakeTimers();
  try {
    let active = 0, peak = 0;
    const read = vi.fn(async (id: string) => {
      active++; peak = Math.max(peak, active);
      if (id === '0') return new Promise<never>(() => {});
      await new Promise(resolve => setTimeout(resolve, 10));
      active--; return [];
    });
    const deliver = vi.fn();
    const listing = canvasStoreListing(read, deliver);
    for (let retry = 0; retry < 4; retry++) {
      listing.load(workspaces);
      await vi.advanceTimersByTimeAsync(15_001);
      expect(deliver.mock.calls.at(-1)?.[0].failed).toEqual(['Workspace 0']);
    }
    expect(deliver).toHaveBeenCalledTimes(4);
    expect(read.mock.calls.filter(([id]) => id === '0')).toHaveLength(1);
    expect(read).toHaveBeenCalledTimes(1 + 17 * 4);
    expect(peak).toBeLessThanOrEqual(6);
    listing.dispose();
  } finally { vi.useRealTimers(); }
});

it('delivers completed coverage despite continuous same-scope refreshes', async () => {
  const pending: (() => void)[] = [];
  const deliver = vi.fn();
  const listing = canvasStoreListing(async () => {
    await new Promise<void>(resolve => pending.push(resolve)); return [];
  }, deliver);
  const scope = workspaces.slice(0, 6);
  listing.load(scope);
  for (let n = 0; n < 3; n++) {
    listing.load(scope);
    pending.splice(0).forEach(resolve => resolve()); await flush();
    expect(deliver).toHaveBeenCalledTimes(n + 1);
  }
  listing.dispose();
  pending.splice(0).forEach(resolve => resolve()); await flush();
});
