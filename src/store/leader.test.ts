import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('../goTo', () => ({ runGoTarget: vi.fn() }));
vi.mock('./runtime', () => ({ shortcutsOverlayOpen: () => false, flashFooter: vi.fn() }));
vi.mock('../modalGuard', () => ({ modalOpen: () => false }));
import { WHICH_KEY_DELAY_MS } from '../keyboardLeader';
import { cancelLeader, leaderKeydown, leaderPending, whichKeyOpen } from './leader';

const key = (k: string) => ({
  key: k, code: '', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, isComposing: false,
  target: null, preventDefault() {}, stopPropagation() {},
}) as unknown as KeyboardEvent;

// P1-49: the which-key card used to give up after 2 s, often while the
// person was still reading it.
describe('the go-to leader waits once its card shows', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { cancelLeader(); vi.useRealTimers(); });

  it('keeps waiting after its card shows, until a key, Esc or a click', () => {
    expect(leaderKeydown(key('g'))).toBe(true);
    expect(leaderPending()).toBe(true);
    vi.advanceTimersByTime(WHICH_KEY_DELAY_MS);
    expect(whichKeyOpen()).toBe(true);
    vi.advanceTimersByTime(60_000);
    expect(leaderPending(), 'still waiting for a key').toBe(true);
    expect(whichKeyOpen()).toBe(true);
    leaderKeydown(key('Escape'));
    expect(leaderPending()).toBe(false);
    expect(whichKeyOpen()).toBe(false);
  });
});
