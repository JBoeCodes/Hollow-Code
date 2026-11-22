import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Run 15 P2 second-pass review: Back and Forward are the person's own
// navigation, so they clear a refused address's failure and end a pending
// reload's time limit (a page from WebKit's cache may send no event); and a
// stopped load's start event, already on its way when Stop came, never brings
// the spinner back (no `loaded` follows a stopped load).
type Ev = { tabId: string; kind: string; url?: string; title?: string; gen?: number };
const state = vi.hoisted(() => ({ handler: null as null | ((e: { payload: Ev }) => void) }));
vi.stubGlobal('window', globalThis);
vi.mock('../nativeEvents', () => ({
  listen: vi.fn(async (_name: string, handler: (e: { payload: Ev }) => void) => { state.handler = handler; return () => {}; }),
}));
vi.mock('../api', () => ({
  api: { browserOpen: vi.fn(), browserGo: vi.fn(), browserClose: vi.fn(async () => {}), browserUrl: vi.fn(async () => '') },
}));
vi.mock('../mockMode', () => ({ isBrowserMockMode: () => false }));
vi.mock('./runtime', () => ({
  activePage: () => 'chat', chatCwd: () => null, flashFooter: vi.fn(), openFileInSidePane: vi.fn(),
  openInBrowser: vi.fn(async () => {}), showConfirm: vi.fn(async () => false), confirmDialog: () => null, vaultRoot: () => '/vault',
}));
vi.mock('./workspaces', () => ({ workspaceDetail: () => null }));
vi.mock('./preview', () => ({ openPreview: vi.fn() }));

const { api } = await import('../api');
const { NO_ANSWER_MS } = await import('../browserReview');
const browser = await import('./browser');

const open = vi.mocked(api.browserOpen);
const go = vi.mocked(api.browserGo);
const emit = (payload: Ev) => state.handler!({ payload });
const tab = (id: string) => browser.allBrowserTabs().find(t => t.id === id)!;
const settle = () => vi.advanceTimersByTimeAsync(0);

/** A tab showing https://a.example/ (load 1, answered). */
async function pageA(): Promise<string> {
  open.mockResolvedValueOnce(1);
  await browser.openBrowserUrl('https://a.example/');
  const id = browser.activeBrowserTab()!;
  emit({ tabId: id, kind: 'loaded', url: 'https://a.example/', gen: 1 });
  expect(tab(id)).toMatchObject({ ready: true, committed: true, loading: false, error: null, gen: 1 });
  // A later load opens after this answer (fake time stands still otherwise).
  await vi.advanceTimersByTimeAsync(1);
  return id;
}

beforeEach(() => {
  // Every load arms its time limit: fake timers, so none outlives its test.
  vi.useFakeTimers();
  browser.setBrowserTabs([]);
  open.mockReset();
  go.mockReset();
});
afterEach(() => { vi.useRealTimers(); });

describe('Back, Forward and Stop', () => {
  it('Back after a refused address shows the page it went back to', async () => {
    const id = await pageA();
    open.mockRejectedValueOnce(new Error('b.example refused the connection.'));
    await browser.openBrowserUrl('https://b.example/');
    expect(tab(id).error).toContain('refused');
    go.mockResolvedValueOnce(0);
    browser.browserGo('back');
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 1 });
    emit({ tabId: id, kind: 'loaded', url: 'https://a.example/', gen: 1 });
    expect(tab(id)).toMatchObject({ error: null, timedOut: false, loading: false, url: 'https://a.example/' });
    expect(go).toHaveBeenCalledWith(id, 'back');
  });

  it('Forward clears a failure the same way', async () => {
    const id = await pageA();
    open.mockRejectedValueOnce(new Error('b.example refused the connection.'));
    await browser.openBrowserUrl('https://b.example/');
    go.mockResolvedValueOnce(0);
    browser.browserGo('forward');
    expect(tab(id).error).toBeNull();
  });

  it('Back to a cached page during an unanswered reload never fails with the reload’s time limit', async () => {
    const id = await pageA();
    go.mockResolvedValueOnce(2);
    browser.browserGo('reload');
    await vi.advanceTimersByTimeAsync(0);
    expect(tab(id)).toMatchObject({ gen: 2, loading: true });
    go.mockResolvedValueOnce(0);
    browser.browserGo('back');
    await vi.advanceTimersByTimeAsync(NO_ANSWER_MS + 1);
    expect(tab(id)).toMatchObject({ error: null, timedOut: false, loading: false });
  });

  it('a reload that never answers still fails after the time limit', async () => {
    const id = await pageA();
    go.mockResolvedValueOnce(2);
    browser.browserGo('reload');
    await vi.advanceTimersByTimeAsync(NO_ANSWER_MS + 1);
    expect(tab(id)).toMatchObject({ loading: false, timedOut: true });
    expect(tab(id).error).toContain('didn’t answer');
  });

  it('Back on a tab that never showed a page keeps its failure and Try again', async () => {
    open.mockResolvedValueOnce(1);
    await browser.openBrowserUrl('https://slow.example/');
    const id = browser.activeBrowserTab()!;
    await vi.advanceTimersByTimeAsync(NO_ANSWER_MS + 1);
    expect(tab(id)).toMatchObject({ committed: false, timedOut: true });
    go.mockResolvedValueOnce(0);
    browser.browserGo('back');
    expect(tab(id).error).toContain('didn’t answer');
  });

  it('Back while a tab’s first open is still pending leaves that open in charge', async () => {
    let answer!: (gen: number) => void;
    open.mockReturnValueOnce(new Promise<number>(resolve => { answer = resolve; }));
    const opening = browser.openBrowserUrl('https://c.example/');
    const id = browser.activeBrowserTab()!;
    go.mockRejectedValueOnce(new Error('no such tab'));
    browser.browserGo('back');
    answer(1);
    await opening;
    expect(tab(id)).toMatchObject({ ready: true, opening: false, gen: 1 });
  });

  it('a stopped load’s late start never brings the spinner back; the next answer is followed again', async () => {
    const id = await pageA();
    go.mockResolvedValueOnce(2);
    browser.browserGo('reload');
    await settle();
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 2 });
    expect(tab(id).loading).toBe(true);
    go.mockResolvedValueOnce(0);
    browser.browserGo('stop');
    expect(tab(id).loading).toBe(false);
    // Sent before the page stopped, delivered after.
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 2 });
    expect(tab(id).loading).toBe(false);
    // A page that then navigates itself is followed: its answer, then its next load.
    emit({ tabId: id, kind: 'loaded', url: 'https://a.example/next', gen: 2 });
    expect(tab(id)).toMatchObject({ loading: false, url: 'https://a.example/next' });
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/later', gen: 2 });
    expect(tab(id).loading).toBe(true);
  });

  it('Back right after a Stop (no history) still keeps the stopped load’s late start out', async () => {
    const id = await pageA();
    go.mockResolvedValueOnce(2);
    browser.browserGo('reload');
    await settle();
    go.mockResolvedValueOnce(0);
    browser.browserGo('stop');
    go.mockResolvedValueOnce(0);
    browser.browserGo('back');
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 2 });
    expect(tab(id).loading).toBe(false);
    emit({ tabId: id, kind: 'loaded', url: 'https://a.example/', gen: 2 });
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/next', gen: 2 });
    expect(tab(id).loading).toBe(true);
  });

  it('a new load after a Stop shows its spinner', async () => {
    const id = await pageA();
    go.mockResolvedValueOnce(0);
    browser.browserGo('stop');
    go.mockResolvedValueOnce(3);
    browser.browserGo('reload');
    await settle();
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 3 });
    expect(tab(id).loading).toBe(true);
    emit({ tabId: id, kind: 'loading', url: 'https://a.example/', gen: 1 });
    emit({ tabId: id, kind: 'loaded', url: 'https://a.example/', gen: 3 });
    expect(tab(id).loading).toBe(false);
  });
});
