import { describe, expect, it } from 'vitest';
import { FRAME_BUDGET, NEAR_MARGIN, budgetFor, chooseLive, createFrameBudget, type ObserverFactory } from './frameBudget';

/** Two fake IntersectionObservers (on screen, near), driven by hand. */
function fakeObservers() {
  const made: Array<{ margin: string; root: Element | null; onChange: Parameters<ObserverFactory>[0]; watched: Set<Element> }> = [];
  const factory: ObserverFactory = (onChange, { root, rootMargin }) => {
    const o = { margin: rootMargin, root, onChange, watched: new Set<Element>() };
    made.push(o);
    return { observe: el => o.watched.add(el), unobserve: el => o.watched.delete(el), disconnect: () => o.watched.clear() };
  };
  const by = (margin: string) => made.find(o => o.margin === margin)!;
  /** Report `els` as on screen (and near), `near` as only near, every other watched slot as neither. */
  const show = (onScreen: Element[], nearOnly: Element[] = []) => {
    const visible = by('0px'), near = by(NEAR_MARGIN);
    visible.onChange([...visible.watched].map(target => ({ target, isIntersecting: onScreen.includes(target) })));
    near.onChange([...near.watched].map(target => ({ target, isIntersecting: onScreen.includes(target) || nearOnly.includes(target) })));
  };
  return { factory, made, show };
}

const el = () => ({}) as Element;

describe('chooseLive', () => {
  it('keeps on-screen frames before near ones, then the most recently seen, up to the cap', () => {
    const slots = [
      { id: 0, visible: false, near: true, seen: 9 },
      { id: 1, visible: true, near: true, seen: 1 },
      { id: 2, visible: false, near: false, seen: 10 },
      { id: 3, visible: true, near: true, seen: 5 },
      { id: 4, visible: false, near: true, seen: 2 },
    ];
    expect([...chooseLive(slots, 3)]).toEqual([3, 1, 0]);
    expect(chooseLive(slots, 0).size).toBe(0);
    // A slot neither on screen nor near is never live, even with room.
    expect(chooseLive(slots, 6).has(2)).toBe(false);
  });

  it('is six frames by default', () => {
    expect(FRAME_BUDGET).toBe(6);
    const slots = Array.from({ length: 12 }, (_, id) => ({ id, visible: true, near: true, seen: id }));
    expect([...chooseLive(slots)].sort((a, b) => a - b)).toEqual([6, 7, 8, 9, 10, 11]);
  });
});

describe('createFrameBudget', () => {
  it('starts a frame unmounted and mounts it once it is reported near', () => {
    const io = fakeObservers();
    const root = el();
    const budget = createFrameBudget({ observe: io.factory, root: () => root });
    const a = el();
    const seen: boolean[] = [];
    budget.register(a, live => seen.push(live));
    expect(seen).toEqual([]);
    expect(io.made.map(o => [o.margin, o.root])).toEqual([['0px', root], [NEAR_MARGIN, root]]);
    io.show([], [a]);
    expect(seen).toEqual([true]);
    io.show([]);
    expect(seen).toEqual([true, false]);
    expect(budget.liveCount()).toBe(0);
  });

  it('never has more than the cap live while twelve frames scroll past, and drops the ones behind', () => {
    const io = fakeObservers();
    const budget = createFrameBudget({ observe: io.factory });
    const els = Array.from({ length: 12 }, el);
    const live = new Map<Element, boolean>();
    let most = 0;
    for (const e of els) budget.register(e, on => { live.set(e, on); most = Math.max(most, budget.liveCount()); });
    // Scroll down: a window of three on screen with two near on each side.
    for (let top = 0; top < 10; top++) {
      const onScreen = els.slice(top, top + 3);
      const near = [...els.slice(Math.max(0, top - 2), top), ...els.slice(top + 3, top + 5)];
      io.show(onScreen, near);
      expect(budget.liveCount()).toBeLessThanOrEqual(FRAME_BUDGET);
      for (const e of onScreen) expect(live.get(e)).toBe(true);
      // Anything far away is unmounted.
      for (const e of els) if (!onScreen.includes(e) && !near.includes(e)) expect(live.get(e) ?? false).toBe(false);
    }
    expect(most).toBeLessThanOrEqual(FRAME_BUDGET);
    // Back to the top: the first frames mount again.
    io.show(els.slice(0, 3), els.slice(3, 5));
    expect(els.slice(0, 3).map(e => live.get(e))).toEqual([true, true, true]);
  });

  it('tells leaving frames before joining ones, so the count never passes the cap', () => {
    const io = fakeObservers();
    const budget = createFrameBudget({ observe: io.factory, cap: 2 });
    const [a, b, c, d] = [el(), el(), el(), el()];
    const order: string[] = [];
    const counts: number[] = [];
    for (const [e, n] of [[a, 'a'], [b, 'b'], [c, 'c'], [d, 'd']] as const) {
      budget.register(e, on => { order.push(`${n}${on ? '+' : '-'}`); counts.push(budget.liveCount()); });
    }
    io.show([a, b]);
    io.show([c, d]);
    expect(order).toEqual(['a+', 'b+', 'a-', 'b-', 'c+', 'd+']);
    expect(Math.max(...counts)).toBeLessThanOrEqual(2);
  });

  it('gives a frame the person asks for a place, and frees a place when one goes', () => {
    const io = fakeObservers();
    const budget = createFrameBudget({ observe: io.factory, cap: 2 });
    const [a, b, c] = [el(), el(), el()];
    const live = new Map<Element, boolean>();
    const slots = [a, b, c].map(e => budget.register(e, on => live.set(e, on)));
    io.show([a, b, c]);
    // All three on screen, two places: the first reported in report order is oldest.
    expect([a, b, c].map(e => live.get(e) ?? false)).toEqual([false, true, true]);
    slots[0].touch();
    expect(live.get(a)).toBe(true);
    expect(budget.liveCount()).toBe(2);
    slots[2].dispose();
    expect(budget.liveCount()).toBe(2);
    expect([a, b].map(e => live.get(e))).toEqual([true, true]);
    // A disposed slot is forgotten: its observer no longer watches it.
    expect(io.made.every(o => !o.watched.has(c))).toBe(true);
    slots[2].touch();
    expect(budget.liveCount()).toBe(2);
  });

  it('without IntersectionObserver every frame counts as on screen and the newest six are live', () => {
    const budget = createFrameBudget({ observe: () => null });
    const live: boolean[] = [];
    for (let i = 0; i < 8; i++) budget.register(el(), on => { live[i] = on; });
    expect(budget.liveCount()).toBe(FRAME_BUDGET);
    expect(live.slice(0, 2)).toEqual([false, false]);
    expect(live.slice(2)).toEqual([true, true, true, true, true, true]);
  });

  it('counts a pinned panel frame toward the transcript budget, on screen at once and never observed (C6 · K2)', () => {
    const io = fakeObservers();
    const budget = createFrameBudget({ observe: io.factory });
    const els = Array.from({ length: 6 }, el);
    const live = new Map<Element, boolean>();
    for (const e of els) budget.register(e, on => live.set(e, on));
    io.show(els);
    expect(budget.liveCount()).toBe(FRAME_BUDGET);
    // The side panel's curated view joins: live at once, and the transcript
    // gives up its least recently seen frame so the total stays at the cap.
    const panel = el();
    const slot = budget.register(panel, on => live.set(panel, on), { pinned: true });
    expect(live.get(panel)).toBe(true);
    expect(budget.liveCount()).toBe(FRAME_BUDGET);
    expect(els.filter(e => live.get(e)).length).toBe(FRAME_BUDGET - 1);
    expect(io.made.every(o => !o.watched.has(panel))).toBe(true);
    // Visibility reports for the transcript never unmount it.
    io.show(els.slice(0, 2));
    expect(live.get(panel)).toBe(true);
    // When it goes, its place returns to the transcript.
    io.show(els);
    slot.dispose();
    expect(els.filter(e => live.get(e)).length).toBe(FRAME_BUDGET);
  });

  it('a frame scrolled in later can take a pinned frame’s place, and Show takes it back', () => {
    const io = fakeObservers();
    const budget = createFrameBudget({ observe: io.factory, cap: 2 });
    const panel = el(), a = el(), b = el();
    const live = new Map<Element, boolean>();
    const pinned = budget.register(panel, on => live.set(panel, on), { pinned: true });
    budget.register(a, on => live.set(a, on));
    budget.register(b, on => live.set(b, on));
    io.show([a]);
    expect([live.get(panel), live.get(a)]).toEqual([true, true]);
    io.show([a, b]);
    expect(live.get(panel)).toBe(false);
    expect(budget.liveCount()).toBe(2);
    pinned.touch();
    expect(live.get(panel)).toBe(true);
    expect(budget.liveCount()).toBe(2);
  });

  it('keeps one budget per transcript scope', () => {
    const scope = { root: () => null };
    expect(budgetFor(scope)).toBe(budgetFor(scope));
    expect(budgetFor(scope)).not.toBe(budgetFor({ root: () => null }));
  });
});
