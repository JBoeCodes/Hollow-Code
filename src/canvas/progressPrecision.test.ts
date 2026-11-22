import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => vi.unstubAllGlobals());

class Part {
  attributes = new Map<string, string>();
  children: Part[] = [];
  className = '';
  textContent = '';
  values = new Map<string, string>();
  style = { setProperty: (key: string, value: string) => this.values.set(key, value) };
  getAttribute(key: string) { return this.attributes.get(key) ?? null; }
  hasAttribute(key: string) { return this.attributes.has(key); }
  setAttribute(key: string, value: string) { this.attributes.set(key, value); }
  append(...parts: Part[]) { this.children.push(...parts); }
  prepend(...parts: Part[]) { this.children.unshift(...parts); }
}

async function progress(value: string | null, max = '100') {
  vi.stubGlobal('HTMLElement', Part);
  vi.stubGlobal('document', { createElement: () => new Part() });
  const { HcProgress } = await import('./kit/components/progress');
  const el = new HcProgress() as unknown as Part & { render(): void };
  if (value !== null) el.setAttribute('value', value);
  el.setAttribute('max', max);
  el.render();
  const [head, track] = el.children;
  return { text: head.children.at(-1)?.textContent, track, fraction: Number(track.children[0].values.get('--hc-pct')) };
}

it('progress keeps fractional endpoints and exact labels rather than rounding geometry', async () => {
  const p = await progress('0.5');
  expect(p.fraction).toBe(0.005);
  expect(p.text).toBe('0.5%');
  expect(p.track.getAttribute('aria-valuenow')).toBe('0.5');
  expect((await progress('1', '3')).fraction).toBeCloseTo(1 / 3, 12);
});

it('progress speaks a concise percentage without rounding the fill', async () => {
  const p = await progress('1', '3');
  expect(p.track.getAttribute('aria-valuetext')).toBe('1 of 3 (33%)');
  expect(p.fraction).toBeCloseTo(1 / 3, 12);
});

it('progress clamps only geometry and exposes authored out-of-range values', async () => {
  for (const raw of ['125', '-5']) {
    const p = await progress(raw);
    expect(p.text).toContain(raw);
    expect(p.text).toContain('outside range');
    expect(p.track.getAttribute('aria-valuetext')).toContain(raw);
    expect(p.fraction).toBe(raw === '125' ? 1 : 0);
  }
});

it('missing progress never acquires an ARIA numeric value', async () => {
  const p = await progress(null);
  expect(p.track.getAttribute('aria-valuenow')).toBeNull();
  expect(p.text).toBe('—');
});
