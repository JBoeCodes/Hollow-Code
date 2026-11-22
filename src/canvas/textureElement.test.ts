import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => vi.unstubAllGlobals());

it('live texture veto and inherited overrides change only presentation and disconnect cleanly', async () => {
  const events = new Map<string, () => void>();
  let changed = () => {}, allowed = true, request = 'dither';
  const disconnect = vi.fn(), render = vi.fn();
  vi.stubGlobal('HTMLElement', class {
    isConnected = false;
    attrs = new Map<string, string>();
    setAttribute(k: string, v: string) { this.attrs.set(k, v); }
    closest() { return { getAttribute: () => request, matches: () => false }; }
  });
  vi.stubGlobal('window', { hollowCanvas: { get textureAllowed() { return allowed; } } });
  vi.stubGlobal('document', { readyState: 'complete', documentElement: {} });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal('MutationObserver', class { constructor(fn: () => void) { changed = fn; } observe() {} disconnect = disconnect; });
  vi.stubGlobal('addEventListener', (name: string, fn: () => void) => events.set(name, fn));
  vi.stubGlobal('removeEventListener', (name: string) => events.delete(name));
  const { TextureElement } = await import('./kit/textureElement');
  class Graphic extends TextureElement { protected render = render; }
  const graphic = new Graphic();
  const attrs = (graphic as unknown as { attrs: Map<string, string> }).attrs;
  graphic.connectedCallback();
  expect(attrs.get('data-hc-texture')).toBe('dither');
  allowed = false; events.get('hollow-texture')!();
  expect(attrs.get('data-hc-texture')).toBe('plain');
  allowed = true; request = 'ascii'; changed();
  expect(attrs.get('data-hc-texture')).toBe('ascii');
  expect(render).not.toHaveBeenCalled();
  graphic.disconnectedCallback();
  expect(disconnect).toHaveBeenCalledTimes(1);
  expect(events.size).toBe(0);
});
