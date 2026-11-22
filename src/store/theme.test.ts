import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { CustomTheme } from '../types';

const reads = vi.hoisted(() => ({ theme: vi.fn(), vibrancy: vi.fn(async () => {}) }));
vi.mock('../api', () => ({ api: { readCustomTheme: reads.theme, setWindowVibrancy: reads.vibrancy } }));
vi.mock('solid-js', async importOriginal => ({ ...await importOriginal<typeof import('solid-js')>(), createResource: () => [() => [], {}] }));

let saved: Map<string, string>;
let dark: boolean;
let changed: (() => void) | undefined;
const fixture = { name: 'Old', isDark: true, background: '#000000', foreground: '#ffffff', accent: '#bbbbbb', colors: [] } as unknown as CustomTheme;
beforeEach(() => {
  vi.resetModules();
  saved = new Map([['theme', 'light'], ['activeCustomTheme', 'Old']]);
  dark = false;
  changed = undefined;
  vi.stubGlobal('localStorage', { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => saved.set(k, v), removeItem: (k: string) => saved.delete(k) });
  vi.stubGlobal('document', { documentElement: { dataset: { motion: 'reduced' }, style: { setProperty: vi.fn(), removeProperty: vi.fn() }, setAttribute: vi.fn(), removeAttribute: vi.fn() } });
  vi.stubGlobal('window', { matchMedia: () => ({ get matches() { return dark; }, addEventListener: (_: string, cb: () => void) => { changed = cb; } }) });
});
afterEach(() => vi.unstubAllGlobals());

it('an explicit stock choice owns the theme over a pending custom boot read', async () => {
  let resolve!: (t: CustomTheme) => void;
  reads.theme.mockReturnValue(new Promise<CustomTheme>(r => { resolve = r; }));
  const theme = await import('./theme');
  theme.setActiveTheme('light');
  resolve(fixture);
  await new Promise(r => setTimeout(r, 0));
  expect(theme.customTheme()).toBeNull();
  expect(theme.theme()).toBe('light');
  expect(saved.has('activeCustomTheme')).toBe(false);
});

it('a failed old boot read cannot clear a newer custom selection', async () => {
  let reject!: (e: Error) => void;
  reads.theme.mockReturnValue(new Promise<CustomTheme>((_, r) => { reject = r; }));
  const theme = await import('./theme');
  await theme.setActiveTheme({ custom: { ...fixture, name: 'New' } });
  reject(new Error('old read failed'));
  await new Promise(r => setTimeout(r, 0));
  expect(saved.get('activeCustomTheme')).toBe('New');
});

it('saved system appearance follows changes without persisting the effective polarity', async () => {
  saved = new Map([['theme', 'system']]);
  const theme = await import('./theme');
  dark = true;
  changed?.();
  expect(theme.theme()).toBe('dark');
  expect(saved.get('theme')).toBe('system');
});
