import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// Settings' jump lives in its navigation controller (L4, X18).
import settingsSource from './settings/controllers/settingsNavigation.ts?raw';
import { KEEP_IN_VIEW_MS, controlNamed, keepInView, landOnPageControl, preferExact, retryFind } from './settingsJump';
import pageTargetSource from './store/settingsPageTarget.ts?raw';
import railSource from './components/Rail.tsx?raw';
import source from './settingsJump.ts?raw';

describe('Settings jump to a control in a lazy panel (morning P3)', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('keeps looking while the panel loads, then lands once', () => {
    let ready = false;
    const found = vi.fn();
    retryFind(() => (ready ? 'row' : null), found, () => true);
    expect(found).not.toHaveBeenCalled();
    vi.advanceTimersByTime(500);
    ready = true;
    vi.advanceTimersByTime(100);
    expect(found).toHaveBeenCalledTimes(1);
    expect(found).toHaveBeenCalledWith('row');
    vi.advanceTimersByTime(1000);
    expect(found).toHaveBeenCalledTimes(1);
  });

  it('gives up after the limit, and stops when the person moves on or it is cancelled', () => {
    const find = vi.fn(() => null);
    retryFind(find, vi.fn(), () => true, { limit: 1000 });
    vi.advanceTimersByTime(5000);
    expect(find.mock.calls.length).toBeLessThanOrEqual(12);
    let wanted = true;
    const later = vi.fn(() => null);
    retryFind(later, vi.fn(), () => wanted);
    vi.advanceTimersByTime(300);
    wanted = false;
    const calls = later.mock.calls.length;
    vi.advanceTimersByTime(1000);
    expect(later.mock.calls.length).toBe(calls);
    const cancelled = vi.fn(() => null);
    const cancel = retryFind(cancelled, vi.fn(), () => true);
    cancel();
    vi.advanceTimersByTime(1000);
    expect(cancelled).toHaveBeenCalledTimes(1);
  });

  it('is what Settings\' jump uses', () => {
    expect(settingsSource).toContain('retryFind(find, land, () => jumps.isLatest(gen) && activeSection() === section, { limit: JUMP_LIMIT_MS + EXACT_GRACE_MS })');
  });
});

describe('the jump prefers the control\'s own row while the section is still loading (overnight A07 review)', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('waits a moment for the row that names the control before settling for an earlier match', () => {
    let row: string | null = null;
    const found = vi.fn();
    retryFind(preferExact(() => row, () => 'explainer card', 400), found, () => true);
    vi.advanceTimersByTime(200);
    expect(found).not.toHaveBeenCalled();
    row = 'switch row';
    vi.advanceTimersByTime(100);
    expect(found).toHaveBeenCalledTimes(1);
    expect(found).toHaveBeenCalledWith('switch row');
  });

  it('lands on the earlier match once the grace has passed and the row never came', () => {
    const found = vi.fn();
    retryFind(preferExact(() => null, () => 'explainer card', 400), found, () => true);
    vi.advanceTimersByTime(300);
    expect(found).not.toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(found).toHaveBeenCalledWith('explainer card');
  });

  it('takes the row at once when it is already there, and finds nothing while neither is', () => {
    const now = vi.fn(() => 0);
    expect(preferExact(() => 'row', () => 'card', 400, now)()).toBe('row');
    const find = preferExact(() => null, () => null, 400, now);
    expect(find()).toBeNull();
    now.mockReturnValue(10_000);
    expect(find()).toBeNull();
  });

  it('is what Settings\' jump uses', () => {
    expect(settingsSource).toContain('preferExact<HTMLElement>(');
  });
});

describe('a jump keeps its target in view while the section fills in (S2 review)', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });
  function hold() {
    let resize = () => {};
    let interrupt = () => {};
    const unsubscribed = { resize: 0, interrupt: 0 };
    const rescroll = vi.fn();
    const stop = keepInView({
      onResize: changed => { resize = changed; return () => { unsubscribed.resize++; }; },
      onInterrupt: s => { interrupt = s; return () => { unsubscribed.interrupt++; }; },
      rescroll,
    });
    return { resize: () => resize(), interrupt: () => interrupt(), rescroll, stop, unsubscribed };
  }

  it('re-scrolls on each size change until the limit, then lets go', () => {
    const h = hold();
    h.resize(); h.resize();
    expect(h.rescroll).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(KEEP_IN_VIEW_MS);
    h.resize();
    expect(h.rescroll).toHaveBeenCalledTimes(2);
    expect(h.unsubscribed).toEqual({ resize: 1, interrupt: 1 });
  });

  it('stops for good once the person scrolls or moves focus', () => {
    const h = hold();
    h.resize();
    h.interrupt();
    h.resize();
    expect(h.rescroll).toHaveBeenCalledTimes(1);
    expect(h.unsubscribed).toEqual({ resize: 1, interrupt: 1 });
    h.stop();
    expect(h.unsubscribed).toEqual({ resize: 1, interrupt: 1 });
  });

  it('is what a landed Settings jump uses, and a later jump drops it', () => {
    expect(settingsSource).toContain('cancelHold = keepInView({');
    expect(settingsSource).toContain('    cancelJump?.();\n    cancelHold?.();');
  });
});

/** A fake element for the page-control finder (no DOM in this suite). */
function el(o: { control?: string; aria?: string; text?: string; heading?: boolean; inNav?: boolean }) {
  return {
    dataset: { settingsControl: o.control },
    getAttribute: (n: string) => (n === 'aria-label' ? o.aria ?? null : null),
    textContent: o.text ?? null,
    matches: (sel: string) => (sel.includes('h3') ? !!o.heading : false),
    closest: (sel: string) => (sel.startsWith('nav') && o.inNav ? {} : null),
    parentElement: null,
    scrollIntoView: vi.fn(),
    focus: vi.fn(),
    querySelector: () => null,
  };
}

describe('an old link into Library lands on its control (overnight 11-13 S1)', () => {
  it('prefers an exactly named element, then a heading that starts with the label, never a nav or tab strip', () => {
    const tab = el({ aria: 'Ready to add', inNav: true });
    const heading = el({ text: '  Ready to add  (3)', heading: true });
    const block = el({ aria: 'Ready to add' });
    expect(controlNamed([tab, heading, block], 'Ready to add')).toBe(block);
    expect(controlNamed([tab, heading], 'Ready to add')).toBe(heading);
    expect(controlNamed([tab], 'Ready to add')).toBeUndefined();
    expect(controlNamed([el({ control: 'Automated-write runs' })], 'Automated-write runs')).toBeDefined();
  });

  it('looks again while the tab loads, then scrolls to the control', () => {
    const later = el({ aria: 'Ready to add' });
    let shown: ReturnType<typeof el>[] = [];
    const queue: (() => void)[] = [];
    landOnPageControl('Ready to add', () => true, () => shown as never, { schedule: f => queue.push(f), now: () => 0, onInterrupt: () => () => {} });
    expect(queue).toHaveLength(1);
    shown = [later];
    queue.shift()!();
    expect(later.scrollIntoView).toHaveBeenCalledWith({ block: 'center' });
  });

  it('stops when a newer landing starts, the page changes or the person acts (review of #1369)', () => {
    const target = el({ aria: 'Available to' });
    const run = (wanted: () => boolean, interrupt?: (stop: () => void) => void) => {
      const queue: (() => void)[] = [];
      landOnPageControl('Available to', wanted, () => [] as never, { schedule: f => queue.push(f), now: () => 0, onInterrupt: stop => { interrupt?.(stop); return () => {}; } });
      return queue;
    };
    // A newer landing cancels the older: its queued look finds nothing to do.
    const older = run(() => true);
    const newer = run(() => true);
    landOnPageControl('Available to', () => true, () => [target] as never, { schedule: () => {}, now: () => 0, onInterrupt: () => () => {} });
    expect(target.scrollIntoView).toHaveBeenCalledTimes(1);
    target.scrollIntoView.mockClear();
    older.forEach(f => f()); newer.forEach(f => f());
    expect(target.scrollIntoView).not.toHaveBeenCalled();
    // The page changed: no more looks.
    let onPage = true;
    const leaving = run(() => onPage);
    onPage = false;
    leaving.shift()!();
    expect(leaving).toHaveLength(0);
    // Input stops it.
    let stopIt!: () => void;
    const typing = run(() => true, stop => { stopIt = stop; });
    stopIt();
    typing.shift()!();
    expect(typing).toHaveLength(0);
  });

  it('removes its input listeners when the limit passes with no match', () => {
    const queue: (() => void)[] = [];
    let t = 0;
    const unlisten = vi.fn();
    landOnPageControl('Nothing here', () => true, () => [] as never, { schedule: f => queue.push(f), now: () => t, limit: 200, onInterrupt: () => unlisten });
    t = 100; queue.shift()!();
    expect(unlisten).not.toHaveBeenCalled();
    t = 250; queue.shift()!();
    expect(unlisten).toHaveBeenCalled();
    expect(queue).toHaveLength(0);
  });

  it('looks only inside the opened page, and only at shown elements', () => {
    expect(source).toContain("document.querySelector('.canvas-split > .page') ?? document");
    expect(source).toContain('.filter(el => el.getClientRects().length > 0)');
  });

  it('is what a moved target opens with, past its tab name', () => {
    expect(pageTargetSource).toContain("label !== MOVED_LABELS[label]?.search?.tabLabel");
    expect(pageTargetSource).toContain("void import('../settingsJump').then(j => j.landOnPageControl(label, () => activePage() === page));");
    expect(pageTargetSource).toContain("else if (landingLoaded) void import('../settingsJump').then(j => j.cancelPageLanding());");
    expect(settingsSource).toContain('label: target.label ?? target.tab ?? target.page,');
  });
});

// #1332 review lows. No DOM in this suite and both live in components'
// setup, so the source is checked: the mark's timer is cleared with
// Settings, and hiding the sidebar forgets where Search was opened from.
describe('nothing left behind (overnight 11-13 S3)', () => {
  it('clears the found mark timer when Settings closes', () => {
    expect(settingsSource).toContain('onCleanup(() => { cancelJump?.(); cancelHold?.(); clearTimeout(foundTimer); });');
  });

  it('drops the Rail search return target when the sidebar hides', () => {
    expect(railSource).toContain('if (hidden) { setSearching(false); focusReturn.forget(); shownForSearch = false; }');
  });
});
