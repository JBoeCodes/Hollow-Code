/**
 * The transcript's frame budget (Canvas C3, plan §4): at most
 * `FRAME_BUDGET` live canvas and html frames per transcript. A frame off
 * screen unmounts to a placeholder of the height it last had, and mounts
 * again when it comes back near the viewport; its saved state comes back
 * through bridge `init` on that load.
 *
 * Visibility is the host's own IntersectionObserver, rooted at the
 * transcript's scroller. Nothing here runs inside a frame: WKWebView's
 * ResizeObserver dies in sandboxed frames, and an off-screen frame is not
 * running at all.
 *
 * Which frames are live (`chooseLive`): only frames near the viewport
 * (within `NEAR_MARGIN`) are eligible. Frames on screen come first, then
 * those just outside it; within each, the one that arrived (or was asked
 * for) most recently. So scrolling keeps the frames ahead of you and drops
 * the ones behind you, and a frame you click "Show" on wins its place.
 *
 * This module loads with the frame (`HtmlArtifact`); the transcript only
 * provides a scope object (`frameBudgetScope.ts`), so the App chunk carries
 * none of it.
 */
import type { FrameBudgetScope } from './frameBudgetScope';

/** Live frames per transcript, at most. */
export const FRAME_BUDGET = 6;
/** How far beyond the visible part of the transcript a frame still counts
 *  as near (CSS px, above and below), so it is loaded before it scrolls in. */
export const NEAR_MARGIN = '600px 0px';

/** One frame as the budget sees it. `seen`: when it last came on screen or
 *  near it, or was asked for (a counter, higher is more recent). */
export interface SlotView {
  id: number;
  visible: boolean;
  near: boolean;
  seen: number;
}

/** The slots that are live: on screen before near, then the most recently
 *  seen, at most `cap`. A slot neither on screen nor near is never live. */
export function chooseLive(slots: readonly SlotView[], cap = FRAME_BUDGET): Set<number> {
  const ranked = slots
    .filter(s => s.visible || s.near)
    .sort((a, b) => Number(b.visible) - Number(a.visible) || b.seen - a.seen || a.id - b.id);
  return new Set(ranked.slice(0, Math.max(0, cap)).map(s => s.id));
}

/** The part of IntersectionObserver the budget uses. */
export interface BudgetObserver {
  observe(el: Element): void;
  unobserve(el: Element): void;
  disconnect(): void;
}
export type ObserverFactory = (
  onChange: (entries: ReadonlyArray<{ target: Element; isIntersecting: boolean }>) => void,
  options: { root: Element | null; rootMargin: string },
) => BudgetObserver | null;

const browserObserver: ObserverFactory = (onChange, options) =>
  typeof IntersectionObserver === 'undefined'
    ? null
    : new IntersectionObserver(entries => onChange(entries), { root: options.root, rootMargin: options.rootMargin, threshold: 0 });

export interface FrameSlot {
  /** The person asked for this frame (its Show button, focus): it wins a place if it is near. */
  touch(): void;
  dispose(): void;
}

/** How a frame joins a budget. `pinned` (Canvas C6 · K2): a frame outside
 *  the transcript that shares its budget (the side panel's curated section
 *  view). It is not observed: it counts as on screen from the moment it
 *  registers, like a frame that just scrolled in, so the transcript keeps
 *  one place fewer while it shows. Frames that come on screen later can
 *  still take its place; its Show (`touch`) takes it back. */
export interface SlotOptions { pinned?: boolean }

export interface FrameBudget {
  /** Track `el` (the frame's slot). `onLive` hears each change of whether
   *  its frame may be mounted; it starts not live until the first
   *  visibility report. Frames leaving are told before frames joining, so
   *  the transcript never holds more than the cap. */
  register(el: Element, onLive: (live: boolean) => void, options?: SlotOptions): FrameSlot;
  /** How many slots are live now. */
  liveCount(): number;
}

export interface FrameBudgetOptions {
  cap?: number;
  /** The transcript's scroller (null: the viewport). Read when the first
   *  slot registers. */
  root?: () => Element | null | undefined;
  /** Tests pass their own; without IntersectionObserver every slot counts as
   *  on screen, so the newest `cap` are live and the cap still holds. */
  observe?: ObserverFactory;
}

interface Slot extends SlotView {
  el: Element;
  live: boolean;
  pinned: boolean;
  onLive: (live: boolean) => void;
}

export function createFrameBudget(options: FrameBudgetOptions = {}): FrameBudget {
  const cap = options.cap ?? FRAME_BUDGET;
  const factory = options.observe ?? browserObserver;
  const slots = new Map<Element, Slot>();
  let clock = 0;
  let nextId = 0;
  let observers: { visible: BudgetObserver; near: BudgetObserver } | null | undefined;

  const apply = () => {
    const chosen = chooseLive([...slots.values()], cap);
    const all = [...slots.values()];
    for (const s of all) if (s.live && !chosen.has(s.id)) { s.live = false; s.onLive(false); }
    for (const s of all) if (!s.live && chosen.has(s.id) && slots.get(s.el) === s) { s.live = true; s.onLive(true); }
  };
  const report = (key: 'visible' | 'near') => (entries: ReadonlyArray<{ target: Element; isIntersecting: boolean }>) => {
    let changed = false;
    for (const e of entries) {
      const s = slots.get(e.target);
      if (!s || s[key] === e.isIntersecting) continue;
      s[key] = e.isIntersecting;
      if (e.isIntersecting) s.seen = ++clock;
      changed = true;
    }
    if (changed) apply();
  };
  const ensureObservers = () => {
    if (observers !== undefined) return observers;
    const root = options.root?.() ?? null;
    const visible = factory(report('visible'), { root, rootMargin: '0px' });
    const near = visible ? factory(report('near'), { root, rootMargin: NEAR_MARGIN }) : null;
    if (visible && near) observers = { visible, near };
    else { visible?.disconnect(); observers = null; }
    return observers;
  };

  return {
    register(el, onLive, opts) {
      const pinned = !!opts?.pinned;
      const slot: Slot = { id: nextId++, el, visible: false, near: false, seen: 0, live: false, pinned, onLive };
      slots.set(el, slot);
      const obs = pinned ? null : ensureObservers();
      if (obs) { obs.visible.observe(el); obs.near.observe(el); }
      else { slot.visible = slot.near = true; slot.seen = ++clock; apply(); }
      return {
        touch() {
          if (slots.get(el) !== slot) return;
          slot.seen = ++clock;
          apply();
        },
        dispose() {
          if (slots.get(el) !== slot) return;
          slots.delete(el);
          if (obs) { obs.visible.unobserve(el); obs.near.unobserve(el); }
          // Its place goes to the next frame in line.
          if (slot.live) apply();
        },
      };
    },
    liveCount: () => [...slots.values()].filter(s => s.live).length,
  };
}

const budgets = new WeakMap<FrameBudgetScope, FrameBudget>();

/** The budget of a transcript's scope, made on first use. */
export function budgetFor(scope: FrameBudgetScope): FrameBudget {
  let budget = budgets.get(scope);
  if (!budget) budgets.set(scope, budget = createFrameBudget({ root: scope.root }));
  return budget;
}
