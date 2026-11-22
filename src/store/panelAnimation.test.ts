import { afterEach, describe, expect, it, vi } from 'vitest';
// @ts-expect-error Solid's browser runtime is intentionally used for effects.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// Contract for 0.2: the setting is 0..400 ms (default 0); panels animate only when it
// is positive, motion is not reduced (app setting or system) and navigation is not
// suppressing it; a closing panel stays mounted for its exit; reopening cancels it.
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

async function load(opts: { saved?: string; reducedAttr?: boolean; systemReduced?: boolean } = {}) {
  vi.resetModules();
  const saved = new Map<string, string>(opts.saved === undefined ? [] : [['hollow.studio.panelAnimationMs', opts.saved]]);
  vi.stubGlobal('localStorage', { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => saved.set(k, v) });
  const props: Record<string, string> = {}; const attrs = new Set<string>();
  vi.stubGlobal('document', { documentElement: {
    dataset: opts.reducedAttr ? { motion: 'reduced' } : { motion: 'full' },
    style: { setProperty: (k: string, v: string) => { props[k] = v; } },
    setAttribute: (k: string) => attrs.add(k), removeAttribute: (k: string) => attrs.delete(k),
  }, defaultView: undefined });
  vi.stubGlobal('window', { matchMedia: () => ({ matches: !!opts.systemReduced }) });
  const mod = await import('./panelAnimation');
  // Same module instance as the store's, so signals made here are the ones its effects track.
  const { createRoot, createSignal } = await import('solid-js');
  return { mod, saved, props, attrs, createRoot, createSignal };
}

describe('panel animation setting', () => {
  it('defaults to 0, clamps to 0..400 and persists', async () => {
    const { mod, saved, props } = await load();
    expect(mod.panelAnimationMs()).toBe(0);
    expect(mod.normalizePanelDuration(-5)).toBe(0);
    expect(mod.normalizePanelDuration('9999')).toBe(400);
    expect(mod.normalizePanelDuration('abc')).toBe(0);
    expect(mod.normalizePanelDuration(149.6)).toBe(150);
    mod.setPanelAnimationMs(250);
    expect(saved.get('hollow.studio.panelAnimationMs')).toBe('250');
    expect(props['--panel-duration-user']).toBe('250ms');
    const again = await load({ saved: '250' });
    expect(again.mod.panelAnimationMs()).toBe(250);
    expect(again.props['--panel-duration-user']).toBe('250ms');
  });

  it('is 0 under the app reduced setting and under the system preference, whatever is saved', async () => {
    for (const o of [{ reducedAttr: true }, { systemReduced: true }]) {
      const { mod } = await load({ saved: '300', ...o });
      expect(mod.panelAnimationMs()).toBe(300);
      expect(mod.panelDurationMs()).toBe(0);
    }
    const full = await load({ saved: '300' });
    expect(full.mod.panelDurationMs()).toBe(300);
  });

  it('is off while navigating and returns two frames after the paint', async () => {
    const { mod, attrs } = await load({ saved: '200' });
    const queue: (() => void)[] = [];
    mod.suppressPanelAnimation(cb => { queue.push(cb); });
    expect(mod.panelDurationMs()).toBe(0);
    expect(attrs.has('data-panel-still')).toBe(true);
    queue.shift()!();            // frame 1 runs, queues frame 2
    expect(mod.panelDurationMs()).toBe(0);
    queue.shift()!();            // frame 2
    expect(mod.panelDurationMs()).toBe(200);
    expect(attrs.has('data-panel-still')).toBe(false);
  });

  it('a second navigation keeps it suppressed until its own frames finish', async () => {
    const { mod } = await load({ saved: '200' });
    const a: (() => void)[] = []; const b: (() => void)[] = [];
    mod.suppressPanelAnimation(cb => { a.push(cb); });
    mod.suppressPanelAnimation(cb => { b.push(cb); });
    a.shift()!(); a.shift()!();  // the first navigation's frames finish
    expect(mod.panelDurationMs()).toBe(0);
    b.shift()!(); b.shift()!();
    expect(mod.panelDurationMs()).toBe(200);
  });

  it('with no animation frames available it never leaves a navigation stuck', async () => {
    const { mod, attrs } = await load({ saved: '200' });
    mod.suppressPanelAnimation(undefined);
    expect(mod.panelDurationMs()).toBe(200);
    expect(attrs.has('data-panel-still')).toBe(false);
  });
});

describe('panel presence', () => {
  it('keeps a closing panel mounted for the exit duration, and a reopen cancels the exit', async () => {
    vi.useFakeTimers(); // sleep-ok: the exit timer is the behaviour under test
    const { mod, createRoot, createSignal } = await load();
    await createRoot(async dispose => {
      const [open, setOpen] = createSignal(true);
      const p = mod.createPanelPresence(open, () => 200);
      await Promise.resolve(); // let the effect's first run record `open`
      expect(p.mounted()).toBe(true);
      setOpen(false);
      await Promise.resolve();
      expect(p.mounted()).toBe(true); expect(p.closing()).toBe(true);
      vi.advanceTimersByTime(199);
      expect(p.mounted()).toBe(true);
      setOpen(true); await Promise.resolve();
      vi.advanceTimersByTime(500);
      expect(p.mounted()).toBe(true); expect(p.closing()).toBe(false);
      setOpen(false); await Promise.resolve();
      vi.advanceTimersByTime(200);
      expect(p.mounted()).toBe(false); expect(p.closing()).toBe(false);
      dispose();
    });
  });

  it('unmounts at once when the duration is 0 (off, reduced or navigating)', async () => {
    const { mod, createRoot, createSignal } = await load();
    await createRoot(async dispose => {
      const [open, setOpen] = createSignal(true);
      const p = mod.createPanelPresence(open, () => 0);
      await Promise.resolve();
      setOpen(false); await Promise.resolve();
      expect(p.mounted()).toBe(false); expect(p.closing()).toBe(false);
      dispose();
    });
  });
});
