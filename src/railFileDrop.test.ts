import { describe, expect, it, vi } from 'vitest';
import { attachFilesToRailChat, type RailDropSnapshot, type RailFileDropDriver } from './railFileDrop';

function harness() {
  const origin = { key: 's-from', version: 1, page: 'today' };
  let now: RailDropSnapshot = origin, visible = false;
  const files = [{ file: { name: 'brief.pdf' } as File, known: true }];
  const driver: RailFileDropDriver = {
    snapshot: () => now, visible: () => visible, pending: () => false,
    open: vi.fn(async () => { now = { key: 's-target', version: 2, page: 'today' }; return true; }),
    attach: vi.fn(() => true), wait: vi.fn(async () => { visible = true; }),
  };
  return { origin, files, driver, set: (next: RailDropSnapshot) => { now = next; }, show: () => { visible = true; } };
}

describe('file drop exact conversation ownership', () => {
  it('opens the chosen row and waits for its composer before attaching', async () => {
    const h = harness();
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('attached');
    expect(h.driver.open).toHaveBeenCalledWith('s-target', expect.any(Function));
    expect(h.driver.attach).toHaveBeenCalledWith(h.files[0].file, true);
  });
  it('refuses a navigation before its lazy module loads', async () => {
    const h = harness(); h.set({ key: 's-other', version: 2, page: 'today' });
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('changed');
    expect(h.driver.open).not.toHaveBeenCalled(); expect(h.driver.attach).not.toHaveBeenCalled();
  });
  it('allows the workspace opener to switch page before committing its conversation', async () => {
    const h = harness();
    const origin = { ...h.origin, page: 'settings' }; h.set(origin);
    h.driver.open = async (_key, current) => {
      expect(current()).toBe(true);
      h.set({ ...origin, page: 'today' });
      expect(current()).toBe(true);
      h.set({ key: 's-target', version: 2, page: 'today' });
      return true;
    };
    expect(await attachFilesToRailChat('s-target', h.files, origin, h.driver)).toBe('attached');
  });
  it('does not attach when navigation is refused', async () => {
    const h = harness(); h.driver.open = vi.fn(async () => false);
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('unavailable');
    expect(h.driver.attach).not.toHaveBeenCalled();
  });
  it('drops ownership when a different chat wins while waiting for mount', async () => {
    const h = harness(); h.driver.wait = async () => { h.set({ key: 's-other', version: 3, page: 'today' }); h.show(); };
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('changed');
    expect(h.driver.attach).not.toHaveBeenCalled();
  });
  it('rejects a new incarnation of the same conversation', async () => {
    const h = harness(); h.driver.wait = async () => { h.set({ key: 's-target', version: 3, page: 'today' }); h.show(); };
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('changed');
    expect(h.driver.attach).not.toHaveBeenCalled();
  });
  it('reports a rejected file through the existing attachment sink', async () => {
    const h = harness(); h.show(); h.driver.attach = vi.fn(() => false);
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('rejected');
  });
  it('continues to later files after the composer rejects one', async () => {
    const h = harness(); h.show();
    h.driver.attach = vi.fn().mockReturnValueOnce(false).mockReturnValueOnce(true);
    const valid = { file: { name: 'valid.txt' } as File, known: true };
    expect(await attachFilesToRailChat('s-target', [...h.files, valid], h.origin, h.driver)).toBe('rejected');
    expect(h.driver.attach).toHaveBeenCalledTimes(2);
    expect(h.driver.attach).toHaveBeenLastCalledWith(valid.file, true);
  });
  it('waits for an exact history-page request to hydrate', async () => {
    const h = harness();
    h.driver.open = async () => { h.set({ ...h.origin, page: 'chat' }); return true; };
    h.driver.pending = key => key === 's-target';
    h.driver.wait = async () => { h.set({ key: 's-target', version: 2, page: 'chat' }); h.show(); };
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('attached');
  });
  it('refuses history hydration when another request takes its place', async () => {
    const h = harness();
    h.driver.open = async () => { h.set({ ...h.origin, page: 'chat' }); return true; };
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('changed');
    expect(h.driver.attach).not.toHaveBeenCalled();
  });
  it('times out without putting a file in an unscoped queue', async () => {
    const h = harness(); h.driver.wait = vi.fn(async () => {});
    expect(await attachFilesToRailChat('s-target', h.files, h.origin, h.driver)).toBe('unavailable');
    expect(h.driver.wait).toHaveBeenCalledTimes(100); expect(h.driver.attach).not.toHaveBeenCalled();
  });
});
