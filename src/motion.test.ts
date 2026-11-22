import { afterEach, expect, it, vi } from 'vitest';
import { motionQuery, motionReduced } from './motion';

afterEach(() => vi.unstubAllGlobals());

it('reads live app and system preferences while reusing the window query', () => {
  const media = { matches: false };
  const view = { matchMedia: vi.fn(() => media) };
  const doc = { documentElement: { dataset: { motion: 'full' } }, defaultView: view };
  vi.stubGlobal('document', doc); vi.stubGlobal('window', view);
  expect(motionReduced()).toBe(false);
  media.matches = true; expect(motionReduced()).toBe(true);
  media.matches = false; doc.documentElement.dataset.motion = 'reduced';
  expect(motionReduced()).toBe(true);
  doc.documentElement.dataset.motion = 'full'; expect(motionReduced()).toBe(false);
  expect(view.matchMedia).toHaveBeenCalledOnce();
  expect(motionQuery()).toBe(media);
});

it('uses the supplied document window rather than another window preference', () => {
  vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
  const doc = { documentElement: { dataset: { motion: 'full' } }, defaultView: { matchMedia: () => ({ matches: false }) } };
  expect(motionReduced(doc as unknown as Document)).toBe(false);
});

it('is safe without a DOM or media-query API', () => {
  expect(motionReduced()).toBe(false);
  vi.stubGlobal('window', {});
  expect(motionReduced()).toBe(false);
});
