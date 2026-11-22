import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// @ts-expect-error: browser build has no declarations; Node effects are inert.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// A chat's shared subagents read (COO rebuild L3): one read per chat however
// many views use it, polled while something on screen needs it, with older
// finished pages added by Show older.
const h = vi.hoisted(() => ({ sessionSubagents: vi.fn() }));
vi.mock('../api', () => ({ api: { sessionSubagents: h.sessionSubagents } }));
vi.mock('./runQueue', async () => {
  const { createSignal } = await import('solid-js');
  const [runQueue, setRunQueue] = createSignal<unknown[]>([]);
  return { runQueue, setRunQueue };
});
import { createRoot, createSignal } from 'solid-js';
import { setRunQueue } from './runQueue';
import { createSecondClock, createSubagentsRead } from './subagents';

const viewOf = (running: number, queued = 0) => ({ parent: 'p', running, queued, doneOlder: [], olderTotal: 0 });
let n = 0;
const nextKey = () => `chat-${++n}`;
const settle = () => vi.advanceTimersByTimeAsync(0);
const disposers: Array<() => void> = [];
function mount<T>(fn: () => T): T {
  return createRoot(dispose => { disposers.push(dispose); return fn(); });
}

beforeEach(() => {
  vi.useFakeTimers();
  h.sessionSubagents.mockReset();
  h.sessionSubagents.mockResolvedValue(viewOf(0));
});
afterEach(() => {
  while (disposers.length) disposers.pop()!();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('createSubagentsRead', () => {
  it('reads once on mount with no older page, and shows the view', async () => {
    const key = nextKey();
    const r = mount(() => createSubagentsRead(() => key));
    await settle();
    expect(h.sessionSubagents).toHaveBeenCalledTimes(1);
    expect(h.sessionSubagents).toHaveBeenCalledWith(key, 0);
    expect(r.view()?.running).toBe(0);
    expect(r.error()).toBeNull();
  });

  it('does nothing without a chat key', async () => {
    const r = mount(() => createSubagentsRead(() => null));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.sessionSubagents).not.toHaveBeenCalled();
    expect(r.view()).toBeNull();
    expect(r.error()).toBeNull();
  });

  it('polls every 20 s while idle and every 3 s once something runs', async () => {
    const key = nextKey();
    mount(() => createSubagentsRead(() => key));
    await settle();
    await vi.advanceTimersByTimeAsync(19_999);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(2);
    h.sessionSubagents.mockResolvedValue(viewOf(1));
    await vi.advanceTimersByTimeAsync(20_000);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(4);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(5);
  });

  it('shares one read between two views of a chat, and stops polling when the last unmounts', async () => {
    const key = nextKey();
    const first = createRoot(dispose => { createSubagentsRead(() => key); return dispose; });
    const second = createRoot(dispose => { createSubagentsRead(() => key); return dispose; });
    await settle();
    expect(h.sessionSubagents).toHaveBeenCalledTimes(1);
    first();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(2);
    second();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(2);
  });

  it('keeps the last view cached for the next view that mounts', async () => {
    const key = nextKey();
    h.sessionSubagents.mockResolvedValue(viewOf(2));
    const gone = createRoot(dispose => { createSubagentsRead(() => key); return dispose; });
    await settle();
    gone();
    const again = mount(() => createSubagentsRead(() => key));
    expect(again.view()?.running).toBe(2);
  });

  it('Show older asks for the next 25 and keeps asking for that many', async () => {
    const key = nextKey();
    const r = mount(() => createSubagentsRead(() => key));
    await settle();
    await r.showOlder();
    expect(h.sessionSubagents).toHaveBeenLastCalledWith(key, 25);
    await r.showOlder();
    expect(h.sessionSubagents).toHaveBeenLastCalledWith(key, 50);
    await r.refresh();
    expect(h.sessionSubagents).toHaveBeenLastCalledWith(key, 50);
  });

  it('shows a failed read as an error, and a later good read clears it', async () => {
    const key = nextKey();
    h.sessionSubagents.mockRejectedValueOnce(new Error('offline'));
    const r = mount(() => createSubagentsRead(() => key));
    await settle();
    expect(r.error()).toContain('offline');
    await r.refresh();
    expect(r.error()).toBeNull();
    expect(r.view()).not.toBeNull();
  });

  it('drops a read that finishes after a newer one started', async () => {
    const key = nextKey();
    let finishSlow!: (v: unknown) => void;
    h.sessionSubagents.mockReturnValueOnce(new Promise(res => { finishSlow = res; }));
    const r = mount(() => createSubagentsRead(() => key));
    await settle();
    h.sessionSubagents.mockResolvedValueOnce(viewOf(5));
    await r.refresh();
    expect(r.view()?.running).toBe(5);
    finishSlow(viewOf(1));
    await settle();
    expect(r.view()?.running).toBe(5);
  });

  it('skips the read while the window is hidden', async () => {
    vi.stubGlobal('document', { visibilityState: 'hidden', addEventListener: vi.fn(), removeEventListener: vi.fn() });
    const key = nextKey();
    const r = mount(() => createSubagentsRead(() => key));
    await settle();
    expect(h.sessionSubagents).not.toHaveBeenCalled();
    expect(r.view()).toBeNull();
  });

  it('re-reads every chat in use half a second after the last run-queue change', async () => {
    const key = nextKey();
    mount(() => createSubagentsRead(() => key));
    await settle();
    expect(h.sessionSubagents).toHaveBeenCalledTimes(1);
    setRunQueue([{}] as never);
    await vi.advanceTimersByTimeAsync(300);
    setRunQueue([{}, {}] as never);
    await vi.advanceTimersByTimeAsync(499);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(h.sessionSubagents).toHaveBeenCalledTimes(2);
  });

  it('follows the chat when the key changes', async () => {
    const [key, setKey] = createSignal(nextKey());
    const second = nextKey();
    mount(() => createSubagentsRead(key));
    await settle();
    setKey(second);
    await settle();
    expect(h.sessionSubagents).toHaveBeenLastCalledWith(second, 0);
  });
});

describe('createSecondClock', () => {
  it('ticks each second only while live, and stops when it is not', async () => {
    vi.setSystemTime(1_000_000);
    const [live, setLive] = createSignal(true);
    const now = mount(() => createSecondClock(live));
    expect(now()).toBe(1_000_000);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(now()).toBe(1_003_000);
    setLive(false);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(now()).toBe(1_003_000);
    setLive(true);
    expect(now()).toBe(1_013_000);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(now()).toBe(1_014_000);
  });
});
