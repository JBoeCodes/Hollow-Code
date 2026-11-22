import { beforeEach, expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ snapshot: { items: [] as any[] }, setTaskStatus: vi.fn() }));
vi.mock('./api', () => ({ api: { setTaskStatus: mock.setTaskStatus } }));
vi.mock('./store', () => ({
  tasksData: () => mock.snapshot, mutateTasks: (s: typeof mock.snapshot) => { mock.snapshot = s; },
  projectsData: () => undefined, mutateProjects: vi.fn(), refetchProjects: vi.fn(),
  refetchTasks: vi.fn(async () => mock.snapshot), flashFooter: vi.fn(), flashWithUndo: vi.fn(),
  trackTask: (_name: string, work: () => Promise<unknown>) => work(),
}));
import { applyDrop } from './dragdrop';

beforeEach(() => {
  mock.snapshot = { items: [{ url: 'a', status: 'Not started', contentHash: 'a0' }, { url: 'b', status: 'Not started', contentHash: 'b0' }] };
  mock.setTaskStatus.mockReset();
});

it('a failed concurrent tick rolls back only its own field and preserves B success', async () => {
  let fail!: (e: Error) => void;
  mock.setTaskStatus.mockImplementation((url: string) => url === 'a' ? new Promise((_resolve, reject) => { fail = reject; }) : Promise.resolve('b1'));
  const a = applyDrop({ kind: 'task', url: 'a', dimension: 'status', value: 'Delivered', originValue: 'Not started' });
  await applyDrop({ kind: 'task', url: 'b', dimension: 'status', value: 'Delivered', originValue: 'Not started' });
  fail(new Error('A failed')); await a;
  expect(mock.snapshot.items.find(t => t.url === 'a')?.status).toBe('Not started');
  expect(mock.snapshot.items.find(t => t.url === 'b')).toMatchObject({ status: 'Delivered', contentHash: 'b1' });
});

it('a failed write never restores an older version over a newer edit to that task', async () => {
  let fail!: (e: Error) => void;
  mock.setTaskStatus.mockImplementation(() => new Promise((_resolve, reject) => { fail = reject; }));
  const a = applyDrop({ kind: 'task', url: 'a', dimension: 'status', value: 'Delivered', originValue: 'Not started' });
  mock.snapshot.items[0] = { ...mock.snapshot.items[0], status: 'In progress', contentHash: 'a1' };
  fail(new Error('changed')); await a;
  expect(mock.snapshot.items[0]).toMatchObject({ status: 'In progress', contentHash: 'a1' });
});
