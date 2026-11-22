import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// The browser build: the server build vitest resolves runs no effects.
// @ts-expect-error: the browser build has no type declarations of its own.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// R13 (2026-11-15): the transcript scroll controller had only browser specs.
// This one drives createChatScroll against a fake scroller (plain numbers, no
// DOM): it follows the bottom while the reply streams, lets go the moment the
// reader scrolls up, and takes hold again on "Jump to latest". Mocked store,
// manual animation frames; ResizeObserver is left undefined, so the observers
// that need a real layout stay off.
// `key` differs per test: the controller keeps scroll places by conversation
// at module level, and a place saved by one test would restore into the next.
const st = vi.hoisted(() => ({ key: 'conv-0', turns: [] as { text: string; tools: unknown[]; partial?: boolean }[] }));
vi.mock('../../store', async () => {
  // @ts-expect-error: the browser build has no type declarations of its own.
  const { createSignal } = await import('solid-js/dist/solid.js');
  // The streaming (last) turn's fields are reactive and `touch` re-runs them;
  // the array's shape and the first turn are not, as in the real store, where
  // the scroll signature (length + first text) does not change as a reply grows.
  const [stream, setStream] = createSignal(0, { equals: false });
  const live = (turn: object) => new Proxy(turn, { get(t, k) { stream(); return Reflect.get(t, k); } });
  const turns = new Proxy(st.turns, {
    get(t, k) {
      const i = typeof k === 'string' && /^\d+$/.test(k) ? Number(k) : -1;
      return i > 0 && i === t.length - 1 ? live(t[i]) : Reflect.get(t, k);
    },
  });
  const [busy, setBusy] = createSignal(false);
  (st as Record<string, unknown>).touch = () => setStream(0);
  (st as Record<string, unknown>).setBusy = setBusy;
  return {
    chatTurns: turns,
    chatAgentSessionKey: () => st.key,
    chatBusy: busy,
    chatSessionLoading: () => false,
    chatConversationVersion: () => 0,
    flashFooter: () => {},
    pendingApprovals: () => [],
    chatInteractions: () => [],
  };
});

// @ts-expect-error: the browser build has no type declarations of its own.
import { createRoot } from 'solid-js/dist/solid.js';
import { createChatScroll } from './chatScrollController';

class FakeEvent { constructor(public type: string, public target: unknown = null) {} }
class FakeWheel extends FakeEvent { deltaY = 0; }
class FakeKey extends FakeEvent { key = ''; }

function fakeScroller() {
  const handlers = new Map<string, (e: unknown) => void>();
  const el = {
    scrollTop: 0, scrollHeight: 1000, clientHeight: 400, offsetParent: {}, children: [],
    addEventListener: (k: string, fn: (e: unknown) => void) => { handlers.set(k, fn); },
    removeEventListener: (k: string) => { handlers.delete(k); },
    querySelector: () => null, querySelectorAll: () => [], contains: () => false,
    getBoundingClientRect: () => ({ top: 0, bottom: 400, left: 0, right: 300 }),
    scrollTo: (o: { top: number }) => { el.scrollTop = o.top; },
  };
  return { el, fire: (k: string, e: unknown) => handlers.get(k)?.(e) };
}

let runs = 0;
let frames: (() => void)[] = [];
const flushFrames = () => { for (let i = 0; i < 20 && frames.length; i++) { const run = frames; frames = []; run.forEach(f => f()); } };
const microtasks = () => new Promise<void>(r => setTimeout(r, 0));

let dispose = () => {};
function mount() {
  const s = fakeScroller();
  let scroll!: ReturnType<typeof createChatScroll>;
  createRoot((d: () => void) => {
    dispose = d;
    scroll = createChatScroll({}, { lastUser: () => 0, openFoldAt: i => i, textarea: () => undefined });
    scroll.setScrollEl(s.el as unknown as HTMLDivElement);
  });
  flushFrames();
  return { ...s, scroll };
}
/** The reply grows by `by` px: layout first, then the store change the follow reads. */
function grow(el: { scrollHeight: number }, by: number) {
  el.scrollHeight += by;
  st.turns[st.turns.length - 1].text += ' more';
  (st as unknown as { touch: () => void }).touch();
  flushFrames();
}

beforeEach(() => {
  frames = [];
  st.key = `conv-${++runs}`;
  st.turns.length = 0;
  st.turns.push({ text: 'hello', tools: [] }, { text: 'streaming', tools: [], partial: true });
  const g = globalThis as Record<string, unknown>;
  const win = new EventTarget();
  g.window = win;
  g.document = { documentElement: { dataset: { motion: 'reduced' } }, hasFocus: () => true, activeElement: null };
  g.requestAnimationFrame = (f: () => void) => frames.push(f);
  g.cancelAnimationFrame = () => {};
  g.KeyboardEvent = FakeKey;
  g.WheelEvent = FakeWheel;
  g.Element = class {};
  g.MutationObserver = class { observe() {} disconnect() {} };
});
afterEach(() => { dispose(); dispose = () => {}; });

describe('createChatScroll', () => {
  it('starts at the end and keeps following while the reply streams', () => {
    const { el, scroll } = mount();
    expect(scroll.atBottom()).toBe(true);
    expect(el.scrollTop).toBe(1000);
    grow(el, 300);
    expect(el.scrollTop).toBe(1300);
    grow(el, 120);
    expect(el.scrollTop).toBe(1420);
    expect(scroll.atBottom()).toBe(true);
  });

  it('stops following when the reader scrolls up, and the stream no longer moves the view', () => {
    const { el, fire, scroll } = mount();
    const wheel = new FakeWheel('wheel', el); wheel.deltaY = -120;
    fire('wheel', wheel);
    el.scrollTop = 200;
    scroll.onScroll();
    expect(scroll.atBottom()).toBe(false);
    grow(el, 300);
    grow(el, 300);
    expect(el.scrollTop).toBe(200);
    expect(scroll.atBottom()).toBe(false);
  });

  it('a sideways wheel is not a scroll up: the follow stays', () => {
    const { el, fire, scroll } = mount();
    const wheel = new FakeWheel('wheel', el); // deltaY 0
    fire('wheel', wheel);
    grow(el, 200);
    expect(scroll.atBottom()).toBe(true);
    expect(el.scrollTop).toBe(1200);
  });

  it('Jump to latest takes hold again, and the next growth follows', () => {
    const { el, fire, scroll } = mount();
    const wheel = new FakeWheel('wheel', el); wheel.deltaY = -120;
    fire('wheel', wheel);
    el.scrollTop = 100;
    scroll.onScroll();
    grow(el, 300);
    expect(scroll.atBottom()).toBe(false);
    scroll.jumpToLatest();
    expect(scroll.atBottom()).toBe(true);
    expect(el.scrollTop).toBe(el.scrollHeight);
    grow(el, 250);
    expect(el.scrollTop).toBe(el.scrollHeight);
  });

  it('scrolling back down to the end re-anchors without the pill', () => {
    const { el, fire, scroll } = mount();
    const wheel = new FakeWheel('wheel', el); wheel.deltaY = -120;
    fire('wheel', wheel);
    el.scrollTop = 100;
    scroll.onScroll();
    expect(scroll.atBottom()).toBe(false);
    el.scrollTop = el.scrollHeight - el.clientHeight; // the reader drags back to the end
    scroll.onScroll();
    expect(scroll.atBottom()).toBe(true);
    grow(el, 200);
    expect(el.scrollTop).toBe(el.scrollHeight);
  });

  it('a send always goes to the end, even from further up', async () => {
    const { el, fire, scroll } = mount();
    const wheel = new FakeWheel('wheel', el); wheel.deltaY = -120;
    fire('wheel', wheel);
    el.scrollTop = 50;
    scroll.onScroll();
    await microtasks();
    el.scrollHeight += 200;
    scroll.followSend();
    flushFrames();
    expect(el.scrollTop).toBe(el.scrollHeight);
    expect(scroll.atBottom()).toBe(true);
  });
});
