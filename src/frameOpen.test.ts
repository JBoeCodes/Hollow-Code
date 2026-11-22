import { it, expect, vi, afterEach } from 'vitest';
const spies = vi.hoisted(() => ({ open: vi.fn(), preview: vi.fn(), confirm: vi.fn(() => Promise.resolve(false)) }));
vi.mock('./api', () => ({ api: {} }));
vi.mock('./utils', () => ({ errMsg: String, openExternal: spies.open }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('./mockMode', () => ({ isBrowserMockMode: () => false }));
vi.mock('./store/runtime', () => ({ activePage: () => 'today', flashFooter: vi.fn(), openInBrowser: spies.open, showConfirm: spies.confirm, confirmDialog: () => null }));
// Run 15 P1: an approved web link opens in the Preview pane's web mode.
vi.mock('./store/preview', () => ({ openPreview: spies.preview }));
vi.mock('./store/workspaces', () => ({ focusWorkspaceConversation: vi.fn(), setWorkspaceView: vi.fn(), viewOf: vi.fn(), workspaceDetail: () => null, workspaceId: () => null }));
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });
import { openFrameLink } from './store/browser';
it('a focused scripted frame cannot open a generated destination without host confirmation', async () => {
  const frame = { isConnected: true } as HTMLIFrameElement;
  vi.stubGlobal('document', { activeElement: frame });
  vi.stubGlobal('navigator', { userActivation: { isActive: true } });
  spies.open.mockResolvedValue(undefined);
  const href = 'http://127.0.0.1:49231/canary?value=synthetic';
  expect(openFrameLink({ href, link: 'web' }, frame)).toBe(true);
  await Promise.resolve();
  expect(spies.open).not.toHaveBeenCalled();
  expect(spies.confirm).toHaveBeenCalledWith(expect.objectContaining({ body: expect.stringContaining(href) }));
  vi.unstubAllGlobals();
});

it('only the approved destination opens, and replacing its frame cancels the request', async () => {
  vi.spyOn(Date, 'now').mockReturnValue(120000);
  const frame = { isConnected: true, src: 'artifact://localhost/one', srcdoc: '' } as HTMLIFrameElement;
  vi.stubGlobal('document', { activeElement: frame });
  vi.stubGlobal('navigator', {});
  let answer!: (ok: boolean) => void;
  spies.confirm.mockImplementation(() => new Promise(resolve => { answer = resolve; }));
  const href = 'http://127.0.0.1:49231/approved';
  const data = { href, link: 'web' as const };
  openFrameLink(data, frame);
  expect(spies.preview).not.toHaveBeenCalled();
  data.href = 'http://127.0.0.1:49231/changed';
  answer(true); await Promise.resolve();
  expect(spies.preview).toHaveBeenCalledTimes(1);
  expect(spies.preview).toHaveBeenCalledWith({ kind: 'url', url: href });
  expect(spies.open).not.toHaveBeenCalled();
  spies.preview.mockClear();
  openFrameLink(data, frame);
  frame.src = 'artifact://localhost/two';
  answer(true); await Promise.resolve();
  expect(spies.preview).not.toHaveBeenCalled();
  vi.restoreAllMocks();
});
