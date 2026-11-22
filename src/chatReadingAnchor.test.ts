import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createReadingAnchor } from './chatReadingAnchor';

// Run 15 S7d: while the transcript follows the bottom, nothing is held
// (the anchor is disabled there) and leaving the bottom always captures
// afresh (a scroll, or reset()), so a resize or a follow's scroll event must
// not measure every rendered turn. A node test: the observers, frames and
// the scroller (one turn holding one paragraph) are fakes; `scans` counts how
// often the anchor walks the transcript.
let resize: (() => void) | null = null;
let frames: { id: number; cb: FrameRequestCallback }[] = [];
let nextFrame = 1;
const flushFrames = () => { const queued = frames; frames = []; for (const f of queued) f.cb(0); };
const realRO = globalThis.ResizeObserver, realMO = globalThis.MutationObserver;
const realRAF = globalThis.requestAnimationFrame, realCAF = globalThis.cancelAnimationFrame;

beforeEach(() => {
  globalThis.ResizeObserver = class { constructor(cb: () => void) { resize = cb; } observe() {} unobserve() {} disconnect() {} } as unknown as typeof ResizeObserver;
  globalThis.MutationObserver = class { observe() {} disconnect() {} } as unknown as typeof MutationObserver;
  globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => { const id = nextFrame++; frames.push({ id, cb }); return id; }) as typeof requestAnimationFrame;
  globalThis.cancelAnimationFrame = ((id: number) => { frames = frames.filter(f => f.id !== id); }) as typeof cancelAnimationFrame;
});
afterEach(() => {
  globalThis.ResizeObserver = realRO; globalThis.MutationObserver = realMO;
  globalThis.requestAnimationFrame = realRAF; globalThis.cancelAnimationFrame = realCAF;
  resize = null; frames = [];
});

/** A 600px scroller showing one turn with one paragraph at `para.top`. */
function scroller() {
  const para = {
    top: 100,
    tagName: 'P',
    textContent: 'The garage leases held their contract rates all quarter.',
    parentElement: { closest: () => null },
    getBoundingClientRect: () => ({ top: para.top, bottom: para.top + 20, height: 20 }),
    getClientRects: () => [{}],
    matches: () => false,
    contains: () => false,
  };
  const turn = {
    getBoundingClientRect: () => ({ top: 0, bottom: 600, height: 600 }),
    querySelectorAll: () => [para],
  };
  const el = {
    scans: 0,
    scrollTop: 0,
    clientHeight: 600,
    offsetParent: {},
    children: [] as unknown[],
    getBoundingClientRect: () => ({ top: 0, bottom: 600, height: 600 }),
    querySelectorAll: () => { el.scans++; return [turn]; },
    contains: () => true,
  };
  return { el, para };
}

describe('the reading anchor while the transcript follows the bottom', () => {
  it('measures nothing on resizes, follow scrolls or a reset, and holds the place the reader leaves to', () => {
    const { el, para } = scroller();
    let following = true, moves = 0;
    const anchor = createReadingAnchor(el as unknown as HTMLElement, () => !following, () => { moves++; }, () => following);
    for (let flush = 0; flush < 5; flush++) {
      resize!();
      el.scrollTop += 40;
      anchor.scrolled();
    }
    anchor.reset();
    flushFrames();
    expect(el.scans).toBe(0);

    // The reader scrolls up: the controller clears following, then reports the scroll.
    following = false;
    el.scrollTop = 500;
    anchor.scrolled();
    expect(el.scans).toBe(1);
    // A delayed duplicate of that scroll event is not new intent.
    anchor.scrolled();
    expect(el.scans).toBe(1);
    // Content above grows by 30px: the place captured on leaving is held.
    para.top = 130;
    resize!();
    expect(el.scrollTop).toBe(530);
    expect(moves).toBe(1);
    expect(el.scans).toBe(1);
    anchor.dispose();
  });

  it('a reset captures on the next frame once not following, and not while following', () => {
    const { el } = scroller();
    let following = true;
    const anchor = createReadingAnchor(el as unknown as HTMLElement, () => false, () => {}, () => following);
    anchor.reset();
    flushFrames();
    expect(el.scans).toBe(0);
    following = false;
    anchor.reset();
    expect(el.scans).toBe(0);
    flushFrames();
    expect(el.scans).toBe(1);
    // A second reset replaces the first's frame: one capture, not two.
    anchor.reset();
    anchor.reset();
    expect(frames).toHaveLength(1);
    flushFrames();
    expect(el.scans).toBe(2);
    // Disposal cancels a pending capture.
    anchor.reset();
    anchor.dispose();
    expect(frames).toHaveLength(0);
    flushFrames();
    expect(el.scans).toBe(2);
  });

  it('still captures on a resize while held off for another reason (a settle, canvas focus)', () => {
    const { el } = scroller();
    const anchor = createReadingAnchor(el as unknown as HTMLElement, () => false, () => {}, () => false);
    resize!();
    expect(el.scans).toBe(1);
    anchor.dispose();
  });
});
