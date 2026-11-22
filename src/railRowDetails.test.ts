import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { closeRailDetail, railDetail, railRowSummary, setRailDetail } from './railRowDetails';
import type { AgentSessionSummary } from './types';

beforeEach(() => vi.useFakeTimers());
afterEach(() => { closeRailDetail(); vi.useRealTimers(); vi.unstubAllGlobals(); });

function harness() {
  let focused = false, hovered = false, cardHovered = false;
  const anchor = {
    contains: () => focused,
    matches: () => hovered,
  } as unknown as HTMLElement;
  vi.stubGlobal('document', { activeElement: {}, querySelector: () => ({ matches: () => cardHovered }) });
  const session = { key: 's-summary', title: 'Summary example' } as AgentSessionSummary;
  const root = createRoot(dispose => ({ handlers: railRowSummary(() => session), dispose }));
  return {
    ...root, anchor, session,
    show: () => root.handlers.onPointerEnter({ currentTarget: anchor, type: 'pointerenter' } as unknown as Event),
    focus: (value: boolean) => { focused = value; }, hover: (value: boolean) => { hovered = value; },
    cardHover: (value: boolean) => { cardHovered = value; },
  };
}

describe('row detail hover and focus ownership', () => {
  it('keeps keyboard-focused metadata visible when the pointer leaves', () => {
    const h = harness(); h.show(); h.focus(true); h.handlers.onPointerLeave();
    vi.advanceTimersByTime(200);
    expect(railDetail()?.session.key).toBe('s-summary'); h.dispose();
  });
  it('keeps hovered metadata visible when keyboard focus leaves', () => {
    const h = harness(); h.show(); h.hover(true); h.handlers.onFocusOut({ currentTarget: h.anchor, relatedTarget: null } as unknown as FocusEvent);
    vi.advanceTimersByTime(200);
    expect(railDetail()?.anchor).toBe(h.anchor); h.dispose();
  });
  it('allows moving the pointer onto the portalled card', () => {
    const h = harness(); h.show(); h.handlers.onPointerLeave(); h.cardHover(true);
    vi.advanceTimersByTime(200);
    expect(railDetail()).not.toBeNull(); h.dispose();
  });
  it('closes when neither row nor card owns hover or focus', () => {
    const h = harness(); h.show(); h.handlers.onPointerLeave();
    vi.advanceTimersByTime(200);
    expect(railDetail()).toBeNull(); h.dispose();
  });
  it('cleans up its own card when the row disappears', () => {
    const h = harness(); h.show(); h.dispose(); expect(railDetail()).toBeNull();
  });
  it('does not invent a summary for a project without a loaded lead', () => {
    const h = harness(); h.show();
    const empty = createRoot(dispose => ({ handlers: railRowSummary(() => undefined), dispose }));
    empty.handlers.onPointerEnter({ currentTarget: h.anchor, type: 'pointerenter' } as unknown as Event);
    expect(railDetail()).toBeNull(); empty.dispose(); h.dispose();
  });
  it('does not immediately reopen after closing the persistent card back to its row', () => {
    const h = harness();
    setRailDetail({ session: h.session, anchor: h.anchor, persistent: true }); closeRailDetail();
    h.handlers.onFocusIn({ currentTarget: h.anchor, type: 'focusin' } as unknown as Event);
    expect(railDetail()).toBeNull(); h.dispose();
  });
});
