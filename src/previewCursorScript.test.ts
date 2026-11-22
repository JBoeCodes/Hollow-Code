import { describe, expect, it } from 'vitest';
import script from '../src-tauri/src/browser_pane/agent_cursor.js?raw';

function harness() {
  class Element {
    style: Record<string, unknown> & { setProperty: (name: string, value: string) => void } = { setProperty: (name, value) => { this.style[name] = value; } };
    attributes = new Map<string, string>();
    children: Element[] = [];
    removed = false;
    textContent = '';
    setAttribute(name: string, value: string) { this.attributes.set(name, value); }
    append(...children: Element[]) { this.children.push(...children); }
    attachShadow() { return new Element(); }
    remove() { this.removed = true; }
  }
  const events = new Map<string, Set<() => void>>();
  const add = (name: string, fn: () => void) => { if (!events.has(name)) events.set(name, new Set()); events.get(name)!.add(fn); };
  const remove = (name: string, fn: () => void) => events.get(name)?.delete(fn);
  const media = () => Object.assign(new EventTarget(), { matches: false });
  const reduced = media(), forced = media();
  let next = 0;
  const timers = new Map<number, () => void>();
  const frames = new Map<number, () => void>();
  type Marker = { host: Element; dot: Element; seq: number };
  const context = {
    location: { href: 'https://example.test/' }, innerWidth: 800, innerHeight: 600, scrollX: 0, scrollY: 0,
    document: { createElement: () => new Element(), createElementNS: () => new Element(), documentElement: new Element() },
    matchMedia: (q: string) => q.includes('forced') ? forced : reduced,
    getComputedStyle: (el: Element) => el.style,
    addEventListener: add, removeEventListener: remove,
    setTimeout: (fn: () => void) => { timers.set(++next, fn); return next; }, clearTimeout: (id: number) => timers.delete(id),
    requestAnimationFrame: (fn: () => void) => { frames.set(++next, fn); return next; }, cancelAnimationFrame: (id: number) => frames.delete(id),
    __hollowInspectionMarker: undefined as Marker | undefined,
  };
  const payload = (seq = 1, duration = 200) => ({seq, duration, remaining: 3000, url: context.location.href,
    point: {x: 30, y: 40, width: 800, height: 600, scroll_x: 0, scroll_y: 0}, color: 'Highlight', background: 'Canvas', ink: 'CanvasText'});
  return { context, reduced, forced, timers, frames, payload,
    run: (p: ReturnType<typeof payload> | null) => {
      const entries = Object.entries({ ...context, globalThis: context });
      return new Function(...entries.map(([key]) => key), `return ${script.replace('__PAYLOAD__', JSON.stringify(p))}`)(...entries.map(([, value]) => value));
    },
    emit: (name: string) => { for (const fn of [...events.get(name) ?? []]) fn(); },
  };
}

describe('native Preview inspection decoration', () => {
  it('never receives input and clears on scroll, resize, navigation and expiry', () => {
    for (const reason of ['scroll', 'resize', 'pagehide', 'timer', 'same-document']) {
      const h = harness();
      expect(h.run(h.payload())).toBe('shown');
      const marker = h.context.__hollowInspectionMarker!;
      expect(marker.host.style['pointer-events']).toBe('none');
      expect(marker.host.attributes.get('aria-hidden')).toBe('true');
      if (reason === 'timer') [...h.timers.values()][0]();
      else if (reason === 'same-document') { h.context.location.href = 'https://example.test/other'; [...h.frames.values()][0](); }
      else h.emit(reason);
      expect(h.context.__hollowInspectionMarker).toBeUndefined();
      expect(marker.host.removed).toBe(true);
      expect(h.timers.size).toBe(0);
      expect(h.frames.size).toBe(0);
    }
  });

  it('rejects coordinates from another URL, viewport or scroll position', () => {
    const h = harness();
    const p = h.payload();
    expect(h.run({...p, url: 'https://elsewhere.test/'})).toBe('cleared');
    expect(h.run({...p, point: {...p.point, width: 799}})).toBe('cleared');
    expect(h.run({...p, point: {...p.point, scroll_y: 20}})).toBe('cleared');
    expect(h.context.__hollowInspectionMarker).toBeUndefined();
  });

  it('a policy refresh cannot extend the inspection lifetime', () => {
    const h = harness();
    h.run(h.payload());
    const timer = [...h.timers.keys()][0];
    expect(h.run(h.payload())).toBe('kept');
    expect([...h.timers.keys()]).toEqual([timer]);
    expect(h.run(null)).toBe('cleared');
  });

  it('zero duration and system preferences snap; a live reduction cancels motion', () => {
    for (const policy of ['zero', 'reduced', 'forced', 'full']) {
      const h = harness();
      h.run(h.payload());
      h.reduced.matches = policy === 'reduced'; h.forced.matches = policy === 'forced';
      h.run({...h.payload(2, policy === 'zero' ? 0 : 250), point: {...h.payload().point, x: 120}});
      const dot = h.context.__hollowInspectionMarker!.dot;
      if (policy === 'full') {
        expect(dot.style.transitionDuration).toBe('250ms');
        h.reduced.matches = true; h.reduced.dispatchEvent(new Event('change'));
        expect(dot.style.transitionDuration).toBe('0ms');
      } else expect(dot.style.transitionDuration).toBeUndefined();
      expect(dot.style.transform).toBe('translate(120px, 40px)');
    }
  });
});
