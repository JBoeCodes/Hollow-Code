import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CanvasPin } from '../canvas/liveModel';

const key = 'hollow.canvasPins.v1';
const pin: CanvasPin = { vault: '/vault', workspaceId: 'workspace', sessionKey: null, id: 'report', revision: 1,
  sha256: 'a'.repeat(64), title: 'Report', destination: 'home' };
const replacement = { ...pin, revision: 2, sha256: 'b'.repeat(64) };
describe('explicit pin replacement ownership', () => {
  let saved: Map<string, string>;
  beforeEach(() => {
    vi.resetModules(); saved = new Map([[key, JSON.stringify([pin])]]);
    vi.stubGlobal('localStorage', { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => saved.set(k, v) });
  });
  afterEach(() => vi.unstubAllGlobals());

  it('never resurrects a pin removed while the new revision was read', async () => {
    const { saveCanvasPin } = await import('./canvasPins');
    saved.set(key, '[]');
    expect(() => saveCanvasPin(replacement, pin)).toThrow(/changed or was removed/);
    expect(JSON.parse(saved.get(key)!)).toEqual([]);
  });
  it('never overwrites a replacement from another view', async () => {
    const { saveCanvasPin } = await import('./canvasPins');
    const other = { ...pin, revision: 3, sha256: 'c'.repeat(64) };
    saved.set(key, JSON.stringify([other]));
    expect(() => saveCanvasPin(replacement, pin)).toThrow(/changed or was removed/);
    expect(JSON.parse(saved.get(key)!)).toEqual([other]);
  });
  it('updates revision and hash together while preserving other destinations', async () => {
    const { saveCanvasPin } = await import('./canvasPins');
    const workspace = { ...pin, destination: 'workspace' };
    saved.set(key, JSON.stringify([pin, workspace]));
    saveCanvasPin(replacement, pin);
    expect(JSON.parse(saved.get(key)!)).toEqual([workspace, replacement]);
  });
});
