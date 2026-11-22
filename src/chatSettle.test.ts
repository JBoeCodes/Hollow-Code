import { describe, expect, it } from 'vitest';
import { SETTLE_MAX_MS, SETTLE_QUIET_MS, createSettle, type SettleView } from './chatSettle';

/** A scroller of `content` px showing `client` px, at `top`, on a fake clock. */
function fakeView(o: { content: number; client: number; top?: number }) {
  const v = { top: o.top ?? 0, content: o.content, client: o.client, at: 0, moves: 0 };
  const view: SettleView = {
    scrollTop: () => v.top,
    // The browser clamps a scroll to its range.
    setScrollTop: (top) => { v.top = Math.min(Math.max(0, top), Math.max(0, v.content - v.client)); v.moves++; },
    clientHeight: () => v.client,
    scrollHeight: () => v.content,
  };
  return { v, view, settle: createSettle(view, () => v.at) };
}

describe('createSettle: an opened conversation stays on its target while it settles', () => {
  it('canvases growing above the end keep the view at the end', () => {
    // Opened at the end while seven canvases were still placeholders.
    const { v, settle } = fakeView({ content: 4469, client: 670 });
    settle.start({ kind: 'bottom' });
    expect(v.top).toBe(4469 - 670);
    // The frames reach their real height: 1,000 px more above the view.
    v.content = 5469; v.at = 100;
    expect(settle.resized()).toBe(true);
    expect(v.top).toBe(5469 - 670);
  });

  it('a first frame slower than the quiet spell is still followed (a cold frame)', () => {
    const { v, settle } = fakeView({ content: 2000, client: 600 });
    settle.start({ kind: 'bottom' });
    // Observer reports with no size change don't start the quiet clock.
    v.at = 100; expect(settle.resized()).toBe(true);
    v.at = SETTLE_QUIET_MS + 2000; v.content = 2800;
    expect(settle.resized()).toBe(true);
    expect(v.top).toBe(2200);
  });

  it('a restored position is kept through the growth, clamped while the content is short', () => {
    const { v, settle } = fakeView({ content: 3000, client: 670 });
    settle.start({ kind: 'top', top: 3500 });
    expect(v.top).toBe(3000 - 670);
    v.content = 5000; v.at = 50;
    settle.resized();
    expect(v.top).toBe(3500);
  });

  it('ends after a quiet spell, and after the hard limit while the content keeps changing', () => {
    const quiet = fakeView({ content: 2000, client: 600 });
    quiet.settle.start({ kind: 'bottom' });
    quiet.v.at = 200; quiet.v.content = 2300;
    expect(quiet.settle.resized()).toBe(true);
    quiet.v.at = 200 + SETTLE_QUIET_MS + 1;
    quiet.v.content = 2500;
    expect(quiet.settle.resized()).toBe(false);
    expect(quiet.v.top).toBe(1700);
    expect(quiet.settle.active()).toBe(false);

    const busy = fakeView({ content: 2000, client: 600 });
    busy.settle.start({ kind: 'bottom' });
    for (let t = 500; t <= SETTLE_MAX_MS; t += 500) { busy.v.at = t; busy.v.content += 10; expect(busy.settle.resized()).toBe(true); }
    busy.v.at = SETTLE_MAX_MS + 1; busy.v.content += 10;
    expect(busy.settle.resized()).toBe(false);
  });

  it('a cancel (the person scrolled, clicked or sent) leaves the view alone', () => {
    const { v, settle } = fakeView({ content: 2000, client: 600 });
    settle.start({ kind: 'bottom' });
    settle.cancel();
    expect(settle.active()).toBe(false);
    v.content = 3000; v.at = 10;
    expect(settle.resized()).toBe(false);
    expect(v.top).toBe(1400);
  });

  it('no move when the view is already on its target', () => {
    const { v, settle } = fakeView({ content: 2000, client: 600, top: 1400 });
    settle.start({ kind: 'bottom' });
    v.at = 10;
    settle.resized();
    expect(v.moves).toBe(0);
  });
});
