import { describe, expect, it, vi } from 'vitest';
import { parseCanvasFence, canvasFrameBody } from './fence';
import { requestedTexture, textureOf, watchTextures } from './kit/texture';
import CAPTURE from './CaptureHost.tsx?raw';
import { ARTIFACT_STYLES } from './artifactStyles';
import GUIDE from '../../src-tauri/src/canvas_design_prompt.md?raw';

describe('canvas texture contract', () => {
  it('keeps plain rendering available if subscribing to media fails', () => {
    vi.stubGlobal('matchMedia', () => { throw new Error('unavailable'); });
    try { expect(() => watchTextures(() => {})()).not.toThrow(); }
    finally { vi.unstubAllGlobals(); }
  });
  it('shares one subscription and releases its observer and media listener with the last graphic', () => {
    const callbacks = new Map<string, () => void>();
    let mutation = () => {};
    const disconnect = vi.fn(), removeMedia = vi.fn();
    vi.stubGlobal('document', { documentElement: {} });
    vi.stubGlobal('matchMedia', () => ({ addEventListener: (_: string, fn: () => void) => callbacks.set('media', fn), removeEventListener: removeMedia }));
    vi.stubGlobal('MutationObserver', class { constructor(fn: () => void) { mutation = fn; } observe() {} disconnect = disconnect; });
    vi.stubGlobal('addEventListener', (name: string, fn: () => void) => callbacks.set(name, fn));
    vi.stubGlobal('removeEventListener', (name: string) => callbacks.delete(name));
    const a = vi.fn(), b = vi.fn();
    try {
      const stopA = watchTextures(a), stopB = watchTextures(b);
      callbacks.get('hollow-texture')!(); mutation(); callbacks.get('media')!();
      expect(a).toHaveBeenCalledTimes(3); expect(b).toHaveBeenCalledTimes(3);
      stopA(); mutation();
      expect(a).toHaveBeenCalledTimes(3); expect(b).toHaveBeenCalledTimes(4);
      expect(disconnect).not.toHaveBeenCalled();
      stopB();
      expect(disconnect).toHaveBeenCalledTimes(1); expect(removeMedia).toHaveBeenCalledTimes(1);
      expect(callbacks.has('hollow-texture')).toBe(false);
    } finally { vi.unstubAllGlobals(); }
  });

  it('export explicitly vetoes decoration while preserving source requests', () => {
    expect(CAPTURE).toContain("kind: 'motion', reducedMotion: true, textureAllowed: false");
    expect(CAPTURE).toContain('canvasFrameBody(parsed)');
  });

  it('resets the actual content edges for textured and ordinary bodies alike', () => {
    expect(ARTIFACT_STYLES).toContain(':where(body>:first-child,body>[data-hc-texture-root]>:first-child){margin-top:0}');
    expect(ARTIFACT_STYLES).toContain(':where(body>:last-child,body>[data-hc-texture-root]>:last-child){margin-bottom:0}');
  });

  it('keeps the agent design guide within its existing Rust budget', () => {
    expect(new TextEncoder().encode(GUIDE.trimEnd()).length).toBeLessThanOrEqual(2400);
  });

  it('preserves a recognized root request in a frame without changing saved source', () => {
    for (const value of ['dither', 'ascii']) {
      const p = parseCanvasFence(`<hc-canvas id="a" title="A" texture="${value}"><hc-chart>Mon: 2</hc-chart></hc-canvas>`, true);
      expect(p.state).toBe('ready');
      if (p.state === 'ready') expect(canvasFrameBody(p)).toBe(`<div data-hc-texture-root texture="${value}" style="display:contents"><hc-chart>Mon: 2</hc-chart></div>`);
    }
    for (const attr of ['', ' texture="plain"', ' texture="unknown"', ' texture="&quot; onclick=bad"']) {
      const p = parseCanvasFence(`<hc-canvas id="a" title="A"${attr}>original</hc-canvas>`, true);
      expect(p.state).toBe('ready');
      if (p.state === 'ready') expect(canvasFrameBody(p)).toBe('<div data-hc-texture-root texture="dither" style="display:contents">original</div>');
    }
  });

  it('normalizes only named variants and gives an explicit unknown override the plain fallback', () => {
    expect(requestedTexture(' DITHER ')).toBe('dither');
    expect(requestedTexture('ascii')).toBe('ascii');
    for (const v of [null, '']) expect(requestedTexture(v)).toBe('dither');
    for (const v of ['glow', 'plain']) expect(requestedTexture(v)).toBe('plain');
  });

  it('uses the nearest request only after host permission, and media restrictions always veto', () => {
    const element = (value: string | null, root = false) => ({ closest: () => value === null ? null : ({ getAttribute: () => value, matches: () => root }) }) as unknown as Element;
    let allowed = false, blocked = false;
    vi.stubGlobal('window', { hollowCanvas: { get textureAllowed() { return allowed; } } });
    vi.stubGlobal('matchMedia', () => ({ matches: blocked }));
    try {
      expect(textureOf(element('dither'))).toBe('plain');
      allowed = true;
      expect(textureOf(element('dither'))).toBe('dither');
      expect(textureOf(element('ascii'))).toBe('ascii');
      expect(textureOf(element('unknown'))).toBe('plain');
      expect(textureOf(element(null))).toBe('dither');
      expect(textureOf(element('plain', true))).toBe('dither');
      expect(textureOf(element('plain'))).toBe('plain');
      blocked = true;
      expect(textureOf(element('dither'))).toBe('plain');
    } finally { vi.unstubAllGlobals(); }
  });
});
