import { afterEach, expect, it, vi } from 'vitest';
import { windowAtMost } from './windowLayout';

afterEach(() => vi.unstubAllGlobals());

it.each([100, 112, 125])('uses the CSS root width and live font at scale %s', scale => {
  const root = { clientWidth: 860 * scale / 100 };
  vi.stubGlobal('window', { innerWidth: 1600 });
  vi.stubGlobal('document', { documentElement: root });
  vi.stubGlobal('getComputedStyle', () => ({ fontSize: `${16 * scale / 100}px`, getPropertyValue: (key: string) => key.includes('compact') ? '53.75' : '68.75' }));
  expect(windowAtMost('compact')).toBe(true);
  root.clientWidth += 1;
  expect(windowAtMost('compact')).toBe(false);
  root.clientWidth = 1100 * scale / 100;
  expect(windowAtMost('medium')).toBe(true);
  root.clientWidth += 1;
  expect(windowAtMost('medium')).toBe(false);
});

it('has a default-width fallback for non-DOM callers', () => {
  vi.stubGlobal('document', undefined);
  vi.stubGlobal('window', { innerWidth: 720 });
  expect(windowAtMost('compact')).toBe(true);
  vi.stubGlobal('window', undefined);
  expect(windowAtMost('medium')).toBe(false);
});
