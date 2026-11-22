import { afterEach, describe, expect, it, vi } from 'vitest';
import { PAGE_NAME, START_LIMIT_MS, TRANSITIONING, canTransition, flushViewTransition, morphFrom, withViewTransition } from './viewTransitions';

/** A document with just what viewTransitions.ts reads. `engine` decides what
 *  startViewTransition does with the update: run it ('run'), never ('stall'),
 *  or throw ('throw'). */
function fakeDoc(opts: { motion?: string; reduce?: boolean; hidden?: boolean; away?: boolean; engine?: 'run' | 'stall' | 'throw' | 'none' } = {}) {
  const classes = new Set<string>();
  const calls: string[] = [];
  let skipped = false;
  let engineUpdate: (() => Promise<void> | void) | undefined;
  const doc = {
    documentElement: { dataset: { motion: opts.motion ?? 'full' }, classList: { add: (c: string) => classes.add(c), remove: (c: string) => classes.delete(c), contains: (c: string) => classes.has(c) } },
    defaultView: { matchMedia: () => ({ matches: !!opts.reduce }) },
    visibilityState: opts.hidden ? 'hidden' : 'visible',
    body: { classList: { contains: (c: string) => c === 'is-away' && !!opts.away } },
    querySelector: () => null,
    startViewTransition: opts.engine === 'none' ? undefined : (update: () => Promise<void> | void) => {
      calls.push('start');
      engineUpdate = update;
      if (opts.engine === 'throw') throw new Error('no');
      let done!: () => void;
      const callbackDone = opts.engine === 'stall'
        ? new Promise<void>(() => undefined)
        : Promise.resolve().then(() => update()).then(() => undefined);
      const finished = new Promise<void>(resolve => { done = resolve; });
      void callbackDone.then(() => done());
      return { updateCallbackDone: callbackDone, ready: callbackDone, finished, skipTransition: () => { skipped = true; done(); } };
    },
  };
  return { doc: doc as unknown as Document, classes, calls, skipped: () => skipped, lateUpdate: () => engineUpdate?.() };
}

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('local detail morph', () => {
  it('restores distinct inline names after a guard timeout and ignores a late engine update', async () => {
    vi.useFakeTimers();
    class Element { isConnected = true; constructor(public style: { viewTransitionName: string }) {} }
    vi.stubGlobal('HTMLElement', Element);
    const source = new Element({ viewTransitionName: 'source-before' });
    const detail = new Element({ viewTransitionName: 'detail-before' });
    const page = new Element({ viewTransitionName: 'page-before' });
    const f = fakeDoc({ engine: 'stall' });
    f.doc.querySelector = (() => page) as typeof f.doc.querySelector;
    morphFrom(source as unknown as HTMLElement, detail as unknown as HTMLElement);
    const update = vi.fn(() => 'selected');
    const out = withViewTransition(update, 0, f.doc);
    expect(source.style.viewTransitionName).toBe(PAGE_NAME);
    await vi.advanceTimersByTimeAsync(START_LIMIT_MS);
    await expect(out).resolves.toBe('selected');
    expect(f.skipped()).toBe(true);
    expect(f.classes.has(TRANSITIONING)).toBe(false);
    expect([source, detail, page].map(el => el.style.viewTransitionName))
      .toEqual(['source-before', 'detail-before', 'page-before']);
    detail.style.viewTransitionName = 'newer-owner';
    await f.lateUpdate();
    expect(update).toHaveBeenCalledTimes(1);
    expect(detail.style.viewTransitionName).toBe('newer-owner');
    expect(canTransition(f.doc)).toBe(true);
  });

  it('never reassigns detail names when a flushed engine callback arrives late', async () => {
    class Element { isConnected = true; style = { viewTransitionName: '' }; }
    vi.stubGlobal('HTMLElement', Element);
    const source = new Element(), detail = new Element(), page = new Element();
    const f = fakeDoc({ engine: 'stall' });
    f.doc.querySelector = (() => page) as typeof f.doc.querySelector;
    morphFrom(source as unknown as HTMLElement, detail as unknown as HTMLElement);
    const update = vi.fn();
    const out = withViewTransition(update, 0, f.doc);
    flushViewTransition();
    for (const el of [source, detail, page]) expect(el.style.viewTransitionName).toBe('');
    const newer = vi.fn();
    withViewTransition(newer, 0, f.doc);
    expect(newer).toHaveBeenCalledTimes(1);
    await out;
    await f.lateUpdate();
    expect(update).toHaveBeenCalledTimes(1);
    for (const el of [source, detail, page]) expect(el.style.viewTransitionName).toBe('');
  });
  it('lets an immediate keyboard or close choice supersede a waiting row click', async () => {
    const f = fakeDoc({ engine: 'stall' });
    let selection: string | null = null;
    const out = withViewTransition(() => { selection = 'clicked'; }, 0, f.doc);
    flushViewTransition();
    selection = 'keyboard';
    await out;
    expect(selection).toBe('keyboard');
    expect(f.skipped()).toBe(true);
  });
  it('names only the row then detail, keeps the page still and restores owned inline names', async () => {
    class Element { isConnected = true; style = { viewTransitionName: 'existing' }; }
    vi.stubGlobal('HTMLElement', Element);
    const source = new Element(), detail = new Element(), page = new Element();
    const f = fakeDoc();
    f.doc.querySelector = (() => page) as typeof f.doc.querySelector;
    morphFrom(source as unknown as HTMLElement, detail as unknown as HTMLElement);
    const out = withViewTransition(() => {
      expect(source.style.viewTransitionName).toBe('existing');
      expect(detail.style.viewTransitionName).toBe(PAGE_NAME);
      expect(page.style.viewTransitionName).toBe('none');
      return 'selected';
    }, 0, f.doc);
    expect(source.style.viewTransitionName).toBe(PAGE_NAME);
    expect(detail.style.viewTransitionName).toBe('none');
    expect(page.style.viewTransitionName).toBe('none');
    await expect(out).resolves.toBe('selected');
    await Promise.resolve();
    for (const el of [source, detail, page]) expect(el.style.viewTransitionName).toBe('existing');
  });

  it('cuts immediately with reduced motion and never leaves names on either element', () => {
    class Element { isConnected = true; style = { viewTransitionName: '' }; }
    vi.stubGlobal('HTMLElement', Element);
    const source = new Element(), detail = new Element();
    morphFrom(source as unknown as HTMLElement, detail as unknown as HTMLElement);
    const f = fakeDoc({ reduce: true });
    const update = vi.fn(() => 'selected');
    expect(withViewTransition(update, 0, f.doc)).toBe('selected');
    expect(update).toHaveBeenCalledTimes(1);
    expect(source.style.viewTransitionName).toBe('');
    expect(detail.style.viewTransitionName).toBe('');
  });
});

describe('canTransition', () => {
  it('is off without the API, with either motion gate, and in a hidden or away window', () => {
    expect(canTransition(fakeDoc({ engine: 'none' }).doc)).toBe(false);
    expect(canTransition(fakeDoc({ motion: 'reduced' }).doc)).toBe(false);
    expect(canTransition(fakeDoc({ reduce: true }).doc)).toBe(false);
    expect(canTransition(fakeDoc({ hidden: true }).doc)).toBe(false);
    expect(canTransition(fakeDoc({ away: true }).doc)).toBe(false);
    expect(canTransition(fakeDoc().doc)).toBe(true);
    expect(canTransition(undefined)).toBe(false);
  });

  it('is off during the launch entrance', () => {
    const f = fakeDoc();
    f.classes.add('is-booting');
    expect(canTransition(f.doc)).toBe(false);
  });
});

describe('withViewTransition', () => {
  it('runs the update directly, once, when motion is reduced', () => {
    const f = fakeDoc({ motion: 'reduced' });
    const update = vi.fn(() => 7);
    expect(withViewTransition(update, 0, f.doc)).toBe(7);
    expect(update).toHaveBeenCalledTimes(1);
    expect(f.calls).toEqual([]);
  });

  it('runs the update inside a transition, once, and resolves to its result', async () => {
    const f = fakeDoc();
    const update = vi.fn(async () => 'opened');
    const out = withViewTransition(update, 0, f.doc);
    expect(f.calls).toEqual(['start']);
    expect(f.classes.has(TRANSITIONING)).toBe(true);
    await expect(out).resolves.toBe('opened');
    expect(update).toHaveBeenCalledTimes(1);
    await new Promise(r => setTimeout(r, 0));
    expect(f.classes.has(TRANSITIONING)).toBe(false);
  });

  it('never nests: a navigation inside a running transition applies directly', async () => {
    const f = fakeDoc();
    let inner = 0;
    const out = withViewTransition(() => { withViewTransition(() => { inner++; }, 0, f.doc); }, 0, f.doc);
    await out;
    expect(inner).toBe(1);
    expect(f.calls).toEqual(['start']);
  });

  it('a navigation while the previous one waits for the engine applies that one first, so the later one wins', async () => {
    const f = fakeDoc({ engine: 'stall' });
    const order: string[] = [];
    const first = withViewTransition(() => { order.push('A'); return 'A'; }, 0, f.doc);
    const second = withViewTransition(() => { order.push('B'); return 'B'; }, 0, f.doc);
    expect(order).toEqual(['A', 'B']);
    expect(second).toBe('B');
    expect(f.skipped()).toBe(true);
    await expect(first).resolves.toBe('A');
  });

  it('applies a stalled transition after the start limit and skips its motion', async () => {
    vi.useFakeTimers();
    const f = fakeDoc({ engine: 'stall' });
    const update = vi.fn(() => 'late');
    const out = withViewTransition(update, 0, f.doc);
    expect(update).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(START_LIMIT_MS);
    expect(update).toHaveBeenCalledTimes(1);
    expect(f.skipped()).toBe(true);
    await expect(out).resolves.toBe('late');
  });

  it('applies the update when the engine refuses to start', () => {
    const f = fakeDoc({ engine: 'throw' });
    const update = vi.fn(() => 3);
    expect(withViewTransition(update, 0, f.doc)).toBe(3);
    expect(update).toHaveBeenCalledTimes(1);
    expect(f.classes.has(TRANSITIONING)).toBe(false);
  });

  it('passes an update error to the caller', async () => {
    const f = fakeDoc();
    const out = withViewTransition(() => { throw new Error('boom'); }, 0, f.doc);
    await expect(out).rejects.toThrow('boom');
  });

  it('waits at most settleMs for a slow async update before showing the new page', async () => {
    vi.useFakeTimers();
    const f = fakeDoc();
    let resolveLoad!: (v: boolean) => void;
    const load = new Promise<boolean>(r => { resolveLoad = r; });
    const out = withViewTransition(() => load, 120, f.doc);
    await vi.advanceTimersByTimeAsync(120);
    resolveLoad(true);
    await expect(out).resolves.toBe(true);
  });
});

// Overnight 10-11/12 P4: while a transition plays, Chromium hit-tests the
// document element, so a click in the rail just after opening a workspace was
// lost (the warm-switch "stall": Playwright waited ~300–830 ms for its click
// to land). The click skips the motion and reaches what is under the pointer.
describe('clicks on the transition overlay', () => {
  function clickDoc() {
    const f = fakeDoc();
    const listeners: Record<string, ((e: unknown) => void)[]> = {};
    const received: { type: string; clientX: number; metaKey: boolean }[] = [];
    let underPointer: 'rail' | 'page' | 'root' | 'field' = 'rail';
    const focused: string[] = [];
    const el = (inPage: boolean) => ({
      closest: (sel: string) => (inPage && sel === '.canvas-split > .page' ? {} : sel.startsWith('input') && underPointer === 'field' ? { focus: () => focused.push('field') } : null),
      dispatchEvent: (e: { type: string; clientX: number; metaKey: boolean }) => { received.push(e); return true; },
    });
    const d = f.doc as unknown as Record<string, unknown>;
    d.addEventListener = (type: string, fn: (e: unknown) => void) => { (listeners[type] ??= []).push(fn); };
    d.elementFromPoint = () => (underPointer === 'root' ? d.documentElement : el(underPointer === 'page'));
    (d.defaultView as Record<string, unknown>).MouseEvent = class { constructor(type: string, init: object) { Object.assign(this, { type, ...init }); } };
    const fire = (type: string, opts: { target?: unknown; trusted?: boolean; x?: number; detail?: number; pressAt?: number | null } = {}) => {
      // A click comes after its press: on the overlay, where it ends unless `pressAt` says otherwise.
      if (type !== 'pointerdown' && opts.pressAt !== null) fire('pointerdown', { target: opts.target, trusted: opts.trusted, x: opts.pressAt ?? opts.x });
      const e = { type, target: opts.target ?? d.documentElement, isTrusted: opts.trusted ?? true, clientX: opts.x ?? 40, clientY: 300, screenX: 0, screenY: 0, button: 0, buttons: 0, detail: opts.detail ?? 1, ctrlKey: false, shiftKey: false, altKey: false, metaKey: true,
        prevented: false, stopped: false, preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } };
      for (const fn of listeners[type] ?? []) fn(e);
      return e;
    };
    return { ...f, fire, received, focused, listeners, setUnder: (u: typeof underPointer) => { underPointer = u; } };
  }
  const stalled = () => new Promise<void>(() => undefined);

  it('a click in the rail mid-transition ends the motion and reaches the rail, with its coordinates and keys', async () => {
    const c = clickDoc();
    void withViewTransition(() => stalled(), 120, c.doc);
    const e = c.fire('click');
    expect(e.prevented && e.stopped).toBe(true);
    expect(c.skipped()).toBe(true);
    await new Promise(r => setTimeout(r, 0));
    expect(c.received).toEqual([expect.objectContaining({ type: 'click', clientX: 40, metaKey: true, bubbles: true })]);
  });

  it('a right-click reaches the rail too', async () => {
    const c = clickDoc();
    void withViewTransition(() => stalled(), 120, c.doc);
    c.fire('contextmenu');
    await new Promise(r => setTimeout(r, 0));
    expect(c.received.map(e => e.type)).toEqual(['contextmenu']);
  });

  it('a click over the page column only ends the motion: that spot was the old page', async () => {
    const c = clickDoc();
    c.setUnder('page');
    void withViewTransition(() => stalled(), 120, c.doc);
    c.fire('click');
    await new Promise(r => setTimeout(r, 0));
    expect(c.skipped()).toBe(true);
    expect(c.received).toEqual([]);
  });

  it('leaves clicks alone with no transition running, on a real target, or not from the person', async () => {
    const c = clickDoc();
    void withViewTransition(() => stalled(), 120, c.doc);
    expect(c.fire('click', { target: {} }).prevented).toBe(false);
    expect(c.fire('click', { trusted: false }).prevented).toBe(false);
    expect(c.skipped()).toBe(false);
    c.fire('click');
    await new Promise(r => setTimeout(r, 0));
    // Over now: the next click on the root is just a click.
    expect(c.fire('click').prevented).toBe(false);
    // One listener per type, however many transitions ran.
    void withViewTransition(() => undefined, 0, c.doc);
    expect(c.listeners.click).toHaveLength(1);
  });
  it('a drag or a selection that ends on the overlay, or a double-click\'s second click, only ends the motion', async () => {
    const cases: ((c: ReturnType<typeof clickDoc>) => void)[] = [
      c => c.fire('click', { x: 40, pressAt: 120 }),
      c => { c.fire('pointerdown', { target: {} }); c.fire('click', { pressAt: null }); },
      c => c.fire('click', { detail: 2 }),
    ];
    for (const act of cases) {
      const c = clickDoc();
      void withViewTransition(() => stalled(), 120, c.doc);
      act(c);
      await new Promise(r => setTimeout(r, 0));
      expect(c.skipped()).toBe(true);
      expect(c.received).toEqual([]);
    }
  });

  it('a text field it lands on takes focus, as a real click gives', async () => {
    const c = clickDoc();
    c.setUnder('field');
    void withViewTransition(() => stalled(), 120, c.doc);
    c.fire('click');
    await new Promise(r => setTimeout(r, 0));
    expect(c.focused).toEqual(['field']);
    expect(c.received.map(e => e.type)).toEqual(['click']);
  });
});
