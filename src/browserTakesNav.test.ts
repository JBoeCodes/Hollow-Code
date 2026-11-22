import { afterEach, describe, expect, it, vi } from 'vitest';
vi.mock('./api', () => ({ api: {} }));
vi.mock('./utils', () => ({ errMsg: String, openExternal: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('./mockMode', () => ({ isBrowserMockMode: () => false }));
vi.mock('./store/runtime', () => ({ activePage: () => 'today', flashFooter: vi.fn(), openInBrowser: vi.fn(), openSidePane: vi.fn(), sidePaneHosted: () => false, showConfirm: vi.fn(), confirmDialog: () => null }));
vi.mock('./store/workspaces', () => ({ focusWorkspaceConversation: vi.fn(), setWorkspaceView: vi.fn(), viewOf: vi.fn(), workspaceDetail: () => null, workspaceId: () => null }));
import { browserTakesNav, registerBrowserPage } from './store/browser';

let off: (() => void)[] = [];
const register = (on: () => boolean) => { const u = registerBrowserPage(on); off.push(u); return u; };
afterEach(() => { vi.unstubAllGlobals(); off.forEach(u => u()); off = []; });
/** The webview's document: whether it has focus, and whether its focused
 *  element sits inside the browser panel. */
const doc = (focused: boolean, inPane: boolean) => vi.stubGlobal('document', {
  hasFocus: () => focused,
  activeElement: { closest: (sel: string) => (sel === '.browser-pane' && inPane ? {} : null) },
});

// P1-47: Back and Forward mean the side panel's page only while it has the
// keys: focus in the panel, or in its native page (the main document lost
// focus to it), and only while a live page is on show.
describe('browserTakesNav', () => {
  it('needs a live page on show', () => {
    doc(true, true);
    expect(browserTakesNav()).toBe(false);
    register(() => false);
    expect(browserTakesNav()).toBe(false);
    register(() => true);
    expect(browserTakesNav()).toBe(true);
  });
  it('a pane that goes clears only its own page, not a newer pane’s', () => {
    doc(true, true);
    const first = register(() => true);
    register(() => true);
    first();
    expect(browserTakesNav()).toBe(true);
  });
  it('takes the keys with focus in the panel or in its native page, not elsewhere', () => {
    register(() => true);
    doc(true, false);
    expect(browserTakesNav()).toBe(false);
    doc(false, false);
    expect(browserTakesNav()).toBe(true);
  });
});
