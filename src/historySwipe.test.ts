import { describe, expect, it } from 'vitest';
import { SWIPE_DISTANCE, SWIPE_IDLE_MS, createSwipeDetector, scrollsSideways } from './historySwipe';
import source from './historySwipe.ts?raw';
import app from './App.tsx?raw';

describe('swipe detector', () => {
  it('a rightward swipe goes back once, however long the inertia runs', () => {
    const d = createSwipeDetector();
    const steps: (string | null)[] = [];
    for (let i = 0; i < 40; i++) steps.push(d.feed(-12, 1, i * 16));
    expect(steps.filter(Boolean)).toEqual(['back']);
  });
  it('a leftward swipe goes forward', () => {
    const d = createSwipeDetector();
    const got = Array.from({ length: 20 }, (_, i) => d.feed(15, 0, i * 16)).filter(Boolean);
    expect(got).toEqual(['forward']);
  });
  it('a short or diagonal movement does nothing', () => {
    const d = createSwipeDetector();
    expect(d.feed(-(SWIPE_DISTANCE - 20), 0, 0)).toBeNull();
    const diag = createSwipeDetector();
    const got = Array.from({ length: 30 }, (_, i) => diag.feed(-10, 9, i * 16)).filter(Boolean);
    expect(got).toEqual([]);
  });
  it('a vertical scroll that drifts sideways never navigates', () => {
    const d = createSwipeDetector();
    const got = Array.from({ length: 60 }, (_, i) => d.feed(-3, 20, i * 16)).filter(Boolean);
    expect(got).toEqual([]);
  });
  it('the next gesture starts after a quiet gap', () => {
    const d = createSwipeDetector();
    let t = 0;
    const go = (dx: number) => { const out: (string | null)[] = []; for (let i = 0; i < 20; i++) { out.push(d.feed(dx, 0, t)); t += 16; } return out.filter(Boolean); };
    expect(go(-15)).toEqual(['back']);
    t += SWIPE_IDLE_MS + 1;
    expect(go(15)).toEqual(['forward']);
  });
});

describe('a box that scrolls sideways keeps its swipe', () => {
  const box = (scrollLeft: number, scrollWidth = 800, clientWidth = 400, parentElement: any = null) => ({ scrollLeft, scrollWidth, clientWidth, parentElement });
  const auto = () => 'auto';
  it('back is blocked while the box can still scroll toward its start', () => {
    expect(scrollsSideways(box(100), 'back', auto)).toBe(true);
    expect(scrollsSideways(box(0), 'back', auto)).toBe(false);
  });
  it('forward is blocked while it can scroll toward its end', () => {
    expect(scrollsSideways(box(0), 'forward', auto)).toBe(true);
    expect(scrollsSideways(box(400), 'forward', auto)).toBe(false);
  });
  it('a wide box with hidden overflow, or none, does not block, and a parent can', () => {
    expect(scrollsSideways(box(100), 'back', () => 'hidden')).toBe(false);
    expect(scrollsSideways(box(0, 400, 400), 'forward', auto)).toBe(false);
    expect(scrollsSideways(box(0, 400, 400, box(50)), 'back', auto)).toBe(true);
    expect(scrollsSideways(null, 'back', auto)).toBe(false);
  });
});

describe('wiring', () => {
  it('is a passive wheel listener in the window, off with its Settings switch, and skips editors and dialogs', () => {
    expect(app).toContain("listen(document, 'wheel', createHistoryWheel({ enabled: swipeNav, blocked: () => modalOpen() || recordingShortcut(), back: navBack, forward: navForward }), { passive: true })");
    expect(source).toContain('!deps.enabled()');
    expect(source).toContain('deps.blocked()');
    expect(source).toContain('.cm-editor');
  });
  it('uses the same steps as the buttons and keys', () => {
    expect(app).toContain('back: navBack, forward: navForward');
  });
});
