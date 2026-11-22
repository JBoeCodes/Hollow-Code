import { describe, expect, it, vi } from 'vitest';
vi.mock('./store/chatRuntime', () => ({ chatBusy: () => false }));
vi.mock('./store/runtime', () => ({ canNavBack: () => false, canNavForward: () => false }));
vi.mock('./store/browser', () => ({ browserTabs: () => [] }));
vi.mock('./api', () => ({ api: { setMenuEnabled: vi.fn(() => Promise.resolve()) } }));
import { menuStates } from './menuState';

// P1-49: Stop Running Turn, Back and Forward were always clickable in the
// menu bar, and did nothing when there was nothing to stop or step to.
describe('menu items follow the app', () => {
  it('greys Stop Running Turn with nothing running', () => {
    expect(menuStates({ busy: false, back: true, forward: true, browser: false })['stop-run']).toBe(false);
    expect(menuStates({ busy: true, back: true, forward: true, browser: false })['stop-run']).toBe(true);
  });
  it('greys Back and Forward at the ends of the history', () => {
    expect(menuStates({ busy: false, back: false, forward: true, browser: false })).toEqual({ 'stop-run': false, 'go-back': false, 'go-forward': true, 'undo-chat-action': false });
    expect(menuStates({ busy: false, back: true, forward: false, browser: false })).toEqual({ 'stop-run': false, 'go-back': true, 'go-forward': false, 'undo-chat-action': false });
  });
  it('greys Undo Chat Action unless a chat action can be undone', () => {
    expect(menuStates({ busy: false, back: false, forward: false, browser: false })['undo-chat-action']).toBe(false);
    expect(menuStates({ busy: false, back: false, forward: false, browser: false, undo: true })['undo-chat-action']).toBe(true);
  });
  it('keeps Back and Forward on while the side panel page has the keys (they step the page)', () => {
    expect(menuStates({ busy: false, back: false, forward: false, browser: true })).toEqual({ 'stop-run': false, 'go-back': true, 'go-forward': true, 'undo-chat-action': false });
  });
});
