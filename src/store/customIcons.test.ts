import { beforeEach, describe, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ read: vi.fn(), listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('../api', () => ({ api: { readCustomIcon: mocks.read } }));
vi.mock('../nativeEvents', () => ({ listen: mocks.listen }));
import type { CustomIcon } from '../customIcons';
const record = (png: string): CustomIcon => ({ version: 1, revision: png, original: {format: 'png', bytes: png}, editor: null, pngs: {16: png, 32: png, 64: png, 128: png, 512: png} });
beforeEach(() => { vi.resetModules(); mocks.read.mockReset(); mocks.listen.mockClear(); });
describe('independent app and COO artwork', () => {
  it('does not let an old app read undo Reset or affect COO', async () => {
    const store = await import('./customIcons');
    let resolve!: (r: CustomIcon) => void;
    mocks.read.mockReturnValue(new Promise<CustomIcon>(r => { resolve = r; }));
    const pending = store.refreshIcon('app');
    store.useSavedIcon('mainAgent', record('main'));
    store.useSavedIcon('app', null);
    resolve(record('old'));
    await pending;
    expect(store.appIcon()).toBeNull();
    expect(store.mainAgentIcon()).toBe('data:image/png;base64,main');
  });
  it('shares one startup read per target and refreshes only the changed target', async () => {
    mocks.read.mockResolvedValue(null);
    const store = await import('./customIcons');
    store.ensureCustomIcons(); store.ensureCustomIcons();
    expect(mocks.read.mock.calls).toEqual([['mainAgent'], ['app']]);
    expect(mocks.listen).toHaveBeenCalledTimes(1);
    const handler = (mocks.listen.mock.calls[0] as unknown as [string, (e: {payload:string}) => void])[1];
    handler({payload:'app'});
    expect(mocks.read.mock.calls.at(-1)).toEqual(['app']);
    handler({payload:'unknown'});
    expect(mocks.read).toHaveBeenCalledTimes(3);
  });
});
