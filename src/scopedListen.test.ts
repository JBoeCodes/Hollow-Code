import workflowsSource from './pages/Workflows.tsx?raw';
import learningSource from './components/Learnings.tsx?raw';
import { describe, expect, it, vi } from 'vitest';
import { scopedListen } from './scopedListen';

function deferred() {
  let resolve!: (fn: () => void) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<() => void>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

// P6: a view closed before its listener finished registering left the
// handler registered for the app's lifetime.
describe('scopedListen', () => {
  it('disposed before registration resolves: the late unlisten runs as it arrives', async () => {
    const d = deferred();
    const unlisten = vi.fn();
    const dispose = scopedListen(() => d.promise);
    dispose();
    d.resolve(unlisten);
    await Promise.resolve(); await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('disposed after it resolves: unlistens once, however often disposed', async () => {
    const d = deferred();
    const unlisten = vi.fn();
    const dispose = scopedListen(() => d.promise);
    d.resolve(unlisten);
    await Promise.resolve(); await Promise.resolve();
    expect(unlisten).not.toHaveBeenCalled();
    dispose();
    dispose();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('a registration that fails (not in Tauri) is quiet', async () => {
    const d = deferred();
    const dispose = scopedListen(() => d.promise);
    d.reject(new Error('no tauri'));
    await Promise.resolve(); await Promise.resolve();
    expect(() => dispose()).not.toThrow();
  });
});

// The views P6 fixed keep the scoped form (a revert would pass the tests above).
describe('views whose listener could outlive them', () => {
  it.each([['pages/Workflows.tsx', workflowsSource], ['components/Learnings.tsx', learningSource]])('%s registers through scopedListen', (_, source) => {
    expect(source).not.toMatch(/\.then\(\(?fn\)? => \{ unlisten/);
    expect(source).toMatch(/scopedListen\(\(\) =>/);
  });
});
